// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Test d'accettazione: l'app costruita da `Application::build` cerca "ar" in
//! `fixtures/demo` e trova le parole attese (vedi `fixtures/README.md`).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use lynkeus_app::i18n::Language;
use lynkeus_app::{Application, Pump, SearchStatus, Settings};
use lynkeus_core::Location;
use lynkeus_engine::{Hit, Outcome};

const TIMEOUT: Duration = Duration::from_secs(60);

fn demo_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/demo")
}

/// Avvia la ricerca e pompa gli eventi finché finisce, aspettando i segnali del worker
/// con una scadenza (nessuna attesa a vuoto).
fn search(root: &std::path::Path, term: &str) -> Application {
    let settings = Settings {
        language: Some(Language::Italian),
        ..Settings::default()
    };
    let mut app = Application::build(settings).expect("app costruita");
    app.set_root(root);
    app.set_term(term);
    let (wake_tx, wake_rx) = mpsc::channel::<()>();
    let notify = Arc::new(move || {
        // A ricerca finita il test non ascolta più: un segnale perso non conta.
        if wake_tx.send(()).is_err() {}
    });
    app.start_search(notify).expect("ricerca avviata");
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match app.pump_events(usize::MAX) {
            Pump::Ended | Pump::NoSearch => return app,
            Pump::MorePending => {}
            Pump::Drained => {
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(!left.is_zero(), "ricerca non finita entro {TIMEOUT:?}");
                if wake_rx.recv_timeout(left).is_err() {}
            }
        }
    }
}

fn file_name(hit: &Hit) -> String {
    hit.doc
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn find<'a>(hits: &'a [Hit], file: &str, word: &str) -> Option<&'a Hit> {
    hits.iter().find(|h| file_name(h) == file && h.word == word)
}

#[test]
fn ar_in_demo_finds_the_expected_words() {
    let app = search(&demo_dir(), "ar");
    let SearchStatus::Finished(summary) = app.status() else {
        panic!("ricerca non finita: {:?}", app.status());
    };
    assert_eq!(summary.outcome, Outcome::Completed);
    let hits = &app.results().hits;

    let aragosta = find(hits, "mare.txt", "l'aragosta").expect("l'aragosta in mare.txt");
    assert_eq!(aragosta.location, Location::Line(12));
    assert!(find(hits, "mare.txt", "calamari").is_some());

    let arte = find(hits, "citta.txt", "d'arte").expect("d'arte in citta.txt");
    assert_eq!(arte.location, Location::Line(1));
    let armadio = find(hits, "citta.txt", "ARMADIO").expect("ARMADIO in citta.txt");
    assert_eq!(armadio.location, Location::Line(3));
    let citta_words: Vec<&str> = hits
        .iter()
        .filter(|h| file_name(h) == "citta.txt")
        .map(|h| h.word.as_str())
        .collect();
    assert_eq!(citta_words, ["d'arte", "ARMADIO"]);
    let citta_context = &arte.snippet.text;
    assert!(citta_context.contains("città"), "{citta_context}");

    assert!(
        hits.iter().all(|h| file_name(h) != "immagine.png"),
        "nessun risultato da un file binario"
    );
    assert!(summary.counts.binary_skipped >= 1);
}

#[test]
fn empty_term_is_refused_before_starting() {
    let mut app = Application::build(Settings::default()).expect("app costruita");
    app.set_root(&demo_dir());
    app.set_term("  ");
    let notify = Arc::new(|| {});
    assert!(app.start_search(notify).is_err());
    assert!(!app.is_searching());
    assert_eq!(app.status(), SearchStatus::Idle);
}
