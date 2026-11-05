// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Archivi costruiti dal test: limiti abbassati, voci cifrate e speciali, contenuto che
//! mente dentro un archivio, arresto chiesto dal destinatario. Le fixture ostili che
//! sarebbero grandi o dipendenti dal sistema (molte voci, link simbolici) si generano qui
//! invece di stare in `fixtures/malevoli/`.

use std::io::{Cursor, Write};
use std::path::Path;

use lynkeus_core::ByteSize;
use lynkeus_extract::{
    ArchiveLimits, EventSink, ExtractEvent, ExtractOptions, Format, LimitKind, SequenceCheck,
    SkipReason, Stopped, WarningKind, extract_bytes,
};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

/// Uno zip con le voci `(nome, contenuto)`.
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

fn run(name: &str, bytes: Vec<u8>, limits: ArchiveLimits) -> Vec<ExtractEvent> {
    let mut events = Vec::new();
    let options = ExtractOptions { limits };
    assert_eq!(extract_bytes(name, bytes, &options, &mut events), Ok(()));
    let mut check = SequenceCheck::new();
    for event in &events {
        assert_eq!(check.accept(event), Ok(()), "{events:?}");
    }
    assert_eq!(check.finish(), Ok(()));
    events
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

fn documents(events: &[ExtractEvent]) -> Vec<Vec<String>> {
    events
        .iter()
        .filter_map(|e| match e {
            ExtractEvent::DocumentStart { inner, .. } => Some(inner.clone()),
            _ => None,
        })
        .collect()
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn total_size_counts_all_levels() {
    // Ogni livello sta sotto il massimo per voce, ma la somma supera il totale.
    let text = vec![b'a'; 3000];
    let level3 = zip_of(&[("c.txt", &text)]);
    let level2 = zip_of(&[("b.txt", &text), ("l3.zip", &level3)]);
    let level1 = zip_of(&[("a.txt", &text), ("l2.zip", &level2)]);
    let limits = ArchiveLimits {
        max_entry_size: ByteSize::from_bytes(10_000),
        max_archive_size: ByteSize::from_bytes(7_000),
        ..ArchiveLimits::default()
    };
    let events = run("radice.zip", level1, limits);
    let hit: Vec<_> = warnings(&events)
        .into_iter()
        .filter(|(_, kind)| *kind == WarningKind::LimitExceeded(LimitKind::ArchiveSize))
        .collect();
    assert_eq!(hit.len(), 1, "{:?}", warnings(&events));
    // Con il totale predefinito tutto si legge.
    let events = run(
        "radice.zip",
        zip_of(&[("a.txt", &text), ("l2.zip", &zip_of(&[("b.txt", &text)]))]),
        ArchiveLimits::default(),
    );
    assert_eq!(documents(&events).len(), 2);
}

#[test]
fn too_many_entries() {
    let bytes = zip_of(&[("a.txt", b"uno"), ("b.txt", b"due"), ("c.txt", b"tre")]);
    let limits = ArchiveLimits {
        max_entries: 2,
        ..ArchiveLimits::default()
    };
    let events = run("tante.zip", bytes.clone(), limits);
    assert_eq!(
        events,
        [ExtractEvent::Warning {
            inner: Vec::new(),
            kind: WarningKind::LimitExceeded(LimitKind::ArchiveEntries)
        }]
    );
    let limits = ArchiveLimits {
        max_entries: 3,
        ..ArchiveLimits::default()
    };
    assert_eq!(documents(&run("tante.zip", bytes, limits)).len(), 3);
}

/// `malevoli/molte-voci.zip` del threat model, generato al volo: 100.001 voci vuote (circa
/// 9 MB) non stanno nel repository.
#[test]
fn many_entries_zip_hits_the_default_limit() {
    let over = ArchiveLimits::default().max_entries.saturating_add(1);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..over {
        assert!(
            writer
                .start_file(format!("{i}.txt"), SimpleFileOptions::default())
                .is_ok()
        );
    }
    let bytes = writer.finish().map(Cursor::into_inner).unwrap_or_default();
    let events = run("molte-voci.zip", bytes, ArchiveLimits::default());
    // L'elenco si conta prima di decomprimere: nessuna voce letta.
    assert_eq!(
        events,
        [ExtractEvent::Warning {
            inner: Vec::new(),
            kind: WarningKind::LimitExceeded(LimitKind::ArchiveEntries)
        }]
    );
}

#[test]
fn entry_size_limit_is_counted_on_the_stream() {
    let bytes = zip_of(&[("grande.txt", &[b'x'; 5000]), ("piccolo.txt", b"aragosta")]);
    let limits = ArchiveLimits {
        max_entry_size: ByteSize::from_bytes(4096),
        ..ArchiveLimits::default()
    };
    let events = run("a.zip", bytes, limits);
    assert_eq!(
        warnings(&events),
        [(
            names(&["grande.txt"]),
            WarningKind::LimitExceeded(LimitKind::EntrySize)
        )]
    );
    assert_eq!(documents(&events), [names(&["piccolo.txt"])]);
}

/// Accende il bit "cifrato" della prima voce, nell'intestazione locale e in quella centrale.
fn mark_first_entry_encrypted(bytes: &mut [u8]) {
    let local = memchr::memmem::find(bytes, b"PK\x03\x04").unwrap_or(usize::MAX);
    let central = memchr::memmem::find(bytes, b"PK\x01\x02").unwrap_or(usize::MAX);
    if let Some(flag) = bytes.get_mut(local.wrapping_add(6)) {
        *flag |= 1;
    }
    if let Some(flag) = bytes.get_mut(central.wrapping_add(8)) {
        *flag |= 1;
    }
}

#[test]
fn encrypted_entry_is_a_warning_and_others_continue() {
    let mut bytes = zip_of(&[("segreto.txt", b"aragosta"), ("libero.txt", b"aragosta")]);
    mark_first_entry_encrypted(&mut bytes);
    let events = run("misto.zip", bytes, ArchiveLimits::default());
    assert_eq!(
        warnings(&events),
        [(names(&["segreto.txt"]), WarningKind::Encrypted)]
    );
    assert_eq!(documents(&events), [names(&["libero.txt"])]);
}

#[test]
fn symlink_entry_is_skipped() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    assert!(
        writer
            .add_symlink("link", "/etc/passwd", SimpleFileOptions::default())
            .is_ok()
    );
    let bytes = writer.finish().map(Cursor::into_inner).unwrap_or_default();
    let events = run("link.zip", bytes, ArchiveLimits::default());
    assert_eq!(
        events,
        [ExtractEvent::Skipped {
            inner: names(&["link"]),
            reason: SkipReason::SpecialEntry
        }]
    );
}

#[test]
fn lying_extension_inside_archive() {
    let png = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/malevoli/immagine-finta.pdf"),
    )
    .unwrap_or_default();
    assert!(!png.is_empty());
    let bytes = zip_of(&[("finta.pdf", &png), ("vera.txt", b"aragosta")]);
    let events = run("a.zip", bytes, ArchiveLimits::default());
    assert_eq!(
        warnings(&events),
        [(names(&["finta.pdf"]), WarningKind::ContentMismatch)]
    );
    assert_eq!(documents(&events), [names(&["vera.txt"])]);
}

