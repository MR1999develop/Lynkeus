// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Libri EPUB (2 e 3): `META-INF/container.xml` → pacchetto OPF → spine → capitoli XHTML.
//!
//! - Il libro è un contenitore: ogni capitolo della spine è un documento con `inner` = il
//!   suo percorso nel pacchetto (`OEBPS/cap2.xhtml`) e formato [`Format::Epub`]. Un capitolo
//!   ripetuto nella spine si legge una volta; quelli fuori dalla spine (indice, copertina
//!   non elencata) no. Come per gli archivi, i capitoli sono un livello più in profondità e
//!   contano tra le voci del file radice.
//! - Un'unità per blocco di testo non vuoto del capitolo ([`Location::Paragraph`], numerati
//!   da 1 in ogni capitolo): paragrafi, titoli, voci di elenco, celle, citazioni, `div` con
//!   testo proprio. Gli spazi si riducono a uno, `br` è un a capo; `head`, `script` e `style`
//!   esclusi.
//! - XML letto con [`XmlReader`]: nei file del pacchetto `<!DOCTYPE>` è un errore; nei
//!   capitoli è ignorato se non ha un sottoinsieme interno (quello che definisce entità), e
//!   le entità HTML comuni sono risolte ([`XmlReader::xhtml`]).
//! - `mimetype`, se c'è, deve essere `application/epub+zip`; senza `container.xml` il file
//!   non è un EPUB ("contenuto diverso dall'estensione"). I capitoli cifrati elencati in
//!   `META-INF/encryption.xml` (escluso l'offuscamento dei font) danno "protetto da
//!   password".
//!
//! I byte si leggono con i limiti degli archivi, come per docx e odt.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};

use lynkeus_core::Location;

use crate::error::ExtractError;
use crate::event::{Stopped, WarningKind};
use crate::extract::archive::{Container, check_depth, entry_label};
use crate::extract::budget::classify_io;
use crate::extract::ooxml::resolve_target;
use crate::extract::xml::{Element, Ns, XmlEvent, XmlReader};
use crate::extract::{Ctx, Fail, Input};
use crate::format::Format;

const MIMETYPE: &str = "mimetype";
const EPUB_MIME: &str = "application/epub+zip";
const MIMETYPE_MAX_BYTES: u64 = 256;
const CONTAINER_XML: &str = "META-INF/container.xml";
const ENCRYPTION_XML: &str = "META-INF/encryption.xml";
const OPF_MEDIA_TYPE: &str = "application/oebps-package+xml";
/// Tipi dei capitoli letti.
const CHAPTER_MEDIA_TYPES: &[&str] = &["application/xhtml+xml", "text/html"];
/// Algoritmi di offuscamento dei font: non cifrano il testo.
const FONT_OBFUSCATION: &[&str] = &[
    "http://www.idpf.org/2008/embedding",
    "http://ns.adobe.com/pdf/enc#RC",
];

/// Elementi XHTML che sono blocchi di testo.
const BLOCKS: &[&str] = &[
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "dt",
    "dd",
    "blockquote",
    "pre",
    "div",
    "td",
    "th",
    "caption",
    "figcaption",
    "address",
    "section",
    "article",
    "aside",
    "header",
    "footer",
    "nav",
    "main",
    "figure",
    "body",
];
/// Elementi XHTML il cui contenuto non è testo del libro.
const SKIPPED: &[&str] = &["head", "script", "style"];

fn corrupt(reason: &str) -> ExtractError {
    ExtractError::Corrupt {
        reason: reason.to_owned(),
    }
}

/// Estrae i capitoli di un EPUB, ognuno come documento.
///
/// Il libro è un contenitore come gli archivi: si apre solo se i capitoli stanno entro la
/// profondità massima, e i capitoli contano tra le voci del file radice.
pub(crate) fn extract(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    check_depth(ctx)?;
    let mut container = Container::open(ctx, input)?;
    if let Some(part) = container.part(ctx, MIMETYPE)? {
        let mut declared = Vec::new();
        part.take(MIMETYPE_MAX_BYTES)
            .read_to_end(&mut declared)
            .map_err(classify_io)?;
        if !declared.trim_ascii().starts_with(EPUB_MIME.as_bytes()) {
            return Err(ExtractError::ContentMismatch.into());
        }
    }
    let Some(part) = container.part(ctx, CONTAINER_XML)? else {
        return Err(ExtractError::ContentMismatch.into());
    };
    let opf_path = rootfile(XmlReader::new(BufReader::new(part)))?
        .ok_or_else(|| corrupt("EPUB senza pacchetto OPF"))?;
    let encrypted = match container.part(ctx, ENCRYPTION_XML)? {
        Some(part) => encrypted_paths(XmlReader::new(BufReader::new(part)))?,
        None => HashSet::new(),
    };
    let Some(opf) = container.part(ctx, &opf_path)? else {
        return Err(corrupt("pacchetto OPF assente").into());
    };
    let base = opf_path.rsplit_once('/').map_or("", |(dir, _)| dir);
    let mut seen = HashSet::new();
    let chapters: Vec<String> = spine(XmlReader::new(BufReader::new(opf)), base)?
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect();
    ctx.budget()
        .add_entries(u64::try_from(chapters.len()).unwrap_or(u64::MAX))
        .map_err(ExtractError::LimitExceeded)?;
    for path in chapters {
        let is_encrypted = encrypted.contains(&path);
        ctx.within(entry_label(&path), |ctx| {
            read_chapter(ctx, &mut container, &path, is_encrypted)
        })?;
        if ctx.budget().exhausted() {
            break;
        }
    }
    Ok(())
}

