// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Word (docx) e PowerPoint (pptx) 2007 e successivi; riconoscimento degli OOXML cifrati.
//!
//! - **docx**: `word/document.xml`; un'unità per paragrafo non vuoto (`w:p`), numerati da 1
//!   ([`Location::Paragraph`]). Testo solo da `w:t` dentro i run; `w:tab` è una tabulazione,
//!   `w:br`/`w:cr` un a capo. Testo cancellato con revisioni (`w:delText`), codici dei campi
//!   e rami `mc:Fallback` (copie di ripiego dello stesso contenuto) sono esclusi. Intestazioni,
//!   piè di pagina, note e commenti non si leggono.
//! - **pptx**: le diapositive nell'ordine di `ppt/presentation.xml` (relazioni di
//!   `ppt/_rels/presentation.xml.rels`), o in ordine di numero se l'ordine non si ricava;
//!   un'unità per diapositiva ([`Location::Slide`]) con un paragrafo (`a:p`) per riga. Le note
//!   del relatore non si leggono.
//! - Un OOXML cifrato con password è un file composto (CFB) con un flusso `EncryptedPackage`:
//!   avviso "protetto da password". Un file composto senza quel flusso non è un OOXML.
//!
//! Le parti principali si cercano nei percorsi standard (`word/document.xml`,
//! `ppt/presentation.xml`): un pacchetto senza di esse dà "contenuto diverso
//! dall'estensione".

use std::collections::HashMap;
use std::io::{BufRead, BufReader};

use lynkeus_core::Location;

use crate::error::ExtractError;
use crate::extract::archive::Container;
use crate::extract::xml::{Ns, XmlEvent, XmlReader};
use crate::extract::{Ctx, Fail, Input};

const DOCX_MAIN: &str = "word/document.xml";
const PPTX_MAIN: &str = "ppt/presentation.xml";
const PPTX_RELS: &str = "ppt/_rels/presentation.xml.rels";
const PPTX_SLIDE_PREFIX: &str = "ppt/slides/slide";

/// Nome del flusso `EncryptedPackage` in UTF-16LE, come nella directory di un file composto.
const ENCRYPTED_PACKAGE_UTF16: &[u8] = b"E\0n\0c\0r\0y\0p\0t\0e\0d\0P\0a\0c\0k\0a\0g\0e\0";

/// Un file composto con estensione OOXML: [`ExtractError::Encrypted`] se contiene il
/// pacchetto cifrato, altrimenti [`ExtractError::ContentMismatch`] (o l'errore di lettura).
pub(crate) fn classify_compound(ctx: &Ctx<'_>, input: Input<'_>) -> ExtractError {
    match input.into_bytes(ctx) {
        Ok(bytes) if memchr::memmem::find(&bytes, ENCRYPTED_PACKAGE_UTF16).is_some() => {
            ExtractError::Encrypted
        }
        Ok(_) => ExtractError::ContentMismatch,
        Err(error) => error,
    }
}

/// Estrae i paragrafi di un docx.
pub(crate) fn extract_docx(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let mut container = Container::open(ctx, input)?;
    let Some(part) = container.part(ctx, DOCX_MAIN)? else {
        return Err(ExtractError::ContentMismatch.into());
    };
    let mut xml = XmlReader::new(BufReader::new(part));
    // Paragrafi aperti: quelli delle caselle di testo stanno dentro un altro paragrafo.
    let mut open: Vec<String> = Vec::new();
    let mut run_depth = 0_usize;
    let mut in_text = false;
    let mut number = 0_u32;
    loop {
        match xml.next()? {
            XmlEvent::Open(e) if e.is(Ns::MarkupCompatibility, "Fallback") => {
                xml.skip_subtree()?;
            }
            XmlEvent::Open(e) if e.is(Ns::Word, "p") => open.push(String::new()),
            XmlEvent::Open(e) if e.is(Ns::Word, "r") => run_depth += 1,
            XmlEvent::Open(e) if e.is(Ns::Word, "t") => in_text = run_depth > 0,
            XmlEvent::Close(Ns::Word, local) => match local.as_str() {
                "t" => in_text = false,
                "r" => run_depth = run_depth.saturating_sub(1),
                "p" => {
                    if let Some(text) = open.pop()
                        && !text.trim().is_empty()
                    {
                        number = number.saturating_add(1);
                        ctx.unit(Location::Paragraph(number), &text)?;
                    }
                }
                _ => {}
            },
            XmlEvent::Empty(e) if run_depth > 0 && e.ns == Ns::Word => {
                if let Some(top) = open.last_mut() {
                    match e.local.as_str() {
                        "tab" => top.push('\t'),
                        "br" | "cr" => top.push('\n'),
                        _ => {}
                    }
                }
            }
            XmlEvent::Text(text) if in_text => {
                if let Some(top) = open.last_mut() {
                    top.push_str(&text);
                }
            }
            XmlEvent::Eof => return Ok(()),
            _ => {}
        }
    }
}