/// Un tar ostile: link simbolici verso file di sistema e fuori dall'archivio, link fisico,
/// FIFO e una voce con estensione che mente. I link non si seguono e non creano file; la
/// voce che mente dà l'avviso; la voce regolare si legge.
#[test]
fn tar_with_links_special_entries_and_a_lying_extension() {
    let png = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/malevoli/immagine-finta.pdf"),
    )
    .unwrap_or_default();
    assert!(!png.is_empty());
    let mut builder = tar::Builder::new(Vec::new());
    for (name, target, kind) in [
        ("passwd.txt", "/etc/passwd", tar::EntryType::Symlink),
        ("fuori.txt", "../../fuori.txt", tar::EntryType::Symlink),
        ("windows.txt", "C:/Windows/win.ini", tar::EntryType::Symlink),
        ("duro.txt", "vera.txt", tar::EntryType::Link),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_size(0);
        assert!(builder.append_link(&mut header, name, target).is_ok());
    }
    let mut fifo = tar::Header::new_gnu();
    fifo.set_entry_type(tar::EntryType::Fifo);
    fifo.set_size(0);
    fifo.set_cksum();
    assert!(builder.append_data(&mut fifo, "tubo.txt", &[][..]).is_ok());
    for (name, data) in [("finta.pdf", png.as_slice()), ("vera.txt", b"aragosta")] {
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(data.len()).unwrap_or(0));
        header.set_mode(0o644);
        header.set_cksum();
        assert!(builder.append_data(&mut header, name, data).is_ok());
    }
    let bytes = builder.into_inner().unwrap_or_default();
    let events = run("ostile.tar", bytes, ArchiveLimits::default());
    let skipped: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ExtractEvent::Skipped { inner, reason } => Some((inner.clone(), *reason)),
            _ => None,
        })
        .collect();
    let special = |name: &str| (names(&[name]), SkipReason::SpecialEntry);
    assert_eq!(
        skipped,
        [
            special("passwd.txt"),
            special("fuori.txt"),
            special("windows.txt"),
            special("duro.txt"),
            special("tubo.txt"),
        ]
    );
    assert_eq!(
        warnings(&events),
        [(names(&["finta.pdf"]), WarningKind::ContentMismatch)]
    );
    assert_eq!(documents(&events), [names(&["vera.txt"])]);
}

