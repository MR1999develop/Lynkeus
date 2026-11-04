// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Formati della fase 5 su input costruiti dal test o alterati: troncati, cifrati, con
//! limiti abbassati, annidati, con contenuto diverso dall'estensione. Sempre: sequenza degli
//! eventi valida, nessun panic, l'avviso giusto.

use std::io::{Cursor, Write};
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;
use lynkeus_core::{ByteSize, Location, MailPart, TextUnit};
use lynkeus_extract::{
    ArchiveLimits, ExtractEvent, ExtractOptions, Format, LimitKind, SequenceCheck, SkipReason,
    WarningKind, extract_bytes,
};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

fn fixture(relative: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(relative);
    let bytes = std::fs::read(&path).unwrap_or_default();
    assert!(!bytes.is_empty(), "fixture mancante: {}", path.display());
    bytes
}

fn run_with(name: &str, bytes: Vec<u8>, limits: ArchiveLimits) -> Vec<ExtractEvent> {
    let mut events = Vec::new();
    let options = ExtractOptions { limits };
    assert_eq!(extract_bytes(name, bytes, &options, &mut events), Ok(()));
    let mut check = SequenceCheck::new();
    for event in &events {
        assert_eq!(check.accept(event), Ok(()), "{name}: {events:?}");
    }
    assert_eq!(check.finish(), Ok(()), "{name}");
    events
}

fn run(name: &str, bytes: Vec<u8>) -> Vec<ExtractEvent> {
    run_with(name, bytes, ArchiveLimits::default())
}

fn warnings(events: &[ExtractEvent]) -> Vec<(Vec<String>, WarningKind)> {
    events
        .iter()
        .filter_map(|e| match e {
            ExtractEvent::Warning { inner, kind } => Some((inner.clone(), *kind)),
            _ => None,
        })
        .collect()
}

fn starts(events: &[ExtractEvent]) -> Vec<(Vec<String>, Format)> {
    events
        .iter()
        .filter_map(|e| match e {
            ExtractEvent::DocumentStart { inner, format } => Some((inner.clone(), *format)),
            _ => None,
        })
        .collect()
}

fn texts(events: &[ExtractEvent]) -> Vec<(Location, String)> {
    events
        .iter()
        .filter_map(|e| match e {
            ExtractEvent::Unit(TextUnit { location, text }) => {
                Some((location.clone(), text.clone()))
            }
            _ => None,
        })
        .collect()
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn only_warning(kind: WarningKind) -> Vec<ExtractEvent> {
    vec![ExtractEvent::Warning {
        inner: Vec::new(),
        kind,
    }]
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        assert!(
            writer
                .start_file(*name, SimpleFileOptions::default())
                .is_ok()
        );
        assert!(writer.write_all(data).is_ok());
    }
    writer.finish().map(Cursor::into_inner).unwrap_or_default()
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    assert!(encoder.write_all(data).is_ok());
    encoder.finish().unwrap_or_default()
}

/// Un tar con le voci `(nome, contenuto)` e un collegamento simbolico `link`.
fn tar_of(entries: &[(&str, &[u8])], with_link: bool) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, data) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(data.len()).unwrap_or(0));
        header.set_mode(0o644);
        header.set_cksum();
        assert!(builder.append_data(&mut header, name, *data).is_ok());
    }
    if with_link {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        assert!(
            builder
                .append_link(&mut header, "link", "/etc/passwd")
                .is_ok()
        );
    }
    builder.into_inner().unwrap_or_default()
}

// ── Office 97-2003 ────────────────────────────────────────────────────────────────────────

/// Accende il flag "cifrato" della FIB di un .doc: il flusso `WordDocument` inizia con
/// `wIdent` 0xA5EC (nella fixture è l'unica occorrenza); i flag sono all'offset 0x0A.
fn mark_doc_encrypted(bytes: &mut [u8]) {
    let at = memchr::memmem::find(bytes, b"\xEC\xA5").unwrap_or(usize::MAX);
    let flags = bytes.get_mut(at.wrapping_add(0x0B));
    assert!(flags.is_some(), "FIB non trovata");
    if let Some(flags) = flags {
        *flags |= 0x01;
    }
}

