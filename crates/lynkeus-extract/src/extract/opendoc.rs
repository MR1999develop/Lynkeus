// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! OpenDocument: testo (odt) e presentazioni (odp); il controllo del pacchetto vale anche
//! per i fogli (ods, letti in [`sheet`](super::sheet)).
//!
//! - Pacchetto: `mimetype`, se c'è, deve essere quello del formato (altrimenti "contenuto
//!   diverso dall'estensione"); un `manifest:encryption-data` nel manifesto vuol dire
//!   documento cifrato ("protetto da password"); `content.xml` deve esserci.
//! - **odt**: un'unità per paragrafo o titolo non vuoto (`text:p`, `text:h`) di
//!   `content.xml`, numerati da 1 ([`Location::Paragraph`]). Gli spazi del sorgente XML si
//!   riducono a uno; `text:s` sono spazi voluti, `text:tab`
//!   una tabulazione, `text:line-break` un a capo. Le note a piè di pagina sono paragrafi a
//!   sé; revisioni cancellate (`text:tracked-changes`), commenti e numeri delle note esclusi.
//! - **odp**: un'unità per pagina (`draw:page`, [`Location::Slide`]), un paragrafo per riga;
//!   le note del relatore (`presentation:notes`) escluse.

use std::io::{BufRead, BufReader, Read};

use lynkeus_core::Location;

use crate::error::ExtractError;
use crate::extract::archive::Container;
use crate::extract::budget::classify_io;
use crate::extract::xml::{Element, Ns, XmlEvent, XmlReader};
use crate::extract::{Ctx, Fail, Input};

/// Tipo MIME dei testi (anche i modelli, `-template`).
pub(crate) const ODT_MIME: &str = "application/vnd.oasis.opendocument.text";
/// Tipo MIME dei fogli di calcolo.
pub(crate) const ODS_MIME: &str = "application/vnd.oasis.opendocument.spreadsheet";
/// Tipo MIME delle presentazioni.
pub(crate) const ODP_MIME: &str = "application/vnd.oasis.opendocument.presentation";

const CONTENT: &str = "content.xml";
const MANIFEST: &str = "META-INF/manifest.xml";
const MIMETYPE: &str = "mimetype";
/// Byte letti della voce `mimetype`: i tipi ODF sono più corti.
const MIMETYPE_MAX_BYTES: u64 = 256;
/// Spazi massimi di un solo `text:s`.
const MAX_SPACES: usize = 1024;

/// Controlla il pacchetto ODF: tipo MIME, cifratura, presenza di `content.xml`.
pub(crate) fn check_package(
    ctx: &Ctx<'_>,
    container: &mut Container,
    mime: &str,
) -> Result<(), ExtractError> {
    if let Some(part) = container.part(ctx, MIMETYPE)? {
        let mut declared = Vec::new();
        part.take(MIMETYPE_MAX_BYTES)
            .read_to_end(&mut declared)
            .map_err(classify_io)?;
        if !declared.starts_with(mime.as_bytes()) {
            return Err(ExtractError::ContentMismatch);
        }
    }
    if let Some(part) = container.part(ctx, MANIFEST)? {
        let mut xml = XmlReader::new(BufReader::new(part));
        loop {
            match xml.next()? {
                XmlEvent::Open(e) | XmlEvent::Empty(e)
                    if e.is(Ns::OdfManifest, "encryption-data") =>
                {
                    return Err(ExtractError::Encrypted);
                }
                XmlEvent::Eof => break,
                _ => {}
            }
        }
    }
    if !container.has(CONTENT) {
        return Err(ExtractError::ContentMismatch);
    }
    Ok(())
}

/// Estrae i paragrafi di un odt.
pub(crate) fn extract_odt(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let mut container = Container::open(ctx, input)?;
    check_package(ctx, &mut container, ODT_MIME)?;
    let Some(part) = container.part(ctx, CONTENT)? else {
        return Err(ExtractError::ContentMismatch.into());
    };
    let mut xml = XmlReader::new(BufReader::new(part));
    let mut paragraphs = Paragraphs::default();
    let mut number = 0_u32;
    loop {
        match xml.next()? {
            XmlEvent::Eof => return Ok(()),
            event => {
                if let Some(text) = paragraphs.feed(&mut xml, event)?
                    && !text.trim().is_empty()
                {
                    number = number.saturating_add(1);
                    ctx.unit(Location::Paragraph(number), &text)?;
                }
            }
        }
    }
}

