// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Formati dei documenti: scelta dall'estensione, verifica dal contenuto e registro degli
//! estrattori, tutto in questo file.
//!
//! - [`Format::for_path`] e [`Format::for_name`] scelgono il formato dall'estensione (ASCII,
//!   senza distinguere maiuscole, dopo l'ultimo punto dell'ultimo componente). Un'estensione
//!   che non è nel registro dà `None`: il file si legge come testo (il motore lo fa nel
//!   processo principale, l'estrattore lo fa per le voci degli archivi). Nessun formato si
//!   prova "per vedere".
//! - [`Signature::sniff`] riconosce il contenuto dai primi [`SNIFF_BYTES`] byte.
//! - Il formato dell'estensione si legge solo se il contenuto ha la firma attesa; altrimenti
//!   si guarda la tabella dei casi innocui ([`HARMLESS`]); se non c'è una riga, avviso
//!   "contenuto diverso dall'estensione". Unica eccezione: un documento OOXML che contiene un
//!   file composto (CFB) è il contenitore di un OOXML cifrato, e si controlla come tale.
//!
//! **Fase 5 (nuovi formati).** Per aggiungere un formato:
//! 1. una variante in fondo a [`Format`] (l'ordine conta per `postcard`: le varianti esistenti
//!    non si spostano; se cambia il significato di una, si alza
//!    [`PROTOCOL_VERSION`](crate::protocol::PROTOCOL_VERSION));
//! 2. se serve, una variante di [`Signature`] e il suo riconoscimento in [`Signature::sniff`];
//! 3. una riga in `REGISTRY` con estensioni, firma attesa, tipo (documento o archivio) e
//!    funzione d'estrazione (in un modulo nuovo di `extract/`);
//! 4. le righe dei casi innocui in [`HARMLESS`], per esempio `(Format::Doc, Signature::Rtf,
//!    Format::Rtf)` per un .doc che in realtà è RTF e `(Format::Xls, Signature::Html,
//!    Format::Html)` per un .xls che è una pagina HTML.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::extract::{Ctx, Fail, Input, archive, ooxml, opendoc, pdf, plain, sheet};

/// Byte iniziali letti per riconoscere il contenuto.
pub const SNIFF_BYTES: usize = 1024;

/// Un formato che l'estrattore sa leggere.
///
/// Viaggia nel protocollo e nella cache: nuove varianti solo in fondo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Format {
    /// Testo semplice (voci degli archivi senza un formato riconosciuto).
    Text,
    /// Word 2007 e successivi.
    Docx,
    /// Excel 2007 e successivi.
    Xlsx,
    /// PowerPoint 2007 e successivi.
    Pptx,
    /// Testo OpenDocument.
    Odt,
    /// Foglio di calcolo OpenDocument.
    Ods,
    /// Presentazione OpenDocument.
    Odp,
    /// PDF.
    Pdf,
    /// Archivio zip.
    Zip,
}

/// Che cosa dicono i primi byte di un file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Signature {
    /// Archivio zip (anche docx, xlsx, pptx e ODF).
    Zip,
    /// PDF: `%PDF-` nei primi [`SNIFF_BYTES`] byte.
    Pdf,
    /// File composto OLE (CFB): formati Office 97-2003 e contenitori OOXML cifrati.
    Cfb,
    /// RTF.
    Rtf,
    /// Nessuna firma nota.
    Unknown,
}

const ZIP_SIGNATURES: &[&[u8]] = &[b"PK\x03\x04", b"PK\x05\x06", b"PK\x07\x08"];
const CFB_SIGNATURE: &[u8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1";
const RTF_SIGNATURE: &[u8] = b"{\\rtf";
const PDF_MARKER: &[u8] = b"%PDF-";

impl Signature {
    /// Riconosce il contenuto dai primi byte (ne bastano [`SNIFF_BYTES`]).
    pub fn sniff(head: &[u8]) -> Signature {
        let head = head.get(..SNIFF_BYTES).unwrap_or(head);
        if ZIP_SIGNATURES.iter().any(|s| head.starts_with(s)) {
            Signature::Zip
        } else if head.starts_with(CFB_SIGNATURE) {
            Signature::Cfb
        } else if head.starts_with(RTF_SIGNATURE) {
            Signature::Rtf
        } else if memchr::memmem::find(head, PDF_MARKER).is_some() {
            // La specifica ammette byte prima dell'intestazione: i lettori la cercano nel
            // primo kilobyte.
            Signature::Pdf
        } else {
            Signature::Unknown
        }
    }
}

/// Documento con testo proprio o contenitore di altri documenti.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Produce unità di testo, tra un inizio e una fine documento.
    Document,
    /// Contiene voci, ognuna un documento a sé (non ha inizio né fine documento).
    Archive,
}

/// Funzione d'estrazione di un formato.
pub(crate) type ExtractFn = fn(&mut Ctx<'_>, Input<'_>) -> Result<(), Fail>;