#[test]
fn encrypted_doc_is_a_warning() {
    let mut bytes = fixture("formati/vecchia-lettera.doc");
    mark_doc_encrypted(&mut bytes);
    assert_eq!(
        run("cifrato.doc", bytes),
        only_warning(WarningKind::Encrypted)
    );
}

#[test]
fn truncated_office_97_files_are_corrupt() {
    for name in [
        "vecchia-lettera.doc",
        "vecchia-spesa.xls",
        "vecchia-presentazione.ppt",
    ] {
        let bytes = fixture(&format!("formati/{name}"));
        for len in [600, bytes.len() / 2] {
            let events = run(name, bytes[..len].to_vec());
            assert_eq!(
                warnings(&events),
                [(Vec::new(), WarningKind::Corrupt)],
                "{name} troncato a {len}"
            );
        }
    }
}

#[test]
fn compound_file_of_another_kind_is_a_mismatch() {
    // Un foglio di Excel chiamato .doc: file composto senza `WordDocument`.
    let events = run("finto.doc", fixture("formati/vecchia-spesa.xls"));
    assert_eq!(events, only_warning(WarningKind::ContentMismatch));
    let events = run("finto.ppt", fixture("formati/vecchia-lettera.doc"));
    assert_eq!(events, only_warning(WarningKind::ContentMismatch));
}

#[test]
fn xls_that_is_html_or_csv() {
    let html = b"<html><body><table><tr><td>Spesa</td><td>aragosta</td></tr></table></body></html>";
    let events = run("esportato.xls", html.to_vec());
    assert_eq!(starts(&events), [(Vec::new(), Format::Html)]);
    assert!(texts(&events).iter().any(|(_, t)| t.contains("aragosta")));
    let events = run("tabella.xls", b"prodotto;prezzo\naragosta;30\n".to_vec());
    assert_eq!(starts(&events), [(Vec::new(), Format::Text)]);
    assert_eq!(
        texts(&events),
        [(
            Location::Line(1),
            "prodotto;prezzo\naragosta;30\n".to_owned()
        )]
    );
    // Un .doc che è HTML non è un caso innocuo.
    assert_eq!(
        run("pagina.doc", html.to_vec()),
        only_warning(WarningKind::ContentMismatch)
    );
}

// ── RTF ───────────────────────────────────────────────────────────────────────────────────

#[test]
fn rtf_too_deep_is_corrupt() {
    let mut rtf = b"{\\rtf1 ".to_vec();
    rtf.extend(std::iter::repeat_n(b'{', 10_000));
    assert_eq!(run("profondo.rtf", rtf), only_warning(WarningKind::Corrupt));
}

#[test]
fn rtf_with_wrong_extension_is_a_mismatch_except_for_doc() {
    let rtf = fixture("formati/lettera.rtf");
    assert_eq!(
        run("lettera.docx", rtf.clone()),
        only_warning(WarningKind::ContentMismatch)
    );
    assert_eq!(
        starts(&run("lettera.doc", rtf)),
        [(Vec::new(), Format::Rtf)]
    );
}

// ── EML ───────────────────────────────────────────────────────────────────────────────────

const HTML_MAIL: &[u8] = b"From: \"Mario\" <mario@example.com>\r\n\
To: a@example.com, Gruppo: b@example.com;\r\n\
Cc: c@example.com\r\n\
Subject: =?utf-8?q?Citt=C3=A0?=\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><p>Prima</p><p>Seconda con l&#39;aragosta</p><script>x()</script></body></html>\r\n";

