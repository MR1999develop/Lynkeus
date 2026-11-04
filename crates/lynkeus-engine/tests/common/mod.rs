// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Driver comune dei test d'integrazione: prepara le richieste ed esegue una ricerca fino a
//! `Finished`, aspettando i segnali di `Notify` con una scadenza (nessuna attesa a vuoto).

#![allow(
    dead_code,
    reason = "ogni file di test usa una parte diversa del driver"
)]

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use lynkeus_core::{Combine, Location, MatchOptions, Query, Term, WildcardMode};
use lynkeus_engine::{
    CommandLauncher, DEFAULT_CACHE_SIZE, DocumentSource, ExtractorPool, Filters, Hit, Limits,
    Received, SearchEvent, SearchHandle, SearchRequest, Summary, TextCache, Warning,
    WorkerLauncher, start_search,
};

/// Tempo massimo di una ricerca nei test.
pub const TIMEOUT: Duration = Duration::from_secs(60);

/// Tutto ciò che una ricerca ha prodotto.
#[derive(Debug)]
pub struct Outcome {
    pub hits: Vec<Hit>,
    pub warnings: Vec<Warning>,
    pub summary: Summary,
}

impl Outcome {
    /// (nome del file, parola, posizione) di ogni risultato, ordinati.
    pub fn rows(&self) -> Vec<(String, String, Location)> {
        let mut rows: Vec<_> = self
            .hits
            .iter()
            .map(|h| (file_name(&h.doc.path), h.word.clone(), h.location.clone()))
            .collect();
        rows.sort();
        rows
    }

    /// Nomi dei file con almeno un risultato, ordinati, con le ripetizioni.
    pub fn files(&self) -> Vec<String> {
        let mut files: Vec<_> = self.hits.iter().map(|h| file_name(&h.doc.path)).collect();
        files.sort();
        files
    }
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn fixtures(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(sub)
}

/// Richiesta con filtri e limiti predefiniti.
pub fn request(roots: Vec<PathBuf>, terms: &[&str], combine: Combine) -> SearchRequest {
    let terms = terms
        .iter()
        .map(|t| Term::parse(t, WildcardMode::Enabled).expect("termine valido"))
        .collect();
    let query = Query::new(terms, MatchOptions::default(), combine)
        .and_then(|q| q.compile())
        .expect("query valida");
    SearchRequest {
        roots,
        query,
        filters: Filters::default(),
        limits: Limits::default(),
    }
}

/// Il processo estrattore dei test (`tests/support/test_worker.rs`).
pub fn test_worker() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lynkeus-test-worker"))
}

/// L'avvio portabile del processo estrattore dei test.
pub fn launcher() -> Arc<dyn WorkerLauncher> {
    Arc::new(CommandLauncher::new(test_worker(), None).expect("percorso assoluto"))
}

/// Una fonte dei documenti con `launcher`, `size` processi e, se indicata, la cache in
/// `cache_dir`.
pub fn documents_with(
    launcher: Arc<dyn WorkerLauncher>,
    size: usize,
    cache_dir: Option<&Path>,
) -> Arc<DocumentSource> {
    let size = NonZeroUsize::new(size).expect("almeno un processo");
    let cache = cache_dir.map(|dir| TextCache::new(dir.to_owned(), DEFAULT_CACHE_SIZE));
    Arc::new(DocumentSource::new(
        ExtractorPool::new(launcher, size),
        cache,
    ))
}

/// Una fonte dei documenti con il processo estrattore dei test, senza cache, due processi.
pub fn documents() -> Arc<DocumentSource> {
    documents_with(launcher(), 2, None)
}

/// Avvia la ricerca senza fonte dei documenti e restituisce la maniglia e il segnale di
/// `Notify`.
pub fn start(request: SearchRequest) -> (SearchHandle, mpsc::Receiver<()>) {
    start_with(request, None)
}

/// Avvia la ricerca con `documents`.
pub fn start_with(
    request: SearchRequest,
    documents: Option<Arc<DocumentSource>>,
) -> (SearchHandle, mpsc::Receiver<()>) {
    let (wake_tx, wake_rx) = mpsc::channel::<()>();
    let notify = Arc::new(move || {
        // Il test può essere già finito: un segnale perso non conta.
        if wake_tx.send(()).is_err() {}
    });
    let handle = start_search(request, documents, notify).expect("thread avviato");
    (handle, wake_rx)
}

/// Il prossimo evento, aspettando il segnale con scadenza.
pub fn next_event(
    handle: &mut SearchHandle,
    wake: &mpsc::Receiver<()>,
    deadline: Instant,
) -> SearchEvent {
    loop {
        match handle.try_recv() {
            Received::Event(event) => return event,
            Received::Ended | Received::WorkerLost => panic!("worker finito senza riepilogo"),
            Received::Empty => {
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(!left.is_zero(), "ricerca non finita entro {TIMEOUT:?}");
                // Il segnale può arrivare prima o dopo il controllo: si ricontrolla comunque.
                if wake.recv_timeout(left).is_err() {}
            }
        }
    }
}

/// Raccoglie gli eventi fino a `Finished`.
pub fn drain(handle: &mut SearchHandle, wake: &mpsc::Receiver<()>) -> Outcome {
    let deadline = Instant::now() + TIMEOUT;
    let mut hits = Vec::new();
    let mut warnings = Vec::new();
    loop {
        match next_event(handle, wake, deadline) {
            SearchEvent::Hits(batch) => {
                assert!(!batch.is_empty(), "lotto vuoto");
                hits.extend(batch);
            }
            SearchEvent::Warning(warning) => warnings.push(warning),
            SearchEvent::Finished(summary) => {
                return Outcome {
                    hits,
                    warnings,
                    summary,
                };
            }
        }
    }
}

/// Esegue la ricerca dall'inizio alla fine, senza fonte dei documenti.
pub fn run(request: SearchRequest) -> Outcome {
    run_with(request, None)
}

/// Esegue la ricerca dall'inizio alla fine con `documents`.
pub fn run_with(request: SearchRequest, documents: Option<Arc<DocumentSource>>) -> Outcome {
    let (mut handle, wake) = start_with(request, documents);
    drain(&mut handle, &wake)
}