/// Una riga del registro.
pub(crate) struct FormatSpec {
    pub format: Format,
    /// Estensioni in minuscolo, senza punto.
    pub extensions: &'static [&'static str],
    /// Firma attesa; `None` = qualsiasi contenuto (testo).
    pub signature: Option<Signature>,
    pub kind: Kind,
    pub extract: ExtractFn,
}

/// Il registro: l'unico posto che lega estensioni, contenuto ed estrattori.
const REGISTRY: &[FormatSpec] = &[
    FormatSpec {
        format: Format::Text,
        extensions: &[],
        signature: None,
        kind: Kind::Document,
        extract: plain::extract,
    },
    FormatSpec {
        format: Format::Docx,
        extensions: &["docx", "docm", "dotx", "dotm"],
        signature: Some(Signature::Zip),
        kind: Kind::Document,
        extract: ooxml::extract_docx,
    },
    FormatSpec {
        format: Format::Xlsx,
        extensions: &["xlsx", "xlsm", "xltx", "xltm"],
        signature: Some(Signature::Zip),
        kind: Kind::Document,
        extract: sheet::extract_xlsx,
    },
    FormatSpec {
        format: Format::Pptx,
        extensions: &["pptx", "pptm", "potx", "potm", "ppsx", "ppsm"],
        signature: Some(Signature::Zip),
        kind: Kind::Document,
        extract: ooxml::extract_pptx,
    },
    FormatSpec {
        format: Format::Odt,
        extensions: &["odt", "ott"],
        signature: Some(Signature::Zip),
        kind: Kind::Document,
        extract: opendoc::extract_odt,
    },
    FormatSpec {
        format: Format::Ods,
        extensions: &["ods", "ots"],
        signature: Some(Signature::Zip),
        kind: Kind::Document,
        extract: sheet::extract_ods,
    },
    FormatSpec {
        format: Format::Odp,
        extensions: &["odp", "otp"],
        signature: Some(Signature::Zip),
        kind: Kind::Document,
        extract: opendoc::extract_odp,
    },
    FormatSpec {
        format: Format::Pdf,
        extensions: &["pdf"],
        signature: Some(Signature::Pdf),
        kind: Kind::Document,
        extract: pdf::extract,
    },
    FormatSpec {
        format: Format::Zip,
        extensions: &["zip"],
        signature: Some(Signature::Zip),
        kind: Kind::Archive,
        extract: archive::extract_zip,
    },
];

/// Casi innocui in cui contenuto ed estensione non coincidono ma il file si legge lo stesso
/// con l'estrattore del contenuto: (formato dell'estensione, firma trovata, formato da usare).
///
/// Vuota finché non ci sono gli estrattori della fase 5 (RTF, HTML): fino ad allora questi
/// file danno l'avviso "contenuto diverso dall'estensione".
pub const HARMLESS: &[(Format, Signature, Format)] = &[];

/// Esito della verifica del contenuto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Si legge con questo formato.
    Read(Format),
    /// Documento OOXML dentro un file composto: probabilmente cifrato, da controllare.
    EncryptedOoxmlCandidate,
    /// Contenuto diverso dall'estensione.
    Mismatch,
}

impl Format {
    /// Il formato di un file dal suo percorso, o `None` se l'estensione non è nel registro.
    pub fn for_path(path: &Path) -> Option<Format> {
        path.file_name()
            .and_then(|name| name.to_str())
            .and_then(Format::for_name)
    }

    /// Il formato dal nome di un file o di una voce d'archivio ("cartella/doc.DOCX"), o
    /// `None` se l'estensione non è nel registro. Separatori `/` e `\` entrambi ammessi.
    pub fn for_name(name: &str) -> Option<Format> {
        let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
        let (stem, extension) = file.rsplit_once('.')?;
        if stem.is_empty() || extension.is_empty() {
            return None;
        }
        Format::for_extension(extension)
    }

    /// Il formato di un'estensione senza punto ("pdf", "Docx"), o `None`.
    pub fn for_extension(extension: &str) -> Option<Format> {
        REGISTRY
            .iter()
            .find(|spec| {
                spec.extensions
                    .iter()
                    .any(|e| e.eq_ignore_ascii_case(extension))
            })
            .map(|spec| spec.format)
    }

    /// Vero per i contenitori di altri documenti (archivi).
    pub fn is_archive(self) -> bool {
        spec(self).kind == Kind::Archive
    }

    /// Vero per i formati Office Open XML.
    fn is_ooxml(self) -> bool {
        matches!(self, Format::Docx | Format::Xlsx | Format::Pptx)
    }
}

/// La riga del registro di `format`.
pub(crate) fn spec(format: Format) -> &'static FormatSpec {
    REGISTRY
        .iter()
        .find(|spec| spec.format == format)
        .unwrap_or(&REGISTRY[0])
}