/// Il percorso del pacchetto OPF (`full-path` del primo `rootfile` OPF).
fn rootfile<R: BufRead>(mut xml: XmlReader<R>) -> Result<Option<String>, ExtractError> {
    let mut first = None;
    loop {
        match xml.next()? {
            XmlEvent::Open(e) | XmlEvent::Empty(e) if e.is(Ns::OdfContainer, "rootfile") => {
                let Some(path) = e.attr(Ns::Other, "full-path") else {
                    continue;
                };
                let path = percent_decode(path);
                if e.attr(Ns::Other, "media-type") == Some(OPF_MEDIA_TYPE) {
                    return Ok(Some(path));
                }
                first.get_or_insert(path);
            }
            XmlEvent::Eof => return Ok(first),
            _ => {}
        }
    }
}

/// I percorsi (dalla radice del pacchetto) cifrati con un algoritmo che non è
/// l'offuscamento dei font.
fn encrypted_paths<R: BufRead>(mut xml: XmlReader<R>) -> Result<HashSet<String>, ExtractError> {
    let mut paths = HashSet::new();
    let mut algorithm: Option<String> = None;
    loop {
        match xml.next()? {
            XmlEvent::Open(e) if e.is(Ns::XmlEnc, "EncryptedData") => algorithm = None,
            XmlEvent::Open(e) | XmlEvent::Empty(e) if e.is(Ns::XmlEnc, "EncryptionMethod") => {
                algorithm = e.attr(Ns::Other, "Algorithm").map(str::to_owned);
            }
            XmlEvent::Open(e) | XmlEvent::Empty(e) if e.is(Ns::XmlEnc, "CipherReference") => {
                let obfuscation = algorithm
                    .as_deref()
                    .is_some_and(|a| FONT_OBFUSCATION.contains(&a));
                if let Some(uri) = e.attr(Ns::Other, "URI")
                    && !obfuscation
                    && let Some(path) = resolve_target("", &percent_decode(uri))
                {
                    paths.insert(path);
                }
            }
            XmlEvent::Eof => return Ok(paths),
            _ => {}
        }
    }
}

