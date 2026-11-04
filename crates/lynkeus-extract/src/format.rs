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
//!
//! Un formato raggiungibile solo come caso innocuo (oggi [`Format::Html`]) ha la riga nel
//! registro senza estensioni, come [`Format::Text`]: nessun file arriva lì dal nome.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::extract::{
    Ctx, Fail, Input, archive, eml, epub, html, ooxml, opendoc, pdf, plain, ppt, rtf, sevenz,
    sheet, tarball, word97,
};

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
    /// Word 97-2003.
    Doc,
    /// Excel 97-2003.
    Xls,
    /// PowerPoint 97-2003.
    Ppt,
    /// Rich Text Format.
    Rtf,
    /// Messaggio e-mail (RFC 5322); gli allegati sono documenti a sé.
    Eml,
    /// Libro EPUB: ogni capitolo è un documento a sé.
    Epub,
    /// Archivio 7z.
    SevenZ,
    /// Archivio tar.
    Tar,
    /// File compresso con gzip (anche `.tar.gz` e `.tgz`).
    Gz,
    /// Pagina HTML: solo come caso innocuo (un .xls che è una pagina HTML), mai
    /// dall'estensione.
    Html,
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
    /// Archivio 7z.
    SevenZ,
    /// Flusso gzip.
    Gzip,
    /// Archivio tar POSIX (`ustar` all'offset 257).
    Tar,
    /// Pagina HTML: dopo un eventuale BOM e spazi, `<!DOCTYPE html`, `<html` o un commento
    /// seguito da uno dei due (maiuscole indifferenti).
    Html,
}

const ZIP_SIGNATURES: &[&[u8]] = &[b"PK\x03\x04", b"PK\x05\x06", b"PK\x07\x08"];
const CFB_SIGNATURE: &[u8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1";
const RTF_SIGNATURE: &[u8] = b"{\\rtf";
const PDF_MARKER: &[u8] = b"%PDF-";
const SEVENZ_SIGNATURE: &[u8] = b"7z\xBC\xAF\x27\x1C";
const GZIP_SIGNATURE: &[u8] = b"\x1F\x8B";
/// `ustar` all'offset 257 dell'intestazione tar (POSIX e GNU).
const TAR_MAGIC: &[u8] = b"ustar";
const TAR_MAGIC_OFFSET: usize = 257;
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";
const HTML_STARTS: &[&[u8]] = &[b"<!doctype html", b"<html"];

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
        } else if head.starts_with(SEVENZ_SIGNATURE) {
            Signature::SevenZ
        } else if head.starts_with(GZIP_SIGNATURE) {
            Signature::Gzip
        } else if head
            .get(TAR_MAGIC_OFFSET..)
            .is_some_and(|rest| rest.starts_with(TAR_MAGIC))
        {
            Signature::Tar
        } else if looks_like_html(head) {
            // Prima del PDF: una pagina può citare "%PDF-" nel testo.
            Signature::Html
        } else if memchr::memmem::find(head, PDF_MARKER).is_some() {
            // La specifica ammette byte prima dell'intestazione: i lettori la cercano nel
            // primo kilobyte.
            Signature::Pdf
        } else {
            Signature::Unknown
        }
    }
}