/// Estrae le diapositive di un pptx.
pub(crate) fn extract_pptx(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let mut container = Container::open(ctx, input)?;
    if !container.has(PPTX_MAIN) {
        return Err(ExtractError::ContentMismatch.into());
    }
    let slides = match slide_order(ctx, &mut container)? {
        Some(order) => order,
        None => slides_by_number(&container.names()),
    };
    for (index, name) in slides.iter().enumerate() {
        let Some(part) = container.part(ctx, name)? else {
            continue;
        };
        let text = drawing_paragraphs(XmlReader::new(BufReader::new(part)))?;
        let number = u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX);
        ctx.unit(Location::Slide(number), &text)?;
    }
    Ok(())
}

/// I paragrafi DrawingML (`a:p`) di una parte, uno per riga.
fn drawing_paragraphs<R: BufRead>(mut xml: XmlReader<R>) -> Result<String, ExtractError> {
    let mut lines: Vec<String> = Vec::new();
    let mut paragraph: Option<String> = None;
    let mut in_text = false;
    loop {
        match xml.next()? {
            XmlEvent::Open(e) if e.is(Ns::MarkupCompatibility, "Fallback") => {
                xml.skip_subtree()?;
            }
            XmlEvent::Open(e) if e.is(Ns::Drawing, "p") => paragraph = Some(String::new()),
            XmlEvent::Open(e) if e.is(Ns::Drawing, "t") => in_text = true,
            XmlEvent::Empty(e) if e.is(Ns::Drawing, "br") => {
                if let Some(p) = paragraph.as_mut() {
                    p.push('\n');
                }
            }
            XmlEvent::Close(Ns::Drawing, local) => match local.as_str() {
                "t" => in_text = false,
                "p" => {
                    if let Some(p) = paragraph.take()
                        && !p.trim().is_empty()
                    {
                        lines.push(p);
                    }
                }
                _ => {}
            },
            XmlEvent::Text(text) if in_text => {
                if let Some(p) = paragraph.as_mut() {
                    p.push_str(&text);
                }
            }
            XmlEvent::Eof => return Ok(lines.join("\n")),
            _ => {}
        }
    }
}

