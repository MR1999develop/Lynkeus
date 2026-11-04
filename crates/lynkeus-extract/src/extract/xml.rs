// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Lettore XML per docx, pptx e ODF: `quick-xml` con i namespace risolti, senza DTD.
//!
//! - `<!DOCTYPE>` è un errore ([`ExtractError::Corrupt`]): niente entità definite dal
//!   documento, quindi niente "billion laughs" né risorse esterne.
//! - Si risolvono solo i riferimenti predefiniti (`&amp;`, `&lt;` …) e quelli numerici; un
//!   riferimento sconosciuto è un errore.
//! - Gli elementi si riconoscono per namespace e nome locale, non per prefisso.
//!
//! La quantità di XML letta la limita chi fornisce la sorgente (un
//! [`LimitedReader`](super::budget::LimitedReader)).

use std::io::BufRead;

use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::{NsReader, XmlVersion};

use crate::error::ExtractError;
use crate::extract::budget::classify_io_ref;

/// Namespace che gli estrattori usano.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ns {
    /// WordprocessingML.
    Word,
    /// DrawingML.
    Drawing,
    /// PresentationML.
    Presentation,
    /// Relazioni nei documenti OOXML (attributi `r:id`).
    Relationships,
    /// File delle relazioni (`_rels/*.rels`).
    PackageRelationships,
    /// Markup compatibility (`mc:AlternateContent`).
    MarkupCompatibility,
    /// ODF `office:`.
    OdfOffice,
    /// ODF `text:`.
    OdfText,
    /// ODF `draw:`.
    OdfDraw,
    /// ODF `presentation:`.
    OdfPresentation,
    /// ODF `manifest:`.
    OdfManifest,
    /// Nessun namespace o uno che non interessa.
    Other,
}

/// URI dei namespace, compresi quelli di OOXML "strict".
const NAMESPACES: &[(&str, Ns)] = &[
    (
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
        Ns::Word,
    ),
    ("http://purl.oclc.org/ooxml/wordprocessingml/main", Ns::Word),
    (
        "http://schemas.openxmlformats.org/drawingml/2006/main",
        Ns::Drawing,
    ),
    ("http://purl.oclc.org/ooxml/drawingml/main", Ns::Drawing),
    (
        "http://schemas.openxmlformats.org/presentationml/2006/main",
        Ns::Presentation,
    ),
    (
        "http://purl.oclc.org/ooxml/presentationml/main",
        Ns::Presentation,
    ),
    (
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
        Ns::Relationships,
    ),
    (
        "http://purl.oclc.org/ooxml/officeDocument/relationships",
        Ns::Relationships,
    ),
    (
        "http://schemas.openxmlformats.org/package/2006/relationships",
        Ns::PackageRelationships,
    ),
    (
        "http://schemas.openxmlformats.org/markup-compatibility/2006",
        Ns::MarkupCompatibility,
    ),
    (
        "urn:oasis:names:tc:opendocument:xmlns:office:1.0",
        Ns::OdfOffice,
    ),
    (
        "urn:oasis:names:tc:opendocument:xmlns:text:1.0",
        Ns::OdfText,
    ),
    (
        "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0",
        Ns::OdfDraw,
    ),
    (
        "urn:oasis:names:tc:opendocument:xmlns:presentation:1.0",
        Ns::OdfPresentation,
    ),
    (
        "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0",
        Ns::OdfManifest,
    ),
];

fn namespace(result: &ResolveResult<'_>) -> Ns {
    match result {
        ResolveResult::Bound(ns) => NAMESPACES
            .iter()
            .find(|(uri, _)| *uri == ns.as_ref())
            .map_or(Ns::Other, |(_, ns)| *ns),
        ResolveResult::Unbound | ResolveResult::Unknown(_) => Ns::Other,
    }
}

/// Un elemento aperto, con gli attributi già risolti e decodificati.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Element {
    pub ns: Ns,
    pub local: String,
    attrs: Vec<(Ns, String, String)>,
}

