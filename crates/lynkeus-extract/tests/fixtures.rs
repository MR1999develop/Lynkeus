// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Estrazione nello stesso processo sulle fixture di `fixtures/formati` e `fixtures/malevoli`
//! (descritte in `fixtures/README.md`): posizione attesa del testo, avviso giusto, sequenza
//! degli eventi valida, nessun panic, tempo limitato.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lynkeus_core::{Location, TextUnit};
use lynkeus_extract::{
    ExtractEvent, ExtractOptions, Format, LimitKind, SequenceCheck, SkipReason, WarningKind,
    extract_path, line_location,
};

/// Tempo massimo per una fixture (in debug, su una macchina lenta): le fixture sono piccole,
/// un tempo maggiore vuol dire un ciclo o una bomba non fermata.
const MAX_TIME: Duration = Duration::from_secs(20);

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(relative)
}

/// Estrae una fixture controllando sequenza e tempo.
fn extract(relative: &str) -> Vec<ExtractEvent> {
    let path = fixture(relative);
    assert!(path.is_file(), "fixture mancante: {}", path.display());
    let start = Instant::now();
    let mut events = Vec::new();
    let result = extract_path(&path, &ExtractOptions::default(), &mut events);
    let elapsed = start.elapsed();
    assert_eq!(result, Ok(()));
    assert!(elapsed < MAX_TIME, "{relative}: {elapsed:?}");
    let mut check = SequenceCheck::new();
    for event in &events {
        assert_eq!(check.accept(event), Ok(()), "{relative}: {events:?}");
    }
    assert_eq!(check.finish(), Ok(()), "{relative}");
    events
}

/// Posizioni delle righe che contengono `word`, con la riga dentro l'unità applicata.
fn locations_of(events: &[ExtractEvent], word: &str) -> Vec<Location> {
    let mut found = Vec::new();
    for event in events {
        if let ExtractEvent::Unit(TextUnit { location, text }) = event {
            for (k, line) in text.split('\n').enumerate() {
                if line.contains(word) {
                    let k = u32::try_from(k).unwrap_or(u32::MAX);
                    found.push(line_location(location, k));
                }
            }
        }
    }
    found
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

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

// ── formati/ ──────────────────────────────────────────────────────────────────────────────

#[test]
fn docx_paragraphs() {
    let events = extract("formati/relazione.docx");
    assert_eq!(starts(&events), [(Vec::new(), Format::Docx)]);
    assert_eq!(locations_of(&events, "aragosta"), [Location::Paragraph(4)]);
    assert_eq!(locations_of(&events, "città"), [Location::Paragraph(2)]);
    assert!(warnings(&events).is_empty());
}

#[test]
fn xlsx_cells() {
    let events = extract("formati/spesa.xlsx");
    assert_eq!(starts(&events), [(Vec::new(), Format::Xlsx)]);
    let spesa = |row, col| Location::Cell {
        sheet: "Spesa".to_owned(),
        row,
        col,
    };
    assert_eq!(locations_of(&events, "aragosta"), [spesa(7, 2)]);
    assert_eq!(locations_of(&events, "città"), [spesa(2, 3)]);
}

#[test]
fn ods_cells() {
    let events = extract("formati/bilancio.ods");
    assert_eq!(starts(&events), [(Vec::new(), Format::Ods)]);
    let conti = |row, col| Location::Cell {
        sheet: "Conti".to_owned(),
        row,
        col,
    };
    assert_eq!(locations_of(&events, "aragosta"), [conti(4, 3)]);
    assert_eq!(locations_of(&events, "città"), [conti(3, 3)]);
}

#[test]
fn pptx_slides() {
    let events = extract("formati/presentazione.pptx");
    assert_eq!(starts(&events), [(Vec::new(), Format::Pptx)]);
    assert_eq!(locations_of(&events, "aragosta"), [Location::Slide(2)]);
    assert_eq!(locations_of(&events, "città"), [Location::Slide(3)]);
}

#[test]
fn odt_paragraphs() {
    let events = extract("formati/appunti.odt");
    assert_eq!(starts(&events), [(Vec::new(), Format::Odt)]);
    assert_eq!(locations_of(&events, "aragosta"), [Location::Paragraph(2)]);
    assert_eq!(locations_of(&events, "città"), [Location::Paragraph(3)]);
    assert_eq!(locations_of(&events, "Perché"), [Location::Paragraph(3)]);
}

#[test]
fn odp_slides() {
    let events = extract("formati/lezione.odp");
    assert_eq!(starts(&events), [(Vec::new(), Format::Odp)]);
    assert_eq!(locations_of(&events, "aragosta"), [Location::Slide(2)]);
    assert_eq!(locations_of(&events, "città"), [Location::Slide(1)]);
}

#[test]
fn pdf_page_lines() {
    let events = extract("formati/catalogo.pdf");
    assert_eq!(starts(&events), [(Vec::new(), Format::Pdf)]);
    assert_eq!(
        locations_of(&events, "aragosta"),
        [Location::PageLine { page: 2, line: 3 }]
    );
    assert_eq!(
        locations_of(&events, "città"),
        [Location::PageLine { page: 2, line: 2 }]
    );
    assert!(warnings(&events).is_empty());
}

#[test]
fn nested_zip_three_levels() {
    let events = extract("formati/annidato.zip");
    let inner = names(&["livello2.zip", "livello3.zip", "interno.docx"]);
    assert_eq!(starts(&events), [(inner, Format::Docx)]);
    assert_eq!(locations_of(&events, "aragosta"), [Location::Paragraph(2)]);
    assert!(warnings(&events).is_empty());
}

#[test]
fn pdf_inside_zip() {
    let events = extract("demo/musica/archivio.zip");
    assert_eq!(starts(&events), [(names(&["elenco.pdf"]), Format::Pdf)]);
    assert_eq!(
        locations_of(&events, "chitarra"),
        [Location::PageLine { page: 1, line: 2 }]
    );
}

#[test]
fn password_protected_docx() {
    let events = extract("formati/protetto-password.docx");
    assert_eq!(
        events,
        [ExtractEvent::Warning {
            inner: Vec::new(),
            kind: WarningKind::Encrypted
        }]
    );
}

#[test]
fn password_protected_pdf() {
    let events = extract("formati/protetto-password.pdf");
    assert_eq!(
        events,
        [ExtractEvent::Warning {
            inner: Vec::new(),
            kind: WarningKind::Encrypted
        }]
    );
}

#[test]
fn plain_text_line_numbers() {
    let events = extract("formati/spesa.txt");
    assert_eq!(locations_of(&events, "aragosta"), [Location::Line(6)]);
}

// ── malevoli/ ─────────────────────────────────────────────────────────────────────────────

#[test]
fn zip_bomb_hits_ratio_limit() {
    let events = extract("malevoli/bomba.zip");
    assert_eq!(
        warnings(&events),
        [(
            names(&["zeri.txt"]),
            WarningKind::LimitExceeded(LimitKind::CompressionRatio)
        )]
    );
    assert!(starts(&events).is_empty());
}

#[test]
fn entry_names_are_only_labels() {
    let before = listing(&fixture("malevoli"));
    let events = extract("malevoli/percorsi.zip");
    assert_eq!(listing(&fixture("malevoli")), before);
    let documents: Vec<Vec<String>> = starts(&events)
        .into_iter()
        .map(|(inner, _)| inner)
        .collect();
    assert_eq!(
        documents,
        [
            names(&["../../evil.txt"]),
            names(&["/assoluto/evil.txt"]),
            names(&["C:/Windows/evil.txt"]),
            names(&["ok/normale.txt"]),
        ]
    );
    assert_eq!(locations_of(&events, "aragosta").len(), 4);
}

/// Nomi dei file di una cartella e delle sue sottocartelle.
fn listing(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        if let Ok(entries) = std::fs::read_dir(&next) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path.clone());
                }
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn deep_nesting_stops_at_depth_limit() {
    let events = extract("malevoli/annidato-profondo.zip");
    let archive = names(&[
        "livello2.zip",
        "livello3.zip",
        "livello4.zip",
        "livello5.zip",
        "livello6.zip",
    ]);
    assert_eq!(
        warnings(&events),
        [(archive, WarningKind::LimitExceeded(LimitKind::ArchiveDepth))]
    );
    assert!(
        starts(&events).is_empty(),
        "fondo.txt raggiunto: {events:?}"
    );
}

