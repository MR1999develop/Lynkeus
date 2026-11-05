// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Misure del motore del livello 3 del piano di test (P-01, P-02, P-03 e P-05 di
//! `docs/piano-di-test.md`). P-04 (filtro delle estensioni preparato, interno al crate) è il
//! test di libreria `prepared_filter_timing`.
//!
//! Sono test `#[ignore]`: li lancia `cargo xtask test-plan`, uno per processo e in release
//! (così il picco di memoria è quello della sola misura), oppure a mano:
//!
//! ```text
//! cargo test --release -p lynkeus-engine --test prestazioni <nome> -- --ignored --exact --nocapture
//! ```
//!
//! Ognuno stampa la riga `MISURA` del modulo `misura`; le soglie le confronta xtask. Ogni
//! misura verifica anche l'esito della ricerca: il tempo di una ricerca sbagliata non vale.

mod common;
mod misura;

use std::hint::black_box;
use std::io::{self, Read};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use common::{documents_with, drain, fixtures, launcher, request, run, start_with};
use lynkeus_core::Combine;
use lynkeus_engine::{DocumentSource, ExtractEnd, LaunchedWorker, Origin, Outcome, WorkerLauncher};
use lynkeus_extract::{ExtractEvent, ExtractOptions};
use misura::Distribution;

/// Ripetizioni a caldo dopo quella scartata (metodo comune del piano).
const REPETITIONS: usize = 15;

// --- P-01: scansione di 100.000 file di testo ---------------------------------------------

/// Cartelle dell'albero di P-01.
const P01_DIRS: usize = 100;
/// File per cartella: 100 × 1000 = 100.000 file.
const P01_FILES_PER_DIR: usize = 1_000;
/// Un file su tanti contiene "comprare la carta" (due parole con "ar"); gli altri nessuna.
const P01_MATCH_EVERY: usize = 10;

/// P-01: "ar" in 100.000 file di testo piccoli, in una cartella generata nella cartella
/// temporanea e cancellata alla fine; tempo fino a `Finished`. La misura a freddo non si fa:
/// svuotare la cache dei file del sistema richiede privilegi (`cold=not_measured`).
#[test]
#[ignore = "misura P-01 del piano di test, lanciata da cargo xtask test-plan"]
fn p01_scan_100k_text_files() {
    let (dir, prepare) = misura::timed(text_tree);
    let files = P01_DIRS * P01_FILES_PER_DIR;
    let expected_hits = files / P01_MATCH_EVERY * 2;
    let times = misura::repeat(REPETITIONS, || {
        let (found, took) =
            misura::timed(|| run(request(vec![dir.path().into()], &["ar"], Combine::Any)));
        assert_eq!(found.summary.outcome, Outcome::Completed);
        assert!(found.warnings.is_empty(), "{:?}", found.warnings);
        assert_eq!(found.hits.len(), expected_hits);
        took
    });
    misura::print_line(
        "P-01",
        &Distribution::new(&times),
        &[
            ("files", files.to_string()),
            ("prepare_ms", misura::duration_ms(prepare)),
            ("cold", "not_measured".to_owned()),
        ],
    );
}

/// L'albero di P-01 in una cartella temporanea nuova (cancellata dal suo `Drop`, anche se il
/// test fallisce).
fn text_tree() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("lynkeus-p01-")
        .tempdir()
        .expect("cartella temporanea");
    for d in 0..P01_DIRS {
        let sub = dir.path().join(format!("d{d:03}"));
        std::fs::create_dir(&sub).expect("cartella");
        for f in 0..P01_FILES_PER_DIR {
            let n = d * P01_FILES_PER_DIR + f;
            // Nessuna parola con "ar" fuori dalla riga attesa: "Prima riga del file",
            // "testo senza corrispondenze", "ultima riga".
            let middle = if n.is_multiple_of(P01_MATCH_EVERY) {
                "comprare la carta"
            } else {
                "testo senza corrispondenze"
            };
            let content = format!("Prima riga del file {n}\n{middle}\nultima riga\n");
            std::fs::write(sub.join(format!("f{f:04}.txt")), content).expect("file");
        }
    }
    dir
}

// --- P-02: Annulla sotto carico -------------------------------------------------------------