impl Element {
    /// Vero se l'elemento è `local` nel namespace `ns`.
    pub fn is(&self, ns: Ns, local: &str) -> bool {
        self.ns == ns && self.local == local
    }

    /// Il valore dell'attributo `local` nel namespace `ns` (`Ns::Other` per gli attributi
    /// senza prefisso).
    pub fn attr(&self, ns: Ns, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, l, _)| *n == ns && l == local)
            .map(|(_, _, v)| v.as_str())
    }
}

/// Un evento XML semplificato.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum XmlEvent {
    /// Elemento aperto (`<a>`).
    Open(Element),
    /// Elemento vuoto (`<a/>`): non arriva la sua chiusura.
    Empty(Element),
    /// Elemento chiuso (`</a>`).
    Close(Ns, String),
    /// Un pezzo di testo (riferimenti già risolti); più pezzi di seguito sono contigui.
    Text(String),
    /// Fine del documento.
    Eof,
}

/// Lettore XML a eventi.
pub(crate) struct XmlReader<R: BufRead> {
    reader: NsReader<R>,
    buf: Vec<u8>,
}

impl<R: BufRead> XmlReader<R> {
    /// Un lettore sulla sorgente `source`.
    pub fn new(source: R) -> XmlReader<R> {
        XmlReader {
            reader: NsReader::from_reader(source),
            buf: Vec::new(),
        }
    }

    /// Il prossimo evento. Errori: [`ExtractError::Corrupt`] per XML non valido o con DTD,
    /// quelli della sorgente (limiti, I/O) classificati.
    pub fn next(&mut self) -> Result<XmlEvent, ExtractError> {
        loop {
            self.buf.clear();
            let (resolved, event) = self
                .reader
                .read_resolved_event_into(&mut self.buf)
                .map_err(xml_error)?;
            let ns = namespace(&resolved);
            let empty = matches!(event, Event::Empty(_));
            let out = match event {
                Event::Start(start) | Event::Empty(start) => {
                    let local = start.local_name().as_ref().to_owned();
                    let mut attrs = Vec::new();
                    for attr in start.attributes() {
                        let attr = attr.map_err(|_| corrupt("attributo XML non valido"))?;
                        let (attr_ns, attr_local) =
                            self.reader.resolver_mut().resolve_attribute(attr.key);
                        let attr_ns = namespace(&attr_ns);
                        let attr_local = attr_local.as_ref().to_owned();
                        let value = attr
                            .normalized_value(XmlVersion::Implicit1_0)
                            .map_err(xml_error)?
                            .into_owned();
                        attrs.push((attr_ns, attr_local, value));
                    }
                    let element = Element { ns, local, attrs };
                    if empty {
                        XmlEvent::Empty(element)
                    } else {
                        XmlEvent::Open(element)
                    }
                }
                Event::End(end) => XmlEvent::Close(ns, end.local_name().as_ref().to_owned()),
                Event::Text(text) => XmlEvent::Text(text.xml10_content().into_owned()),
                Event::CData(data) => XmlEvent::Text(data.xml10_content().into_owned()),
                Event::GeneralRef(reference) => {
                    if reference.is_char_ref() {
                        match reference.resolve_char_ref() {
                            Ok(Some(c)) => XmlEvent::Text(c.to_string()),
                            Ok(None) | Err(_) => {
                                return Err(corrupt("riferimento a carattere non valido"));
                            }
                        }
                    } else {
                        match quick_xml::escape::resolve_predefined_entity(&reference) {
                            Some(text) => XmlEvent::Text(text.to_owned()),
                            None => return Err(corrupt("entità XML non definita")),
                        }
                    }
                }
                Event::DocType(_) => return Err(corrupt("DTD nel documento XML")),
                Event::Decl(_) | Event::PI(_) | Event::Comment(_) => continue,
                Event::Eof => XmlEvent::Eof,
            };
            return Ok(out);
        }
    }