/// Decide come leggere un file dichiarato `declared` (dall'estensione) con firma `found`.
pub(crate) fn verify(declared: Format, found: Signature) -> Verdict {
    match spec(declared).signature {
        None => return Verdict::Read(declared),
        Some(expected) if expected == found => return Verdict::Read(declared),
        Some(_) => {}
    }
    if declared.is_ooxml() && found == Signature::Cfb {
        return Verdict::EncryptedOoxmlCandidate;
    }
    HARMLESS
        .iter()
        .find(|(from, signature, _)| *from == declared && *signature == found)
        .map_or(Verdict::Mismatch, |(_, _, to)| Verdict::Read(*to))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_has_exactly_one_spec() {
        for format in [
            Format::Text,
            Format::Docx,
            Format::Xlsx,
            Format::Pptx,
            Format::Odt,
            Format::Ods,
            Format::Odp,
            Format::Pdf,
            Format::Zip,
        ] {
            let rows = REGISTRY.iter().filter(|s| s.format == format).count();
            assert_eq!(rows, 1, "{format:?}");
        }
    }

    #[test]
    fn extensions_are_unique_and_lowercase() {
        let mut seen = std::collections::HashSet::new();
        for spec in REGISTRY {
            for ext in spec.extensions {
                assert_eq!(*ext, ext.to_ascii_lowercase());
                assert!(seen.insert(*ext), "estensione ripetuta: {ext}");
            }
        }
    }

    #[test]
    fn name_partitions() {
        // Caso comune, maiuscole, cartelle di entrambi i sistemi.
        assert_eq!(Format::for_name("relazione.docx"), Some(Format::Docx));
        assert_eq!(Format::for_name("CATALOGO.PDF"), Some(Format::Pdf));
        assert_eq!(Format::for_name("a/b\\c.Ods"), Some(Format::Ods));
        // Doppia estensione: conta l'ultima.
        assert_eq!(Format::for_name("fattura.pdf.exe"), None);
        assert_eq!(Format::for_name("archivio.txt.zip"), Some(Format::Zip));
        // Sconosciute, assenti, solo punto, nome che è solo estensione.
        assert_eq!(Format::for_name("note.txt"), None);
        assert_eq!(Format::for_name("LEGGIMI"), None);
        assert_eq!(Format::for_name("strano."), None);
        assert_eq!(Format::for_name(".pdf"), None);
        assert_eq!(Format::for_name("cartella.pdf/"), None);
        // Un carattere neutralizzato (era NUL) non nasconde l'estensione vera.
        assert_eq!(Format::for_name("a.pdf\u{FFFD}.txt"), None);
        // Fase 5: non ancora nel registro.
        assert_eq!(Format::for_name("vecchia-lettera.doc"), None);
    }

    #[test]
    fn path_with_non_utf8_or_no_name() {
        assert_eq!(
            Format::for_path(Path::new("dir/catalogo.pdf")),
            Some(Format::Pdf)
        );
        assert_eq!(Format::for_path(Path::new("/")), None);
    }

    #[test]
    fn sniff_partitions() {
        assert_eq!(Signature::sniff(b"PK\x03\x04rest"), Signature::Zip);
        assert_eq!(Signature::sniff(b"PK\x05\x06"), Signature::Zip);
        assert_eq!(Signature::sniff(CFB_SIGNATURE), Signature::Cfb);
        assert_eq!(Signature::sniff(b"{\\rtf1\\ansi"), Signature::Rtf);
        assert_eq!(Signature::sniff(b"%PDF-1.7\n"), Signature::Pdf);
        assert_eq!(Signature::sniff(b"junk\n%PDF-1.4"), Signature::Pdf);
        assert_eq!(Signature::sniff(b"\x89PNG\r\n"), Signature::Unknown);
        assert_eq!(Signature::sniff(b""), Signature::Unknown);
        assert_eq!(Signature::sniff(b"PK"), Signature::Unknown);
        // Oltre il primo kilobyte la firma PDF non conta.
        let mut late = vec![b' '; SNIFF_BYTES];
        late.extend_from_slice(b"%PDF-1.4");
        assert_eq!(Signature::sniff(&late), Signature::Unknown);
    }

    #[test]
    fn verify_partitions() {
        assert_eq!(
            verify(Format::Pdf, Signature::Pdf),
            Verdict::Read(Format::Pdf)
        );
        assert_eq!(verify(Format::Pdf, Signature::Unknown), Verdict::Mismatch);
        assert_eq!(verify(Format::Docx, Signature::Rtf), Verdict::Mismatch);
        assert_eq!(
            verify(Format::Docx, Signature::Cfb),
            Verdict::EncryptedOoxmlCandidate
        );
        assert_eq!(verify(Format::Odt, Signature::Cfb), Verdict::Mismatch);
        assert_eq!(
            verify(Format::Text, Signature::Zip),
            Verdict::Read(Format::Text)
        );
    }

    #[test]
    fn archive_kind() {
        assert!(Format::Zip.is_archive());
        assert!(!Format::Docx.is_archive());
    }
}