/// Estrae le pagine di un odp.
pub(crate) fn extract_odp(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let mut container = Container::open(ctx, input)?;
    check_package(ctx, &mut container, ODP_MIME)?;
    let Some(part) = container.part(ctx, CONTENT)? else {
        return Err(ExtractError::ContentMismatch.into());
    };
    let mut xml = XmlReader::new(BufReader::new(part));
    let mut paragraphs = Paragraphs::default();
    let mut slide = 0_u32;
    let mut lines: Option<Vec<String>> = None;
    loop {
        match xml.next()? {
            XmlEvent::Open(e) if e.is(Ns::OdfDraw, "page") => {
                slide = slide.saturating_add(1);
                lines = Some(Vec::new());
            }
            XmlEvent::Empty(e) if e.is(Ns::OdfDraw, "page") => {
                slide = slide.saturating_add(1);
            }
            XmlEvent::Open(e) if e.is(Ns::OdfPresentation, "notes") => xml.skip_subtree()?,
            XmlEvent::Close(Ns::OdfDraw, local) if local == "page" => {
                if let Some(lines) = lines.take() {
                    ctx.unit(Location::Slide(slide), &lines.join("\n"))?;
                }
            }
            XmlEvent::Eof => return Ok(()),
            event => {
                if let Some(text) = paragraphs.feed(&mut xml, event)?
                    && !text.trim().is_empty()
                    && let Some(lines) = lines.as_mut()
                {
                    lines.push(text);
                }
            }
        }
    }
}

/// Raccoglie il testo dei paragrafi ODF (`text:p`, `text:h`), anche annidati (note).
#[derive(Debug, Default)]
struct Paragraphs {
    open: Vec<String>,
}

impl Paragraphs {
    /// Elabora un evento; restituisce il testo di un paragrafo appena chiuso.
    fn feed<R: BufRead>(
        &mut self,
        xml: &mut XmlReader<R>,
        event: XmlEvent,
    ) -> Result<Option<String>, ExtractError> {
        match event {
            XmlEvent::Open(e) if is_paragraph(&e) => self.open.push(String::new()),
            XmlEvent::Open(e) if is_excluded(&e) => xml.skip_subtree()?,
            XmlEvent::Close(Ns::OdfText, local) if local == "p" || local == "h" => {
                return Ok(self.open.pop());
            }
            XmlEvent::Empty(e) if e.ns == Ns::OdfText => {
                if let Some(top) = self.open.last_mut() {
                    match e.local.as_str() {
                        "s" => {
                            let count = e
                                .attr(Ns::OdfText, "c")
                                .and_then(|c| c.parse::<usize>().ok())
                                .unwrap_or(1)
                                .clamp(1, MAX_SPACES);
                            top.extend(std::iter::repeat_n(' ', count));
                        }
                        "tab" => top.push('\t'),
                        "line-break" => top.push('\n'),
                        _ => {}
                    }
                }
            }
            XmlEvent::Text(text) => {
                if let Some(top) = self.open.last_mut() {
                    push_collapsed(top, &text);
                }
            }
            _ => {}
        }
        Ok(None)
    }
}

/// Aggiunge `text` a `paragraph` con gli spazi XML (spazio, tabulazione, a capo) ridotti a
/// uno solo, come vuole ODF: gli spazi voluti sono `text:s`, `text:tab`, `text:line-break`.
fn push_collapsed(paragraph: &mut String, text: &str) {
    for c in text.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\r') {
            if !paragraph.is_empty() && !paragraph.ends_with(' ') {
                paragraph.push(' ');
            }
        } else {
            paragraph.push(c);
        }
    }
}

fn is_paragraph(e: &Element) -> bool {
    e.is(Ns::OdfText, "p") || e.is(Ns::OdfText, "h")
}

/// Elementi il cui testo non fa parte del documento visibile.
fn is_excluded(e: &Element) -> bool {
    e.is(Ns::OdfText, "tracked-changes")
        || e.is(Ns::OdfText, "note-citation")
        || e.is(Ns::OdfOffice, "annotation")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paragraphs(xml: &str) -> Vec<String> {
        let mut reader = XmlReader::new(xml.as_bytes());
        let mut collector = Paragraphs::default();
        let mut out = Vec::new();
        loop {
            match reader.next() {
                Ok(XmlEvent::Eof) | Err(_) => return out,
                Ok(event) => {
                    if let Ok(Some(text)) = collector.feed(&mut reader, event) {
                        out.push(text);
                    }
                }
            }
        }
    }

    #[test]
    fn spaces_tabs_notes_and_annotations() {
        let xml = r#"<office:text xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
            xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
            <text:h>Titolo</text:h>
            <text:p>a<text:s text:c="3"/>b<text:tab/>c<text:line-break/>d</text:p>
            <text:p>nota<text:note><text:note-citation>1</text:note-citation>
              <text:note-body><text:p>in fondo</text:p></text:note-body></text:note>
              <office:annotation><text:p>commento</text:p></office:annotation></text:p>
            <text:tracked-changes><text:p>cancellato</text:p></text:tracked-changes>
            </office:text>"#;
        let trimmed: Vec<String> = paragraphs(xml)
            .iter()
            .map(|p| p.trim().to_owned())
            .collect();
        assert_eq!(trimmed, ["Titolo", "a   b\tc\nd", "in fondo", "nota"]);
    }
}