#[test]
fn control_characters_in_names_are_neutralized() {
    let bytes = zip_of(&[("fattura\u{202E}fdp.txt", b"aragosta")]);
    let events = run("a.zip", bytes, ArchiveLimits::default());
    assert_eq!(documents(&events), [names(&["fattura\u{FFFD}fdp.txt"])]);
}

#[test]
fn corrupt_entry_does_not_stop_the_others() {
    let bytes = zip_of(&[
        ("rotto.docx", b"PK\x03\x04 non uno zip"),
        ("buono.txt", b"aragosta"),
    ]);
    let events = run("a.zip", bytes, ArchiveLimits::default());
    assert_eq!(
        warnings(&events),
        [(names(&["rotto.docx"]), WarningKind::Corrupt)]
    );
    assert_eq!(documents(&events), [names(&["buono.txt"])]);
}

/// Un destinatario che si ferma dopo `left` eventi.
struct StopAfter {
    left: usize,
    seen: Vec<ExtractEvent>,
}

impl EventSink for StopAfter {
    fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped> {
        if self.left == 0 {
            return Err(Stopped);
        }
        self.left -= 1;
        self.seen.push(event);
        Ok(())
    }
}

#[test]
fn sink_can_stop_the_extraction() {
    let entries: Vec<(String, Vec<u8>)> = (0..10)
        .map(|i| (format!("{i}.txt"), b"aragosta".to_vec()))
        .collect();
    let refs: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let mut sink = StopAfter {
        left: 4,
        seen: Vec::new(),
    };
    let result = extract_bytes(
        "molti.zip",
        zip_of(&refs),
        &ExtractOptions::default(),
        &mut sink,
    );
    assert_eq!(result, Err(Stopped));
    assert_eq!(sink.seen.len(), 4);
    assert!(matches!(
        sink.seen.first(),
        Some(ExtractEvent::DocumentStart {
            format: Format::Text,
            ..
        })
    ));
}

fn fixture(relative: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(relative);
    let bytes = std::fs::read(&path).unwrap_or_default();
    assert!(!bytes.is_empty(), "fixture mancante: {}", path.display());
    bytes
}

/// Uno zip scritto a mano con la sola voce `name`: `data` compresso con deflate, seguito da
/// zeri fino a `declared` byte, che è la dimensione compressa dichiarata. Il flusso deflate
/// finisce prima: chi decomprime non legge il riempimento, ma il rapporto calcolato sul
/// dichiarato scende.
fn padded_zip(name: &str, data: &[u8], declared: u32) -> Vec<u8> {
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    assert!(encoder.write_all(data).is_ok());
    let mut body = encoder.finish().unwrap_or_default();
    let declared_len = usize::try_from(declared).unwrap_or(usize::MAX);
    assert!(body.len() < declared_len);
    body.resize(declared_len, 0);
    let mut crc = flate2::Crc::new();
    crc.update(data);
    let size = u32::try_from(data.len()).unwrap_or(u32::MAX);
    let name_len = u16::try_from(name.len()).unwrap_or(u16::MAX);
    // Versione 2.0, nessuna opzione, deflate, ora 00:00 del 2026-01-01.
    let common = |out: &mut Vec<u8>| {
        for value in [20_u16, 0, 8, 0, ((2026 - 1980) << 9) | (1 << 5) | 1] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in [crc.sum(), declared, size] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
    };
    let mut zip = 0x0403_4B50_u32.to_le_bytes().to_vec();
    common(&mut zip);
    zip.extend_from_slice(name.as_bytes());
    zip.extend_from_slice(&body);
    let central_at = u32::try_from(zip.len()).unwrap_or(u32::MAX);
    let mut central = 0x0201_4B50_u32.to_le_bytes().to_vec();
    central.extend_from_slice(&20_u16.to_le_bytes());
    common(&mut central);
    // Commento, disco, attributi interni ed esterni, posizione dell'intestazione locale.
    central.extend_from_slice(&[0; 2 + 2 + 2 + 4 + 4]);
    central.extend_from_slice(name.as_bytes());
    let central_len = u32::try_from(central.len()).unwrap_or(u32::MAX);
    zip.extend_from_slice(&central);
    zip.extend_from_slice(&0x0605_4B50_u32.to_le_bytes());
    for value in [0_u16, 0, 1, 1] {
        zip.extend_from_slice(&value.to_le_bytes());
    }
    zip.extend_from_slice(&central_len.to_le_bytes());
    zip.extend_from_slice(&central_at.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip
}

/// 20 MiB di testo compressi in circa 20 KB, con la dimensione compressa dichiarata gonfiata
/// a 150 KB da zeri dopo la fine del flusso deflate: sul dichiarato il rapporto (circa
/// 140:1) è sotto 200:1, sui byte davvero letti dal contenitore no.
#[test]
fn padded_zip_hits_ratio_limit() {
    let data = vec![b'a'; 20 * 1024 * 1024];
    let zip = padded_zip("riempito.txt", &data, 150 * 1024);
    let events = run("imbottito.zip", zip, ArchiveLimits::default());
    assert_eq!(
        warnings(&events),
        [(
            names(&["riempito.txt"]),
            WarningKind::LimitExceeded(LimitKind::CompressionRatio)
        )]
    );
    // Lo stesso zip senza riempimento ha già il rapporto oltre il limite dichiarato.
    let honest = padded_zip("riempito.txt", &data, 30 * 1024);
    let events = run("onesto.zip", honest, ArchiveLimits::default());
    assert_eq!(
        warnings(&events),
        [(
            names(&["riempito.txt"]),
            WarningKind::LimitExceeded(LimitKind::CompressionRatio)
        )]
    );
}

/// `data` compresso con gzip.
fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    assert!(encoder.write_all(data).is_ok());
    encoder.finish().unwrap_or_default()
}

