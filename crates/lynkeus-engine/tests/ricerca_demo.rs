// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Ricerca reale su `fixtures/demo`: eventi, limite dei risultati, annulla.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use lynkeus_core::{Combine, Location, MatchOptions, Query, Term, WildcardMode};
use lynkeus_engine::{
    Filters, Hit, Limits, Outcome, Received, SearchEvent, SearchRequest, Summary, start_search,
};

const TIMEOUT: Duration = Duration::from_secs(30);

fn demo(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/demo")
        .join(sub)
}

fn request(roots: Vec<PathBuf>, term: &str, limits: Limits) -> SearchRequest {
    let term = Term::parse(term, WildcardMode::Enabled).expect("termine valido");
    let query = Query::new(vec![term], MatchOptions::default(), Combine::Any)
        .and_then(|q| q.compile())
        .expect("query valida");
    SearchRequest {
        roots,
        query,
        filters: Filters::default(),
        limits,
    }
}

/// Esegue la ricerca fino a `Finished`, aspettando sul segnale di `Notify` con scadenza.
fn run(request: SearchRequest, cancel_at_start: bool) -> (Vec<Hit>, Summary) {
    let (wake_tx, wake_rx) = mpsc::channel::<()>();
    let notify = Arc::new(move || {
        // Il test può essere già finito: un segnale perso non conta.
        if wake_tx.send(()).is_err() {}
    });
    let mut handle = start_search(request, notify).expect("thread avviato");
    if cancel_at_start {
        handle.cancel();
    }
    let deadline = Instant::now() + TIMEOUT;
    let mut hits = Vec::new();
    loop {
        match handle.try_recv() {
            Received::Event(SearchEvent::Hits(batch)) => hits.extend(batch),
            Received::Event(SearchEvent::Warning(_)) => {}
            Received::Event(SearchEvent::Finished(summary)) => return (hits, summary),
            Received::Ended | Received::WorkerLost => panic!("worker finito senza riepilogo"),
            Received::Empty => {
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(!left.is_zero(), "ricerca non finita entro {TIMEOUT:?}");
                // Il segnale può arrivare prima o dopo il controllo: si ricontrolla comunque.
                if wake_rx.recv_timeout(left).is_err() {}
            }
        }
    }
}

#[test]
fn finds_aragosta_in_mare_txt() {
    let (hits, summary) = run(
        request(vec![demo("ricette")], "ar", Limits::default()),
        false,
    );
    assert_eq!(summary.outcome, Outcome::Completed);
    let aragosta = hits
        .iter()
        .find(|h| h.word == "l'aragosta")
        .expect("l'aragosta trovata");
    assert_eq!(aragosta.location, Location::Line(12));
    assert!(aragosta.doc.path.ends_with("mare.txt"));
    assert_eq!(summary.counts.hits, u64::try_from(hits.len()).unwrap_or(0));
}

#[test]
fn stops_at_the_result_limit() {
    let limits = Limits {
        max_results: 1,
        ..Limits::default()
    };
    let (hits, summary) = run(request(vec![demo("")], "ar", limits), false);
    assert_eq!(summary.outcome, Outcome::LimitReached);
    assert_eq!(hits.len(), 1);
}

#[test]
fn cancel_before_the_first_file() {
    let (hits, summary) = run(request(vec![demo("")], "ar", Limits::default()), true);
    assert_eq!(summary.outcome, Outcome::Cancelled);
    assert!(hits.is_empty());
}

#[test]
fn missing_root_becomes_a_warning() {
    let (hits, summary) = run(
        request(vec![demo("non-esiste")], "ar", Limits::default()),
        false,
    );
    assert_eq!(summary.outcome, Outcome::Completed);
    assert!(hits.is_empty());
    assert_eq!(summary.counts.warnings, 1);
}