/// Le parti delle diapositive nell'ordine della presentazione, o `None` se non si ricava.
fn slide_order(
    ctx: &Ctx<'_>,
    container: &mut Container,
) -> Result<Option<Vec<String>>, ExtractError> {
    let Some(rels) = container.part(ctx, PPTX_RELS)? else {
        return Ok(None);
    };
    let targets = relationship_targets(XmlReader::new(BufReader::new(rels)))?;
    let Some(main) = container.part(ctx, PPTX_MAIN)? else {
        return Ok(None);
    };
    let mut xml = XmlReader::new(BufReader::new(main));
    let mut order = Vec::new();
    loop {
        match xml.next()? {
            XmlEvent::Open(e) | XmlEvent::Empty(e) if e.is(Ns::Presentation, "sldId") => {
                let target = e
                    .attr(Ns::Relationships, "id")
                    .and_then(|id| targets.get(id))
                    .and_then(|target| resolve_target("ppt", target));
                match target {
                    Some(target) => order.push(target),
                    None => return Ok(None),
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok((!order.is_empty()).then_some(order))
}

/// Id → destinazione delle relazioni interne di un file `.rels`.
fn relationship_targets<R: BufRead>(
    mut xml: XmlReader<R>,
) -> Result<HashMap<String, String>, ExtractError> {
    let mut targets = HashMap::new();
    loop {
        match xml.next()? {
            XmlEvent::Open(e) | XmlEvent::Empty(e)
                if e.is(Ns::PackageRelationships, "Relationship") =>
            {
                let external = e
                    .attr(Ns::Other, "TargetMode")
                    .is_some_and(|mode| mode.eq_ignore_ascii_case("External"));
                if let (false, Some(id), Some(target)) = (
                    external,
                    e.attr(Ns::Other, "Id"),
                    e.attr(Ns::Other, "Target"),
                ) {
                    targets.insert(id.to_owned(), target.to_owned());
                }
            }
            XmlEvent::Eof => return Ok(targets),
            _ => {}
        }
    }
}

/// Il nome della parte a cui punta `target`, relativo alla cartella `base` del pacchetto
/// (assoluto se inizia con `/`). `None` se esce dal pacchetto.
fn resolve_target(base: &str, target: &str) -> Option<String> {
    let (start, rest) = match target.strip_prefix('/') {
        Some(absolute) => ("", absolute),
        None => (base, target),
    };
    let mut parts: Vec<&str> = start.split('/').filter(|p| !p.is_empty()).collect();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            name => parts.push(name),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Le parti `ppt/slides/slideN.xml` in ordine di N.
fn slides_by_number(names: &[String]) -> Vec<String> {
    let mut numbered: Vec<(u32, &String)> = names
        .iter()
        .filter_map(|name| {
            let number = name
                .strip_prefix(PPTX_SLIDE_PREFIX)?
                .strip_suffix(".xml")?
                .parse::<u32>()
                .ok()?;
            Some((number, name))
        })
        .collect();
    numbered.sort();
    numbered.into_iter().map(|(_, name)| name.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_target_partitions() {
        assert_eq!(
            resolve_target("ppt", "slides/slide1.xml").as_deref(),
            Some("ppt/slides/slide1.xml")
        );
        assert_eq!(
            resolve_target("ppt", "/ppt/slides/slide2.xml").as_deref(),
            Some("ppt/slides/slide2.xml")
        );
        assert_eq!(
            resolve_target("ppt/slides", "../media/a.png").as_deref(),
            Some("ppt/media/a.png")
        );
        assert_eq!(resolve_target("ppt", "../../fuori.xml"), None);
    }

    #[test]
    fn slides_sorted_by_number() {
        let names = [
            "ppt/slides/slide10.xml",
            "ppt/slides/slide2.xml",
            "ppt/slides/_rels/slide2.xml.rels",
            "ppt/slides/slide1.xml",
        ]
        .map(str::to_owned);
        assert_eq!(
            slides_by_number(&names),
            [
                "ppt/slides/slide1.xml",
                "ppt/slides/slide2.xml",
                "ppt/slides/slide10.xml"
            ]
        );
    }

    #[test]
    fn drawing_paragraphs_skip_fallback() {
        let xml = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"
            xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
            xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">
            <a:p><a:r><a:t>Stasera</a:t></a:r><a:r><a:t> aragosta</a:t></a:r></a:p>
            <a:p></a:p>
            <mc:AlternateContent><mc:Choice><a:p><a:r><a:t>uno</a:t></a:r></a:p></mc:Choice>
            <mc:Fallback><a:p><a:r><a:t>uno</a:t></a:r></a:p></mc:Fallback></mc:AlternateContent>
            </p:sld>"#;
        let text = drawing_paragraphs(XmlReader::new(xml.as_bytes()));
        assert_eq!(text.ok().as_deref(), Some("Stasera aragosta\nuno"));
    }
}