#[test]
fn eml_html_body_and_addresses() {
    let events = run("html.eml", HTML_MAIL.to_vec());
    let mail = |part, line| Location::Mail { part, line };
    let found = texts(&events);
    assert!(found.contains(&(mail(MailPart::Subject, 1), "Città".to_owned())));
    assert!(found.contains(&(
        mail(MailPart::From, 1),
        "Mario <mario@example.com>".to_owned()
    )));
    assert!(found.contains(&(mail(MailPart::To, 2), "b@example.com".to_owned())));
    assert!(found.contains(&(mail(MailPart::To, 3), "c@example.com".to_owned())));
    assert!(found.iter().any(|(l, t)| matches!(
        l,
        Location::Mail {
            part: MailPart::Body,
            ..
        }
    ) && t.contains("l'aragosta")));
    assert!(!found.iter().any(|(_, t)| t.contains("x()")));
}

/// Un messaggio con un allegato `nome` (base64).
fn mail_with_attachment(name: &str, data: &[u8]) -> Vec<u8> {
    use std::fmt::Write as _;
    let mut encoded = String::new();
    for chunk in data.chunks(57) {
        let _ = writeln!(encoded, "{}", base64(chunk));
    }
    format!(
        "From: a@example.com\r\nSubject: allegato\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"XX\"\r\n\r\n--XX\r\n\
Content-Type: text/plain\r\n\r\ncorpo\r\n--XX\r\n\
Content-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64\r\n\
Content-Disposition: attachment; filename=\"{name}\"\r\n\r\n{encoded}--XX--\r\n"
    )
    .into_bytes()
}

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for k in 0..4 {
            if k <= chunk.len() {
                let index = usize::try_from((n >> (18 - 6 * k)) & 63).unwrap_or(0);
                out.push(char::from(ALPHABET[index]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[test]
fn eml_attachment_names_are_labels_and_archives_nest() {
    let inner_zip = zip_of(&[("dentro.txt", b"aragosta")]);
    let events = run(
        "posta.eml",
        mail_with_attachment("../../fuori\u{202E}.zip", &inner_zip),
    );
    assert_eq!(
        starts(&events),
        [
            (Vec::new(), Format::Eml),
            (
                names(&["../../fuori\u{FFFD}.zip", "dentro.txt"]),
                Format::Text
            ),
        ]
    );
}

#[test]
fn eml_attachments_follow_the_archive_limits() {
    let bytes = mail_with_attachment("nota.txt", b"aragosta");
    // Profondità: un messaggio dentro 0 livelli ammessi non apre gli allegati.
    let limits = ArchiveLimits {
        max_depth: 0,
        ..ArchiveLimits::default()
    };
    let events = run_with("posta.eml", bytes.clone(), limits);
    assert_eq!(starts(&events), [(Vec::new(), Format::Eml)]);
    assert_eq!(
        warnings(&events),
        [(
            Vec::new(),
            WarningKind::LimitExceeded(LimitKind::ArchiveDepth)
        )]
    );
    // Byte per voce: l'allegato decodificato supera il massimo.
    let limits = ArchiveLimits {
        max_entry_size: ByteSize::from_bytes(4),
        ..ArchiveLimits::default()
    };
    let events = run_with(
        "posta.eml",
        mail_with_attachment("nota.txt", b"aragosta"),
        limits,
    );
    assert_eq!(starts(&events), [(Vec::new(), Format::Eml)]);
    assert_eq!(
        warnings(&events),
        [(
            names(&["nota.txt"]),
            WarningKind::LimitExceeded(LimitKind::EntrySize)
        )]
    );
}

#[test]
fn empty_or_garbage_eml_does_not_panic() {
    assert_eq!(
        run("vuoto.eml", Vec::new()),
        only_warning(WarningKind::Corrupt)
    );
    let events = run("rumore.eml", (0..=255_u8).cycle().take(5000).collect());
    assert!(starts(&events).len() <= 1);
}

// ── EPUB ──────────────────────────────────────────────────────────────────────────────────

const CONTAINER: &[u8] = br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
const OPF: &[u8] = br#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><manifest><item id="a" href="a.xhtml" media-type="application/xhtml+xml"/><item id="b" href="b.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="a"/><itemref idref="b"/><itemref idref="a"/></spine></package>"#;

fn epub(chapter_a: &[u8], chapter_b: &[u8]) -> Vec<u8> {
    zip_of(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER),
        ("content.opf", OPF),
        ("a.xhtml", chapter_a),
        ("b.xhtml", chapter_b),
    ])
}

#[test]
fn epub_doctype_entities_and_bad_chapters() {
    let good = br#"<?xml version="1.0"?><!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.1//EN" "http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd"><html xmlns="http://www.w3.org/1999/xhtml"><head><title>T</title><style>p{}</style></head><body><p>Citt&agrave;&nbsp;e&nbsp;mare &amp; l&rsquo;aragosta &ignota;</p><div>uno<br/>due</div></body></html>"#;
    let bomb = br#"<?xml version="1.0"?><!DOCTYPE html [<!ENTITY a "aragosta"><!ENTITY b "&a;&a;&a;">]><html><body><p>&b;</p></body></html>"#;
    let events = run("libro.epub", epub(good, bomb));
    // Il capitolo ripetuto nella spine si legge una volta.
    assert_eq!(starts(&events), [(names(&["a.xhtml"]), Format::Epub)]);
    assert_eq!(
        texts(&events),
        [
            (
                Location::Paragraph(1),
                "Città e mare & l\u{2019}aragosta &ignota;".to_owned()
            ),
            (Location::Paragraph(2), "uno\ndue".to_owned()),
        ]
    );
    assert_eq!(
        warnings(&events),
        [(names(&["b.xhtml"]), WarningKind::Corrupt)]
    );
}

#[test]
fn epub_package_problems() {
    // Senza container.xml: non è un EPUB.
    let bytes = zip_of(&[("a.xhtml", b"<html/>")]);
    assert_eq!(
        run("libro.epub", bytes),
        only_warning(WarningKind::ContentMismatch)
    );
    // mimetype di un altro formato.
    let bytes = zip_of(&[
        ("mimetype", b"application/vnd.oasis.opendocument.text"),
        ("META-INF/container.xml", CONTAINER),
    ]);
    assert_eq!(
        run("libro.epub", bytes),
        only_warning(WarningKind::ContentMismatch)
    );
    // DTD nel pacchetto OPF.
    let bytes = zip_of(&[
        ("META-INF/container.xml", CONTAINER),
        ("content.opf", b"<!DOCTYPE p [<!ENTITY x \"y\">]><package/>"),
    ]);
    assert_eq!(run("libro.epub", bytes), only_warning(WarningKind::Corrupt));
}

// ── tar, gz, 7z ───────────────────────────────────────────────────────────────────────────

#[test]
fn tar_skips_links_and_counts_entries() {
    let bytes = tar_of(&[("a.txt", b"aragosta"), ("b.txt", b"mare")], true);
    let events = run("a.tar", bytes.clone());
    assert_eq!(
        starts(&events),
        [
            (names(&["a.txt"]), Format::Text),
            (names(&["b.txt"]), Format::Text)
        ]
    );
    assert!(events.contains(&ExtractEvent::Skipped {
        inner: names(&["link"]),
        reason: SkipReason::SpecialEntry
    }));
    let limits = ArchiveLimits {
        max_entries: 1,
        ..ArchiveLimits::default()
    };
    let events = run_with("a.tar", bytes, limits);
    assert_eq!(starts(&events), [(names(&["a.txt"]), Format::Text)]);
    assert_eq!(
        warnings(&events),
        [(
            Vec::new(),
            WarningKind::LimitExceeded(LimitKind::ArchiveEntries)
        )]
    );
}

#[test]
fn truncated_tar_and_gz() {
    let tar = tar_of(&[("a.txt", &[b'x'; 3000])], false);
    let events = run("a.tar", tar[..1500].to_vec());
    assert!(
        warnings(&events)
            .iter()
            .all(|(_, kind)| *kind == WarningKind::Corrupt),
        "{events:?}"
    );
    assert!(!warnings(&events).is_empty());
    let gz = gzip(b"riga uno\nriga due aragosta\n");
    let events = run("a.txt.gz", gz[..gz.len() - 6].to_vec());
    assert!(
        warnings(&events)
            .iter()
            .any(|(_, kind)| *kind == WarningKind::Corrupt),
        "{events:?}"
    );
    assert_eq!(
        run("rotto.gz", b"\x1F\x8Bnon gzip".to_vec()),
        only_warning(WarningKind::Corrupt)
    );
}

#[test]
fn gz_bomb_hits_ratio_limit() {
    let bomb = gzip(&vec![b'a'; 5 * 1024 * 1024]);
    let events = run("zeri.txt.gz", bomb.clone());
    assert_eq!(
        warnings(&events),
        [(
            names(&["zeri.txt"]),
            WarningKind::LimitExceeded(LimitKind::CompressionRatio)
        )]
    );
    // Lo stesso con un tar dentro: l'avviso è dell'archivio.
    let tar = tar_of(&[("zeri.txt", &vec![b'a'; 5 * 1024 * 1024])], false);
    let events = run("zeri.tar.gz", gzip(&tar));
    assert_eq!(
        warnings(&events).last().map(|(_, kind)| *kind),
        Some(WarningKind::LimitExceeded(LimitKind::CompressionRatio))
    );
}

#[test]
fn tgz_and_gz_with_tar_content() {
    let tar = tar_of(&[("a.txt", b"aragosta")], false);
    for name in ["x.tgz", "x.tar.gz", "senza-nome.gz"] {
        let events = run(name, gzip(&tar));
        assert_eq!(
            starts(&events),
            [(names(&["a.txt"]), Format::Text)],
            "{name}"
        );
    }
}

#[test]
fn archives_nest_inside_zip_with_depth_limit() {
    let tar = tar_of(&[("a.txt", b"aragosta")], false);
    let sevenz = fixture("formati/archivio.7z");
    let outer = zip_of(&[("dentro.tar.gz", &gzip(&tar)), ("dentro.7z", &sevenz)]);
    let events = run("fuori.zip", outer.clone());
    let documents: Vec<Vec<String>> = starts(&events).into_iter().map(|(i, _)| i).collect();
    assert!(documents.contains(&names(&["dentro.tar.gz", "a.txt"])));
    assert!(documents.contains(&names(&["dentro.7z", "cartella/ricetta.txt"])));
    let limits = ArchiveLimits {
        max_depth: 1,
        ..ArchiveLimits::default()
    };
    let events = run_with("fuori.zip", outer, limits);
    assert!(starts(&events).is_empty());
    assert_eq!(
        warnings(&events),
        [
            (
                names(&["dentro.tar.gz"]),
                WarningKind::LimitExceeded(LimitKind::ArchiveDepth)
            ),
            (
                names(&["dentro.7z"]),
                WarningKind::LimitExceeded(LimitKind::ArchiveDepth)
            ),
        ]
    );
}

#[test]
fn sevenz_limits_and_damage() {
    let bytes = fixture("formati/archivio.7z");
    let limits = ArchiveLimits {
        max_entries: 1,
        ..ArchiveLimits::default()
    };
    assert_eq!(
        run_with("a.7z", bytes.clone(), limits),
        only_warning(WarningKind::LimitExceeded(LimitKind::ArchiveEntries))
    );
    let limits = ArchiveLimits {
        max_entry_size: ByteSize::from_bytes(10),
        ..ArchiveLimits::default()
    };
    let events = run_with("a.7z", bytes.clone(), limits);
    assert!(starts(&events).is_empty());
    assert!(
        warnings(&events)
            .iter()
            .all(|(_, kind)| *kind == WarningKind::LimitExceeded(LimitKind::EntrySize)),
        "{events:?}"
    );
    for len in [10, 40, bytes.len() / 2, bytes.len() - 1] {
        let events = run("a.7z", bytes[..len].to_vec());
        assert!(
            warnings(&events)
                .iter()
                .any(|(_, kind)| *kind == WarningKind::Corrupt),
            "troncato a {len}: {events:?}"
        );
    }
    // Un byte cambiato nei dati compressi.
    let mut flipped = bytes;
    if let Some(byte) = flipped.get_mut(40) {
        *byte ^= 0xFF;
    }
    let events = run("a.7z", flipped);
    assert!(!warnings(&events).is_empty(), "{events:?}");
}
