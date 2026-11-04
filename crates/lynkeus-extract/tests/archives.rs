// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Archivi costruiti dal test: limiti abbassati, voci cifrate e speciali, contenuto che
//! mente dentro un archivio, arresto chiesto dal destinatario.

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