#[test]
fn truncated_pdf_is_corrupt() {
    let events = extract("malevoli/troncato.pdf");
    assert_eq!(
        events,
        [ExtractEvent::Warning {
            inner: Vec::new(),
            kind: WarningKind::Corrupt
        }]
    );
}

#[test]
fn doctype_in_docx_is_corrupt() {
    let events = extract("malevoli/entita.docx");
    assert_eq!(warnings(&events), [(Vec::new(), WarningKind::Corrupt)]);
    // Nessuna espansione: al più il testo letto prima del DTD (nessuno, il DTD è in testa).
    assert!(locations_of(&events, "aragosta").is_empty());
}

#[test]
fn fake_pdf_is_content_mismatch() {
    let events = extract("malevoli/immagine-finta.pdf");
    assert_eq!(
        events,
        [ExtractEvent::Warning {
            inner: Vec::new(),
            kind: WarningKind::ContentMismatch
        }]
    );
}

#[test]
fn long_line_text_is_read_in_bounded_units() {
    let events = extract("malevoli/riga-lunga.txt");
    assert_eq!(locations_of(&events, "aragosta"), [Location::Line(2)]);
    let max_unit = events
        .iter()
        .filter_map(|e| match e {
            ExtractEvent::Unit(unit) => Some(unit.text.len()),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    assert!(max_unit <= lynkeus_extract::MAX_UNIT_TEXT_BYTES);
}

#[test]
fn binary_file_is_skipped() {
    let events = extract("demo/binario/immagine.png");
    assert_eq!(
        events,
        [ExtractEvent::Skipped {
            inner: Vec::new(),
            reason: SkipReason::Binary
        }]
    );
}
