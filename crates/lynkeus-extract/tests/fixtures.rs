// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Estrazione nello stesso processo sulle fixture di `fixtures/formati` e `fixtures/malevoli`
//! (descritte in `fixtures/README.md`): posizione attesa del testo, avviso giusto, sequenza
//! degli eventi valida, nessun panic, tempo limitato.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lynkeus_core::{Location, MailPart, TextUnit};
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

// ── formati/ della fase 5 ─────────────────────────────────────────────────────────────────

#[test]
fn doc_paragraphs() {
    let events = extract("formati/vecchia-lettera.doc");
    assert_eq!(starts(&events), [(Vec::new(), Format::Doc)]);
    assert_eq!(locations_of(&events, "aragosta"), [Location::Paragraph(3)]);
    assert_eq!(locations_of(&events, "città"), [Location::Paragraph(2)]);
    assert!(warnings(&events).is_empty());
}

#[test]
fn xls_cells() {
    let events = extract("formati/vecchia-spesa.xls");
    assert_eq!(starts(&events), [(Vec::new(), Format::Xls)]);
    let spesa = |row, col| Location::Cell {
        sheet: "Spesa".to_owned(),
        row,
        col,
    };
    assert_eq!(locations_of(&events, "aragosta"), [spesa(7, 2)]);
    assert_eq!(locations_of(&events, "città"), [spesa(2, 3)]);
}

/// Un xls vero con un flusso BIFF ben oltre 4 KB (501 righe per 3 colonne, tutte piene):
/// ogni cella si legge, senza avvisi. Il file composto rifatto per `calamine` deve essere
/// uno che `calamine` legge per intero.
#[test]
fn long_xls_reads_every_cell() {
    let events = extract("formati/registro-lungo.xls");
    assert_eq!(starts(&events), [(Vec::new(), Format::Xls)]);
    assert!(warnings(&events).is_empty(), "{:?}", warnings(&events));
    let units = events
        .iter()
        .filter(|e| matches!(e, ExtractEvent::Unit(_)))
        .count();
    assert_eq!(units, 501 * 3);
    let registro = |row, col| Location::Cell {
        sheet: "Registro".to_owned(),
        row,
        col,
    };
    assert_eq!(locations_of(&events, "aragosta"), [registro(501, 3)]);
    assert_eq!(locations_of(&events, "città"), [registro(250, 3)]);
    assert_eq!(locations_of(&events, "Voce 500"), [registro(500, 1)]);
}

#[test]
fn ppt_slides() {
    let events = extract("formati/vecchia-presentazione.ppt");
    assert_eq!(starts(&events), [(Vec::new(), Format::Ppt)]);
    assert_eq!(locations_of(&events, "aragosta"), [Location::Slide(3)]);
    assert_eq!(locations_of(&events, "città"), [Location::Slide(2)]);
    assert!(warnings(&events).is_empty());
}

#[test]
fn rtf_paragraphs_with_hex_and_unicode() {
    let events = extract("formati/lettera.rtf");
    assert_eq!(starts(&events), [(Vec::new(), Format::Rtf)]);
    assert_eq!(locations_of(&events, "aragosta"), [Location::Paragraph(3)]);
    // `citt\'e0` e `perch\u233?`.
    assert_eq!(locations_of(&events, "città"), [Location::Paragraph(2)]);
    assert_eq!(locations_of(&events, "perché"), [Location::Paragraph(2)]);
    assert_eq!(locations_of(&events, "c'è"), [Location::Paragraph(2)]);
}

#[test]
fn doc_that_is_rtf_is_read_as_rtf() {
    let events = extract("formati/in-realta-rtf.doc");
    assert_eq!(starts(&events), [(Vec::new(), Format::Rtf)]);
    assert_eq!(locations_of(&events, "aragosta"), [Location::Paragraph(3)]);
    assert!(warnings(&events).is_empty());
}

fn mail(part: MailPart, line: u32) -> Location {
    Location::Mail { part, line }
}

#[test]
fn eml_with_attachments() {
    let events = extract("formati/ordine.eml");
    assert_eq!(
        starts(&events),
        [
            (Vec::new(), Format::Eml),
            (names(&["allegato.docx"]), Format::Docx),
            (names(&["nota.txt"]), Format::Text),
        ]
    );
    assert_eq!(
        locations_of(&events, "aragosta"),
        [
            mail(MailPart::Subject, 1),
            mail(MailPart::Body, 2),
            Location::Paragraph(2),
            Location::Line(1),
        ]
    );
    assert_eq!(
        locations_of(&events, "mittente@example.com"),
        [mail(MailPart::From, 1)]
    );
    assert_eq!(
        locations_of(&events, "destinatario@example.com"),
        [mail(MailPart::To, 1)]
    );
    assert!(warnings(&events).is_empty());
}

#[test]
fn eml_body_only() {
    let events = extract("formati/saluti.eml");
    assert_eq!(starts(&events), [(Vec::new(), Format::Eml)]);
    assert_eq!(locations_of(&events, "aragosta"), [mail(MailPart::Body, 2)]);
    assert_eq!(locations_of(&events, "città"), [mail(MailPart::Body, 2)]);
}