/// Processi estrattori occupati su un file lento durante l'Annulla.
const P02_PROCESSES: usize = 4;
/// Ripetizioni sotto carico (il piano ne chiede 30).
const P02_REPETITIONS: usize = 30;
/// Tempo massimo perché un processo estrattore parta e inizi il file lento.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// P-02: 4 processi occupati su un file lento, CPU satura, Annulla; tempo da Annulla a
/// `Finished`.
#[test]
#[ignore = "misura P-02 del piano di test, lanciata da cargo xtask test-plan"]
fn p02_cancel_under_load() {
    let times = misura::repeat(P02_REPETITIONS, || cancel_with_busy_workers(P02_PROCESSES));
    let cores = thread::available_parallelism().map_or(0, NonZeroUsize::get);
    misura::print_line(
        "P-02",
        &Distribution::new(&times),
        &[
            ("processes", P02_PROCESSES.to_string()),
            ("load_threads", cores.to_string()),
        ],
    );
}

/// Inoltra le letture e avvisa una volta, al primo byte letto.
struct FirstByte {
    inner: Box<dyn Read + Send>,
    signal: Option<mpsc::Sender<()>>,
}

impl Read for FirstByte {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        if read > 0
            && let Some(signal) = self.signal.take()
        {
            // La misura può essere già finita: un avviso perso non conta.
            if signal.send(()).is_err() {}
        }
        Ok(read)
    }
}

/// Il lanciatore dei test che avvisa quando un processo risponde la prima volta: per
/// `lento.*` vuol dire che ha ricevuto la richiesta ed è occupato.
struct Signaling {
    inner: Arc<dyn WorkerLauncher>,
    busy: Mutex<mpsc::Sender<()>>,
}

impl WorkerLauncher for Signaling {
    fn launch(&self) -> io::Result<LaunchedWorker> {
        let mut launched = self.inner.launch()?;
        let signal = self
            .busy
            .lock()
            .map_err(|_| io::Error::other("avviso avvelenato"))?
            .clone();
        launched.stdout = Box::new(FirstByte {
            inner: launched.stdout,
            signal: Some(signal),
        });
        Ok(launched)
    }
}

/// Un thread per core che gira a vuoto finché vive il valore.
struct CpuLoad {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl CpuLoad {
    fn start() -> CpuLoad {
        let stop = Arc::new(AtomicBool::new(false));
        let cores = thread::available_parallelism().map_or(2, NonZeroUsize::get);
        let threads = (0..cores)
            .map(|_| {
                let stop = Arc::clone(&stop);
                thread::spawn(move || {
                    let mut counter = 0_u64;
                    while !stop.load(Ordering::Relaxed) {
                        counter = black_box(counter.wrapping_add(1));
                    }
                })
            })
            .collect();
        CpuLoad { stop, threads }
    }
}

impl Drop for CpuLoad {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            // Un panic di un thread di carico non riguarda la misura.
            if thread.join().is_err() {}
        }
    }
}

/// `count` file lenti (`lento.pdf`, riconosciuti dal processo estrattore dei test) in una
/// cartella nuova.
fn slow_files(count: usize) -> (tempfile::TempDir, Vec<PathBuf>) {
    let dir = tempfile::tempdir().expect("cartella temporanea");
    let files = (0..count)
        .map(|n| {
            let sub = dir.path().join(format!("d{n}"));
            std::fs::create_dir(&sub).expect("cartella");
            let path = sub.join("lento.pdf");
            std::fs::write(&path, b"%PDF-1.4\n").expect("file");
            path
        })
        .collect();
    (dir, files)
}

/// Avvia una ricerca su `processes` file lenti con altrettanti processi, aspetta che tutti
/// siano occupati, satura la CPU e annulla: il tempo da Annulla a `Finished`.
fn cancel_with_busy_workers(processes: usize) -> Duration {
    let (_dir, files) = slow_files(processes);
    let (busy_tx, busy_rx) = mpsc::channel();
    let signaling: Arc<dyn WorkerLauncher> = Arc::new(Signaling {
        inner: launcher(),
        busy: Mutex::new(busy_tx),
    });
    let source = documents_with(signaling, processes, None);
    let req = request(files, &["carta"], Combine::Any);
    let (mut handle, wake) = start_with(req, Some(Arc::clone(&source)));
    for _ in 0..processes {
        busy_rx
            .recv_timeout(START_TIMEOUT)
            .expect("un processo non ha iniziato il file lento");
    }
    let _load = CpuLoad::start();
    let cancelled = Instant::now();
    handle.cancel();
    let found = drain(&mut handle, &wake);
    let took = cancelled.elapsed();
    // Come l'app all'uscita: i processi lenti si chiudono prima della ripetizione seguente.
    source.pool().shutdown();
    assert_eq!(found.summary.outcome, Outcome::Cancelled);
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    took
}