/// I capitoli della spine, come percorsi dalla radice del pacchetto; `base` è la cartella
/// dell'OPF.
fn spine<R: BufRead>(mut xml: XmlReader<R>, base: &str) -> Result<Vec<String>, ExtractError> {
    let mut manifest: HashMap<String, (String, String)> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    loop {
        match xml.next()? {
            XmlEvent::Open(e) | XmlEvent::Empty(e) if e.is(Ns::Opf, "item") => {
                if let (Some(id), Some(href)) = (e.attr(Ns::Other, "id"), e.attr(Ns::Other, "href"))
                {
                    let media = e.attr(Ns::Other, "media-type").unwrap_or_default();
                    manifest.insert(id.to_owned(), (href.to_owned(), media.to_owned()));
                }
            }
            XmlEvent::Open(e) | XmlEvent::Empty(e) if e.is(Ns::Opf, "itemref") => {
                if let Some(idref) = e.attr(Ns::Other, "idref") {
                    order.push(idref.to_owned());
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok(order
        .iter()
        .filter_map(|id| manifest.get(id))
        .filter(|(_, media)| CHAPTER_MEDIA_TYPES.contains(&media.as_str()))
        .filter_map(|(href, _)| {
            let href = href.split('#').next().unwrap_or_default();
            resolve_target(base, &percent_decode(href))
        })
        .collect())
}

/// Decodifica le sequenze `%hh` di un URI relativo (UTF-8; le sequenze non valide restano).
fn percent_decode(uri: &str) -> String {
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0_usize;
    while let Some(&b) = bytes.get(at) {
        let decoded = (b == b'%')
            .then(|| bytes.get(at + 1..at + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match decoded {
            Some(value) => {
                out.push(value);
                at += 3;
            }
            None => {
                out.push(b);
                at += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| uri.to_owned())
}

/// Legge un capitolo già "entrato" nel contesto; gli errori diventano avvisi del capitolo.
fn read_chapter(
    ctx: &mut Ctx<'_>,
    container: &mut Container,
    path: &str,
    encrypted: bool,
) -> Result<(), Stopped> {
    if encrypted {
        return ctx.warn(WarningKind::Encrypted);
    }
    ctx.begin(Format::Epub);
    match chapter_paragraphs(ctx, container, path) {
        Ok(()) => ctx.finish(),
        Err(Fail::Stopped) => Err(Stopped),
        Err(Fail::Error(error)) => ctx.report(error),
    }
}

/// I blocchi di testo di un capitolo.
fn chapter_paragraphs(
    ctx: &mut Ctx<'_>,
    container: &mut Container,
    path: &str,
) -> Result<(), Fail> {
    let Some(part) = container.part(ctx, path)? else {
        return Err(corrupt("capitolo assente").into());
    };
    let mut xml = XmlReader::xhtml(BufReader::new(part));
    let mut open: Vec<String> = Vec::new();
    let mut number = 0_u32;
    loop {
        match xml.next()? {
            XmlEvent::Open(e) if is_html(&e, SKIPPED) => xml.skip_subtree()?,
            XmlEvent::Open(e) if is_html(&e, BLOCKS) => open.push(String::new()),
            XmlEvent::Empty(e) if is_html(&e, &["br"]) => {
                if let Some(top) = open.last_mut() {
                    top.push('\n');
                }
            }
            XmlEvent::Close(ns, local) if html_ns(ns) && BLOCKS.contains(&local.as_str()) => {
                let text = open.pop().map(|t| tidy(&t)).unwrap_or_default();
                if !text.is_empty() {
                    number = number.saturating_add(1);
                    ctx.unit(Location::Paragraph(number), &text)?;
                }
            }
            XmlEvent::Text(text) => {
                if let Some(top) = open.last_mut() {
                    push_collapsed(top, &text);
                }
            }
            XmlEvent::Eof => return Ok(()),
            _ => {}
        }
    }
}

fn html_ns(ns: Ns) -> bool {
    matches!(ns, Ns::Xhtml | Ns::Other)
}

fn is_html(element: &Element, names: &[&str]) -> bool {
    html_ns(element.ns) && names.contains(&element.local.as_str())
}

/// Aggiunge `text` riducendo ogni sequenza di spazi a uno.
fn push_collapsed(out: &mut String, text: &str) {
    for c in text.chars() {
        if c.is_whitespace() {
            if !out.is_empty() && !out.ends_with([' ', '\n']) {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
}

/// Toglie gli spazi ai bordi di ogni riga e le righe vuote.
fn tidy(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("cap%201.xhtml"), "cap 1.xhtml");
        assert_eq!(percent_decode("citt%C3%A0.xhtml"), "città.xhtml");
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("a%zz"), "a%zz");
        assert_eq!(percent_decode("%FF"), "%FF");
    }

    #[test]
    fn spine_resolves_and_filters() {
        let opf = r#"<package xmlns="http://www.idpf.org/2007/opf">
            <manifest>
              <item id="c1" href="testo/cap%201.xhtml" media-type="application/xhtml+xml"/>
              <item id="c2" href="cap2.xhtml#inizio" media-type="application/xhtml+xml"/>
              <item id="img" href="a.png" media-type="image/png"/>
              <item id="out" href="../../fuori.xhtml" media-type="application/xhtml+xml"/>
            </manifest>
            <spine><itemref idref="c2"/><itemref idref="img"/><itemref idref="c1"/>
              <itemref idref="assente"/><itemref idref="out"/></spine>
            </package>"#;
        let chapters = spine(XmlReader::new(opf.as_bytes()), "OEBPS");
        assert_eq!(
            chapters.ok(),
            Some(vec![
                "OEBPS/cap2.xhtml".to_owned(),
                "OEBPS/testo/cap 1.xhtml".to_owned()
            ])
        );
    }

    #[test]
    fn encryption_ignores_font_obfuscation() {
        let xml = r#"<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container"
            xmlns:enc="http://www.w3.org/2001/04/xmlenc#">
            <enc:EncryptedData><enc:EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/>
              <enc:CipherData><enc:CipherReference URI="font.otf"/></enc:CipherData></enc:EncryptedData>
            <enc:EncryptedData><enc:EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes128-cbc"/>
              <enc:CipherData><enc:CipherReference URI="OEBPS/cap1.xhtml"/></enc:CipherData></enc:EncryptedData>
            </encryption>"#;
        let paths = encrypted_paths(XmlReader::new(xml.as_bytes())).unwrap_or_default();
        assert_eq!(paths.len(), 1);
        assert!(paths.contains("OEBPS/cap1.xhtml"));
    }

    #[test]
    fn whitespace_is_collapsed() {
        let mut text = String::new();
        push_collapsed(&mut text, "  Al \n porto\t\t");
        push_collapsed(&mut text, "si vende");
        text.push('\n');
        push_collapsed(&mut text, " poi");
        assert_eq!(tidy(&text), "Al porto si vende\npoi");
    }
}
