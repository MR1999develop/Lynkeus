// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Ricerca reale su `fixtures/demo` e `fixtures/malevoli/riga-lunga.txt` (vedi
//! `fixtures/README.md`): codifiche, binari, limite dei risultati, annulla.

mod common;

use common::{fixtures, request, run, start};
use lynkeus_core::{Combine, Location};
use lynkeus_engine::{Limits, Outcome, WarningKind};

fn row(file: &str, word: &str, line: u32) -> (String, String, Location) {
    (file.to_owned(), word.to_owned(), Location::Line(line))
}

#[test]
fn ar_in_the_text_files_of_demo() {
    let found = run(request(vec![fixtures("demo")], &["ar"], Combine::Any));
    assert_eq!(found.summary.outcome, Outcome::Completed);
    assert_eq!(
        found.rows(),
        [
            row("citta.txt", "ARMADIO", 3),
            row("citta.txt", "d'arte", 1),
            row("mare.txt", "calamari", 5),
            row("mare.txt", "l'aragosta", 12),
            row("utf16.txt", "carta", 1),
            row("vecchio-1252.txt", "carità", 2),
        ]
    );
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    let counts = found.summary.counts;
    assert_eq!(counts.hits, 6);
    // Binari: immagine.png. Senza fonte dei documenti archivio.zip e note.docx si saltano.
    assert_eq!(counts.binary_skipped, 1);
    assert_eq!(counts.files_skipped, 2);
    assert_eq!(counts.files_seen, 7);
    assert_eq!(counts.files_searched, 4);
}

#[test]
fn hits_carry_root_index_and_shared_document() {
    let found = run(request(
        vec![fixtures("demo/viaggi"), fixtures("demo/ricette")],
        &["ar"],
        Combine::Any,
    ));
    for hit in &found.hits {
        let expected = if hit.doc.path.ends_with("citta.txt") {
            0
        } else {
            1
        };
        assert_eq!(hit.root, expected, "{}", hit.doc);
    }
    let context = &found
        .hits
        .iter()
        .find(|h| h.word == "d'arte")
        .expect("d'arte")
        .snippet;
    assert!(context.text.contains("città"), "{}", context.text);
}

#[test]
fn a_very_long_line_does_not_hide_the_next_one() {
    let found = run(request(
        vec![fixtures("malevoli/riga-lunga.txt")],
        &["ar"],
        Combine::Any,
    ));
    assert_eq!(found.summary.outcome, Outcome::Completed);
    assert_eq!(found.rows(), [row("riga-lunga.txt", "aragosta", 2)]);
}

#[test]
fn stops_at_the_result_limit() {
    let mut req = request(vec![fixtures("demo")], &["ar"], Combine::Any);
    req.limits = Limits {
        max_results: 1,
        ..Limits::default()
    };
    let found = run(req);
    assert_eq!(found.summary.outcome, Outcome::LimitReached);
    assert_eq!(found.hits.len(), 1);
    assert_eq!(found.summary.counts.hits, 1);
}

#[test]
fn cancel_before_the_first_file() {
    let (mut handle, wake) = start(request(vec![fixtures("demo")], &["ar"], Combine::Any));
    handle.cancel();
    let found = common::drain(&mut handle, &wake);
    assert_eq!(found.summary.outcome, Outcome::Cancelled);
    assert!(found.hits.is_empty());
}

#[test]
fn missing_root_becomes_a_warning_and_the_others_are_searched() {
    let found = run(request(
        vec![fixtures("demo/non-esiste"), fixtures("demo/ricette")],
        &["ar"],
        Combine::Any,
    ));
    assert_eq!(found.summary.outcome, Outcome::Completed);
    assert_eq!(found.warnings.len(), 1);
    assert_eq!(found.warnings[0].kind, WarningKind::NotFound);
    assert!(found.warnings[0].doc.path.ends_with("non-esiste"));
    assert_eq!(found.summary.counts.warnings, 1);
    assert_eq!(found.hits.len(), 2);
}