    /// Salta il contenuto dell'elemento appena aperto (dopo [`XmlEvent::Open`]), fino alla
    /// sua chiusura compresa.
    pub fn skip_subtree(&mut self) -> Result<(), ExtractError> {
        let mut depth = 1_usize;
        while depth > 0 {
            match self.next()? {
                XmlEvent::Open(_) => depth += 1,
                XmlEvent::Close(..) => depth -= 1,
                XmlEvent::Eof => return Err(corrupt("XML troncato")),
                XmlEvent::Empty(_) | XmlEvent::Text(_) => {}
            }
        }
        Ok(())
    }

    /// Legge fino alla fine, solo per verificare che l'XML sia valido e senza DTD.
    pub fn validate(&mut self) -> Result<(), ExtractError> {
        while self.next()? != XmlEvent::Eof {}
        Ok(())
    }
}

fn corrupt(reason: &str) -> ExtractError {
    ExtractError::Corrupt {
        reason: reason.to_owned(),
    }
}

/// Errore di `quick-xml`: quelli della sorgente (limiti, I/O) restano tali, il resto è XML
/// non valido. Il testo dell'errore non si conserva: potrebbe contenere pezzi del documento.
fn xml_error(error: quick_xml::Error) -> ExtractError {
    match error {
        quick_xml::Error::Io(io) => classify_io_ref(&io),
        _ => corrupt("XML non valido"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(xml: &str) -> Result<Vec<XmlEvent>, ExtractError> {
        let mut reader = XmlReader::new(xml.as_bytes());
        let mut out = Vec::new();
        loop {
            let event = reader.next()?;
            if event == XmlEvent::Eof {
                return Ok(out);
            }
            out.push(event);
        }
    }

    fn text_of(xml: &str) -> Result<String, ExtractError> {
        Ok(events(xml)?
            .into_iter()
            .filter_map(|e| match e {
                XmlEvent::Text(t) => Some(t),
                _ => None,
            })
            .collect())
    }

    #[test]
    fn doctype_is_rejected() {
        let xml = r#"<?xml version="1.0"?><!DOCTYPE a [<!ENTITY x "aragosta">]><a>&x;</a>"#;
        assert!(matches!(events(xml), Err(ExtractError::Corrupt { .. })));
    }

    #[test]
    fn undefined_entity_is_rejected() {
        assert!(matches!(
            text_of("<a>&lol;</a>"),
            Err(ExtractError::Corrupt { .. })
        ));
    }

    #[test]
    fn references_are_resolved() {
        assert_eq!(
            text_of("<a>citt&#224; &amp; &lt;mare&gt; &#x41;</a>").ok(),
            Some("città & <mare> A".to_owned())
        );
        assert_eq!(
            text_of("<a><![CDATA[x<y]]></a>").ok(),
            Some("x<y".to_owned())
        );
    }

    #[test]
    fn namespaces_are_resolved_not_prefixes() {
        let xml = r#"<x:p xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
            xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
            r:id="rId1" id="7"/>"#;
        let list = events(xml).unwrap_or_default();
        let Some(XmlEvent::Empty(element)) = list.first() else {
            panic!("atteso un elemento vuoto: {list:?}");
        };
        assert!(element.is(Ns::Word, "p"));
        assert_eq!(element.attr(Ns::Relationships, "id"), Some("rId1"));
        assert_eq!(element.attr(Ns::Other, "id"), Some("7"));
    }

    #[test]
    fn malformed_xml_is_corrupt() {
        assert!(matches!(
            events("<a><b></a>"),
            Err(ExtractError::Corrupt { .. })
        ));
    }

    #[test]
    fn skip_subtree_stops_after_close() {
        let mut reader = XmlReader::new("<a><b><c/>testo</b>dopo</a>".as_bytes());
        assert!(matches!(reader.next(), Ok(XmlEvent::Open(_))));
        assert!(matches!(reader.next(), Ok(XmlEvent::Open(_))));
        assert!(reader.skip_subtree().is_ok());
        assert_eq!(reader.next().ok(), Some(XmlEvent::Text("dopo".to_owned())));
    }
}