/// Lo stesso tar compresso con gzip, chiamato "x.gz" (tar riconosciuto dal contenuto) o
/// "x.tar.gz": stessi eventi anche con limiti piccoli. Il massimo per voce vale per ogni voce
/// e non per l'intero tar, e i byte si contano una volta sola.
#[test]
fn gz_with_tar_content_has_tar_gz_limits() {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, fill) in [("a.txt", b'a'), ("b.txt", b'b'), ("c.txt", b'c')] {
        let data = vec![fill; 3000];
        let mut header = tar::Header::new_gnu();
        header.set_size(3000);
        header.set_mode(0o644);
        header.set_cksum();
        assert!(
            builder
                .append_data(&mut header, name, data.as_slice())
                .is_ok()
        );
    }
    let tar = builder.into_inner().unwrap_or_default();
    // Il tar (circa 11 KB) supera il massimo per voce; le voci (9000 byte) stanno nel totale,
    // contate due volte no.
    assert!(tar.len() > 10_000);
    let limits = ArchiveLimits {
        max_entry_size: ByteSize::from_bytes(4000),
        max_archive_size: ByteSize::from_bytes(15_000),
        ..ArchiveLimits::default()
    };
    let compressed = gzip(&tar);
    let as_gz = run("x.gz", compressed.clone(), limits);
    let as_tar_gz = run("x.tar.gz", compressed, limits);
    assert_eq!(as_gz, as_tar_gz);
    assert!(warnings(&as_gz).is_empty(), "{as_gz:?}");
    assert_eq!(
        documents(&as_gz),
        [names(&["a.txt"]), names(&["b.txt"]), names(&["c.txt"])]
    );
}

/// Un EPUB è un contenitore come gli altri: dentro uno zip con profondità massima 1 non si
/// apre, e i suoi capitoli contano tra le voci del file radice.
#[test]
fn epub_respects_depth_and_entry_budget() {
    let book = fixture("formati/libro.epub");
    let shallow = ArchiveLimits {
        max_depth: 1,
        ..ArchiveLimits::default()
    };
    let events = run("esterno.zip", zip_of(&[("libro.epub", &book)]), shallow);
    assert_eq!(
        warnings(&events),
        [(
            names(&["libro.epub"]),
            WarningKind::LimitExceeded(LimitKind::ArchiveDepth)
        )]
    );
    assert!(documents(&events).is_empty(), "{events:?}");
    // Tre voci nello zip più due capitoli per libro (il libro ha 6 parti, entro il massimo
    // di 7 per contenitore): 3 + 2 + 2 = 7, il terzo libro è oltre.
    let few = ArchiveLimits {
        max_entries: 7,
        ..ArchiveLimits::default()
    };
    let books = zip_of(&[
        ("uno.epub", &book),
        ("due.epub", &book),
        ("tre.epub", &book),
    ]);
    let events = run("libri.zip", books, few);
    let chapters = |book: &str| {
        [
            names(&[book, "OEBPS/cap1.xhtml"]),
            names(&[book, "OEBPS/cap2.xhtml"]),
        ]
    };
    assert_eq!(
        documents(&events),
        [chapters("uno.epub"), chapters("due.epub")].concat()
    );
    assert_eq!(
        warnings(&events),
        [(
            names(&["tre.epub"]),
            WarningKind::LimitExceeded(LimitKind::ArchiveEntries)
        )]
    );
}