/// Vero se `head`, dopo un BOM UTF-8, spazi e commenti HTML, inizia come una pagina HTML.
fn looks_like_html(head: &[u8]) -> bool {
    let mut rest = head.strip_prefix(UTF8_BOM).unwrap_or(head);
    loop {
        rest = rest.trim_ascii_start();
        if let Some(comment) = rest.strip_prefix(b"<!--") {
            match memchr::memmem::find(comment, b"-->") {
                Some(end) => rest = comment.get(end + 3..).unwrap_or_default(),
                None => return false,
            }
            continue;
        }
        return HTML_STARTS.iter().any(|start| {
            rest.get(..start.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(start))
        });
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
    FormatSpec {
        format: Format::Doc,
        extensions: &["doc", "dot"],
        signature: Some(Signature::Cfb),
        kind: Kind::Document,
        extract: word97::extract,
    },
    FormatSpec {
        format: Format::Xls,
        extensions: &["xls", "xlt"],
        signature: Some(Signature::Cfb),
        kind: Kind::Document,
        extract: sheet::extract_xls,
    },
    FormatSpec {
        format: Format::Ppt,
        extensions: &["ppt", "pot", "pps"],
        signature: Some(Signature::Cfb),
        kind: Kind::Document,
        extract: ppt::extract,
    },
    FormatSpec {
        format: Format::Rtf,
        extensions: &["rtf"],
        signature: Some(Signature::Rtf),
        kind: Kind::Document,
        extract: rtf::extract,
    },
    FormatSpec {
        format: Format::Eml,
        extensions: &["eml"],
        // Un messaggio è testo: nessuna firma da verificare.
        signature: None,
        kind: Kind::Document,
        extract: eml::extract,
    },
    FormatSpec {
        format: Format::Epub,
        extensions: &["epub"],
        signature: Some(Signature::Zip),
        kind: Kind::Archive,
        extract: epub::extract,
    },
    FormatSpec {
        format: Format::SevenZ,
        extensions: &["7z"],
        signature: Some(Signature::SevenZ),
        kind: Kind::Archive,
        extract: sevenz::extract,
    },
    FormatSpec {
        format: Format::Tar,
        extensions: &["tar"],
        signature: Some(Signature::Tar),
        kind: Kind::Archive,
        extract: tarball::extract_tar,
    },
    FormatSpec {
        format: Format::Gz,
        extensions: &["gz", "tgz"],
        signature: Some(Signature::Gzip),
        kind: Kind::Archive,
        extract: tarball::extract_gz,
    },
    FormatSpec {
        format: Format::Html,
        extensions: &[],
        signature: Some(Signature::Html),
        kind: Kind::Document,
        extract: html::extract,
    },
];

/// Casi innocui in cui contenuto ed estensione non coincidono ma il file si legge lo stesso
/// con l'estrattore del contenuto: (formato dell'estensione, firma trovata, formato da usare).
///
/// - un .doc che in realtà è RTF (Word lo apre senza dire nulla);
/// - un .xls che è una pagina HTML (esportazioni di molti gestionali);
/// - un .xls senza firma nota, cioè testo (CSV o tabulato): letto come testo, che salta da
///   sé i contenuti binari.
pub const HARMLESS: &[(Format, Signature, Format)] = &[
    (Format::Doc, Signature::Rtf, Format::Rtf),
    (Format::Xls, Signature::Html, Format::Html),
    (Format::Xls, Signature::Unknown, Format::Text),
];

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
            Format::Doc,
            Format::Xls,
            Format::Ppt,
            Format::Rtf,
            Format::Eml,
            Format::Epub,
            Format::SevenZ,
            Format::Tar,
            Format::Gz,
            Format::Html,
        ] {
            let rows = REGISTRY.iter().filter(|s| s.format == format).count();
            assert_eq!(rows, 1, "{format:?}");
        }
    }

    /// I formati viaggiano nel protocollo e nella cache: l'indice di `postcard` di ogni
    /// variante non deve cambiare.
    #[test]
    fn variant_indices_are_stable() {
        let encoded = |format: Format| postcard::to_stdvec(&format).unwrap_or_default();
        assert_eq!(encoded(Format::Text), [0]);
        assert_eq!(encoded(Format::Zip), [8]);
        assert_eq!(encoded(Format::Doc), [9]);
        assert_eq!(encoded(Format::Html), [18]);
    }

    #[test]
    fn harmless_targets_are_documents_with_a_spec() {
        for (from, _, to) in HARMLESS {
            assert_eq!(spec(*to).format, *to);
            assert_eq!(spec(*from).format, *from);
        }
        assert_eq!(
            verify(Format::Doc, Signature::Rtf),
            Verdict::Read(Format::Rtf)
        );
        assert_eq!(
            verify(Format::Xls, Signature::Html),
            Verdict::Read(Format::Html)
        );
        assert_eq!(
            verify(Format::Xls, Signature::Unknown),
            Verdict::Read(Format::Text)
        );
        // Solo i casi della tabella: un .docx che è RTF, un .doc che è HTML no.
        assert_eq!(verify(Format::Docx, Signature::Rtf), Verdict::Mismatch);
        assert_eq!(verify(Format::Doc, Signature::Html), Verdict::Mismatch);
        assert_eq!(verify(Format::Ppt, Signature::Unknown), Verdict::Mismatch);
        // Un formato raggiungibile solo così non ha estensioni.
        assert_eq!(Format::for_name("pagina.html"), None);
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
        // Formati della fase 5; per `.tar.gz` conta l'ultima estensione.
        assert_eq!(Format::for_name("vecchia-lettera.doc"), Some(Format::Doc));
        assert_eq!(Format::for_name("SPESA.XLS"), Some(Format::Xls));
        assert_eq!(Format::for_name("lezione.pps"), Some(Format::Ppt));
        assert_eq!(Format::for_name("lettera.rtf"), Some(Format::Rtf));
        assert_eq!(Format::for_name("ordine.eml"), Some(Format::Eml));
        assert_eq!(Format::for_name("libro.epub"), Some(Format::Epub));
        assert_eq!(Format::for_name("archivio.7z"), Some(Format::SevenZ));
        assert_eq!(Format::for_name("archivio.tar"), Some(Format::Tar));
        assert_eq!(Format::for_name("archivio.tar.gz"), Some(Format::Gz));
        assert_eq!(Format::for_name("archivio.tgz"), Some(Format::Gz));
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
        // Archivi della fase 5.
        assert_eq!(Signature::sniff(SEVENZ_SIGNATURE), Signature::SevenZ);
        assert_eq!(Signature::sniff(b"\x1F\x8B\x08\x00"), Signature::Gzip);
        let mut tar = vec![0_u8; 512];
        tar[TAR_MAGIC_OFFSET..TAR_MAGIC_OFFSET + 5].copy_from_slice(TAR_MAGIC);
        assert_eq!(Signature::sniff(&tar), Signature::Tar);
        assert_eq!(
            Signature::sniff(&tar[..TAR_MAGIC_OFFSET + 2]),
            Signature::Unknown
        );
    }

    #[test]
    fn html_partitions() {
        assert_eq!(Signature::sniff(b"<html><body>"), Signature::Html);
        assert_eq!(
            Signature::sniff(b"<!DOCTYPE HTML>\n<html>"),
            Signature::Html
        );
        assert_eq!(
            Signature::sniff(b"\xEF\xBB\xBF \r\n<HTML xmlns:o=\"x\">"),
            Signature::Html
        );
        assert_eq!(
            Signature::sniff(b"<!-- esportato --><html>"),
            Signature::Html
        );
        // Commento non chiuso, XML generico, testo che cita un tag.
        assert_eq!(Signature::sniff(b"<!-- <html>"), Signature::Unknown);
        assert_eq!(
            Signature::sniff(b"<?xml version=\"1.0\"?><a/>"),
            Signature::Unknown
        );
        assert_eq!(Signature::sniff(b"a;b;c\n<html>"), Signature::Unknown);
        assert_eq!(Signature::sniff(b"<htm"), Signature::Unknown);
        // Una pagina che cita "%PDF-" resta HTML.
        assert_eq!(Signature::sniff(b"<html>%PDF-1.4"), Signature::Html);
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
        for format in [Format::SevenZ, Format::Tar, Format::Gz, Format::Epub] {
            assert!(format.is_archive(), "{format:?}");
        }
        for format in [
            Format::Doc,
            Format::Xls,
            Format::Ppt,
            Format::Rtf,
            Format::Eml,
        ] {
            assert!(!format.is_archive(), "{format:?}");
        }
    }
}
