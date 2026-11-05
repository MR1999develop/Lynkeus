// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Estrazione dei byte di una voce d'archivio (`extract_entry`): byte identici all'originale,
//! voci annidate, voci assenti, limiti, pezzi entro il massimo.

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use lynkeus_extract::{
    ENTRY_CHUNK_BYTES, EntryOutcome, EntrySink, ExtractOptions, LimitKind, Stopped, WarningKind,
    extract_entry,
};

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(relative)
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// I byte della voce `inner` letti direttamente con la libreria zip, livello per livello.
fn zip_entry(path: &Path, inner: &[&str]) -> Vec<u8> {
    let mut bytes = std::fs::read(path).expect("fixture");
    for name in inner {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
        let mut entry = archive.by_name(name).expect("voce");
        let mut out = Vec::new();
        entry.read_to_end(&mut out).expect("lettura");
        bytes = out;
    }
    bytes
}

/// Raccoglie i pezzi e ne ricorda la dimensione massima.
#[derive(Default)]
struct Chunks {
    bytes: Vec<u8>,
    largest: usize,
}

impl EntrySink for Chunks {
    fn bytes(&mut self, chunk: &[u8]) -> Result<(), Stopped> {
        assert!(!chunk.is_empty());
        self.largest = self.largest.max(chunk.len());
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }
}

fn extract(path: &Path, inner: &[&str], max_bytes: u64) -> (EntryOutcome, Chunks) {
    let mut sink = Chunks::default();
    let outcome = extract_entry(
        path,
        &names(inner),
        max_bytes,
        &ExtractOptions::default(),
        &mut sink,
    )
    .expect("nessun arresto");
    (outcome, sink)
}

#[test]
fn docx_three_levels_down_is_byte_identical() {
    let path = fixture("formati/annidato.zip");
    let inner = ["livello2.zip", "livello3.zip", "interno.docx"];
    let expected = zip_entry(&path, &inner);
    let (outcome, got) = extract(&path, &inner, u64::MAX);
    assert_eq!(
        outcome,
        EntryOutcome::Extracted {
            bytes: u64::try_from(expected.len()).unwrap_or(0)
        }
    );
    assert_eq!(got.bytes, expected);
    assert!(got.largest <= ENTRY_CHUNK_BYTES);
}

#[test]
fn pdf_in_the_demo_archive_is_byte_identical() {
    let path = fixture("demo/musica/archivio.zip");
    let expected = zip_entry(&path, &["elenco.pdf"]);
    let (outcome, got) = extract(&path, &["elenco.pdf"], u64::MAX);
    assert!(matches!(outcome, EntryOutcome::Extracted { .. }));
    assert_eq!(got.bytes, expected);
}

#[test]
fn entry_partitions() {
    let path = fixture("formati/annidato.zip");
    // Voce assente, percorso che si ferma a un archivio, file radice, oltre il massimo.
    assert_eq!(
        extract(&path, &["livello2.zip", "manca.docx"], u64::MAX).0,
        EntryOutcome::Failed(WarningKind::NotFound)
    );
    assert_eq!(
        extract(&path, &[], u64::MAX).0,
        EntryOutcome::Failed(WarningKind::NotFound)
    );
    assert_eq!(
        extract(
            &path,
            &["livello2.zip", "livello3.zip", "interno.docx"],
            100
        )
        .0,
        EntryOutcome::Failed(WarningKind::LimitExceeded(LimitKind::EntrySize))
    );
    // File inesistente: il motivo dell'avviso del file.
    assert_eq!(
        extract(&fixture("non-esiste.zip"), &["a.txt"], u64::MAX).0,
        EntryOutcome::Failed(WarningKind::NotFound)
    );
}

#[test]
fn every_entry_of_the_other_archives() {
    for archive in [
        "formati/archivio.tar",
        "formati/archivio.7z",
        "formati/archivio.tar.gz",
        "formati/appunti.txt.gz",
        "formati/ordine.eml",
    ] {
        let path = fixture(archive);
        let mut events = Vec::new();
        assert_eq!(
            lynkeus_extract::extract_path(&path, &ExtractOptions::default(), &mut events),
            Ok(())
        );
        // Ogni documento letto nella ricerca si può estrarre con il suo percorso.
        let documents: Vec<Vec<String>> = events
            .iter()
            .filter_map(|event| match event {
                lynkeus_extract::ExtractEvent::DocumentStart { inner, .. } if !inner.is_empty() => {
                    Some(inner.clone())
                }
                _ => None,
            })
            .collect();
        assert!(!documents.is_empty(), "{archive}");
        for inner in documents {
            let mut sink = Chunks::default();
            let outcome = extract_entry(
                &path,
                &inner,
                u64::MAX,
                &ExtractOptions::default(),
                &mut sink,
            );
            assert!(
                matches!(outcome, Ok(EntryOutcome::Extracted { .. })),
                "{archive} {inner:?}: {outcome:?}"
            );
        }
    }
}

/// Un destinatario che si ferma: l'estrazione restituisce `Stopped`.
struct Refuse;

impl EntrySink for Refuse {
    fn bytes(&mut self, _chunk: &[u8]) -> Result<(), Stopped> {
        Err(Stopped)
    }
}

#[test]
fn sink_can_stop() {
    let result = extract_entry(
        &fixture("demo/musica/archivio.zip"),
        &names(&["elenco.pdf"]),
        u64::MAX,
        &ExtractOptions::default(),
        &mut Refuse,
    );
    assert_eq!(result, Err(Stopped));
}