// --- P-03: memoria con AND -----------------------------------------------------------------

/// File di P-03.
const P03_FILES: usize = 16;
/// Ripetizioni di P-03 (il piano ne chiede 5).
const P03_REPETITIONS: usize = 5;

/// P-03: 16 file con 300.000 "il" ciascuno, AND con "zzz": nessun risultato; il picco di
/// memoria del processo dice quanti risultati restano trattenuti tra i thread.
#[test]
#[ignore = "misura P-03 del piano di test, lanciata da cargo xtask test-plan"]
fn p03_and_retained_hits_memory() {
    let dir = tempfile::tempdir().expect("cartella temporanea");
    // 30 "il" per riga, 10.000 righe: 300.000 "il" per file.
    let content = format!("{}\n", "il ".repeat(30)).repeat(10_000);
    for n in 0..P03_FILES {
        std::fs::write(dir.path().join(format!("f{n:02}.txt")), &content).expect("file");
    }
    let times = misura::repeat(P03_REPETITIONS, || {
        let (found, took) = misura::timed(|| {
            run(request(
                vec![dir.path().into()],
                &["il", "zzz"],
                Combine::All,
            ))
        });
        assert_eq!(found.summary.outcome, Outcome::Completed);
        assert!(found.hits.is_empty(), "nessun file ha anche \"zzz\"");
        took
    });
    misura::print_line(
        "P-03",
        &Distribution::new(&times),
        &[("files", P03_FILES.to_string())],
    );
}

// --- P-05: cache contro estrazione ----------------------------------------------------------

/// P-05: i documenti demo letti dal processo estrattore (`baseline_*`) e poi dalla cache;
/// i tempi principali sono quelli della cache.
#[test]
#[ignore = "misura P-05 del piano di test, lanciata da cargo xtask test-plan"]
fn p05_cache_against_extraction() {
    let cache = tempfile::tempdir().expect("cartella della cache");
    let files = [
        fixtures("demo/ufficio/note.docx"),
        fixtures("demo/musica/archivio.zip"),
    ];
    let extractor = documents_with(launcher(), 1, None);
    let cached = documents_with(launcher(), 1, Some(&cache.path().join("cache")));
    let from_extractor = misura::repeat(REPETITIONS, || {
        read_documents(&extractor, &files, Origin::Extractor)
    });
    // Il primo giro riempie la cache.
    let _fill = read_documents(&cached, &files, Origin::Extractor);
    let from_cache = misura::repeat(REPETITIONS, || {
        read_documents(&cached, &files, Origin::Cache)
    });
    extractor.pool().shutdown();
    cached.pool().shutdown();
    let baseline = Distribution::new(&from_extractor);
    misura::print_line(
        "P-05",
        &Distribution::new(&from_cache),
        &[
            ("documents", files.len().to_string()),
            ("baseline_p50_ms", misura::ms(baseline.p50_ms())),
            ("baseline_p95_ms", misura::ms(baseline.p95_ms())),
        ],
    );
}

/// Il tempo per leggere tutti i `files` da `source`, verificando da dove arrivano.
fn read_documents(source: &DocumentSource, files: &[PathBuf], expected: Origin) -> Duration {
    let never = || false;
    let started = Instant::now();
    for file in files {
        let mut events: Vec<ExtractEvent> = Vec::new();
        let (end, origin) = source.extract(
            file,
            &ExtractOptions::default(),
            Duration::from_secs(30),
            &never,
            &mut events,
        );
        assert_eq!(end, ExtractEnd::Completed, "{}", file.display());
        assert_eq!(origin, expected, "{}", file.display());
        assert!(!events.is_empty(), "{}", file.display());
    }
    started.elapsed()
}