#[test]
fn epub_chapters() {
    let events = extract("formati/libro.epub");
    let cap1 = names(&["OEBPS/cap1.xhtml"]);
    let cap2 = names(&["OEBPS/cap2.xhtml"]);
    assert_eq!(
        starts(&events),
        [(cap1.clone(), Format::Epub), (cap2.clone(), Format::Epub)]
    );
    // Il titolo h1 è il paragrafo 1 di ogni capitolo.
    assert_eq!(
        located(&events, "aragosta"),
        [(cap2, Location::Paragraph(2))]
    );
    assert_eq!(
        located(&events, "città"),
        [(cap1.clone(), Location::Paragraph(2))]
    );
    assert_eq!(located(&events, "Perché"), [(cap1, Location::Paragraph(3))]);
}

/// Come [`locations_of`], con il documento (`inner`) di ogni posizione.
fn located(events: &[ExtractEvent], word: &str) -> Vec<(Vec<String>, Location)> {
    let mut inner = Vec::new();
    let mut found = Vec::new();
    for event in events {
        match event {
            ExtractEvent::DocumentStart { inner: i, .. } => inner.clone_from(i),
            ExtractEvent::Unit(unit) => {
                for (k, line) in unit.text.split('\n').enumerate() {
                    if line.contains(word) {
                        let k = u32::try_from(k).unwrap_or(u32::MAX);
                        found.push((inner.clone(), line_location(&unit.location, k)));
                    }
                }
            }
            _ => {}
        }
    }
    found
}

fn ricetta_in(relative: &str) {
    let events = extract(relative);
    // 7-Zip ordina le voci a modo suo: conta l'insieme.
    let mut documents = starts(&events);
    documents.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        documents,
        [
            (names(&["cartella/nota.txt"]), Format::Text),
            (names(&["cartella/ricetta.txt"]), Format::Text),
        ],
        "{relative}"
    );
    assert_eq!(
        located(&events, "aragosta"),
        [(names(&["cartella/ricetta.txt"]), Location::Line(2))],
        "{relative}"
    );
    assert!(warnings(&events).is_empty(), "{relative}");
}

#[test]
fn tar_entries() {
    ricetta_in("formati/archivio.tar");
}

#[test]
fn tar_gz_entries() {
    ricetta_in("formati/archivio.tar.gz");
}

#[test]
fn sevenz_entries() {
    ricetta_in("formati/archivio.7z");
}

#[test]
fn gz_single_file() {
    let events = extract("formati/appunti.txt.gz");
    assert_eq!(starts(&events), [(names(&["appunti.txt"]), Format::Text)]);
    assert_eq!(locations_of(&events, "aragosta"), [Location::Line(2)]);
    assert_eq!(locations_of(&events, "città"), [Location::Line(2)]);
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

/// A1 e XFD1048576 in un xlsx di 2 KB: le due celle si leggono a flusso, senza riservare il
/// rettangolo che le contiene (centinaia di GB).
#[test]
fn sparse_xlsx_cells_are_read_in_flow() {
    let events = extract("malevoli/sparso.xlsx");
    assert!(warnings(&events).is_empty(), "{events:?}");
    let cell = |row, col| Location::Cell {
        sheet: "Sparso".to_owned(),
        row,
        col,
    };
    assert_eq!(locations_of(&events, "aragosta"), [cell(1, 1)]);
    assert_eq!(locations_of(&events, "città"), [cell(1_048_576, 16_384)]);
}

/// Un ods di meno di 1 KB con un testo di 300 caratteri ripetuto su 255 righe per 16.384
/// colonne: le celle sono poche per un tetto in celle, ma il testo clonato per ognuna supera
/// 1 GB. Avviso "memoria" sul file, nessuna cella letta.
#[test]
fn ods_with_repeated_text_is_a_memory_limit() {
    let events = extract("malevoli/testo-ripetuto.ods");
    assert_eq!(
        warnings(&events),
        [(Vec::new(), WarningKind::LimitExceeded(LimitKind::Memory))]
    );
    assert!(
        !events.iter().any(|e| matches!(e, ExtractEvent::Unit(_))),
        "nessuna cella"
    );
}

/// Un 7z che dichiara un dizionario LZMA2 di 2 GiB per 31 byte di testo: la libreria non
/// alloca il dizionario in anticipo, la voce si legge. Nel processo estrattore vero, con il
/// limite di memoria, lo prova anche `tests/processo.rs`.
#[test]
fn sevenz_huge_dictionary_is_not_allocated_up_front() {
    let events = extract("malevoli/dizionario-enorme.7z");
    assert!(warnings(&events).is_empty(), "{events:?}");
    assert_eq!(locations_of(&events, "aragosta"), [Location::Line(1)]);
}

/// La stessa diapositiva elencata 10.000 volte nella presentazione si legge una volta sola.
#[test]
fn pptx_repeated_slide_ids_read_once() {
    let events = extract("malevoli/slide-ripetute.pptx");
    assert_eq!(locations_of(&events, "aragosta"), [Location::Slide(1)]);
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
