// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Annulla con processi estrattori occupati e la CPU satura: la ricerca finisce entro
//! [`CANCEL_BUDGET`] (regola dei tempi di risposta). Un file di test a sé, perché il carico
//! sulla CPU non rallenti i test degli altri file.
//!
//! Il test non aspetta a tempo: il lanciatore avvisa quando ogni processo ha iniziato il file
//! lento (primo byte sul suo stdout).

mod common;

use std::io::{self, Read};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use common::{documents_with, drain, launcher, request, start_with};
use lynkeus_core::Combine;
use lynkeus_engine::{LaunchedWorker, Outcome, WorkerLauncher};

/// Tempo massimo tra Annulla e `Finished`.
const CANCEL_BUDGET: Duration = Duration::from_secs(1);

/// Tempo massimo perché un processo estrattore parta e inizi il file.
const START_TIMEOUT: Duration = Duration::from_secs(30);

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
            // Il test può essere già finito: un avviso perso non conta.
            if signal.send(()).is_err() {}
        }
        Ok(read)
    }
}

/// Il lanciatore dei test, che avvisa quando un processo ha risposto per la prima volta: per
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
                        counter = std::hint::black_box(counter.wrapping_add(1));
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
            // Un panic di un thread di carico non riguarda il test.
            if thread.join().is_err() {}
        }
    }
}

/// `count` file lenti in una cartella nuova.
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
    // Come l'app all'uscita: i processi lenti si chiudono prima della fine del test.
    source.pool().shutdown();
    assert_eq!(found.summary.outcome, Outcome::Cancelled);
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    took
}

#[test]
fn cancel_stops_a_busy_worker_within_one_second() {
    let took = cancel_with_busy_workers(1);
    assert!(took < CANCEL_BUDGET, "annulla in {took:?}");
}

/// Misura P-02 (piano di test): 4 processi lenti, CPU satura, 30 ripetizioni; stampa p50,
/// p95 e massimo. `cargo test --release -p lynkeus-engine --test annulla -- --ignored
/// --nocapture`.
#[test]
#[ignore = "misura di prestazioni (P-02), da lanciare a mano in release"]
fn cancel_under_load_distribution() {
    const REPETITIONS: usize = 30;
    // Una esecuzione scartata, poi le ripetizioni.
    let _warmup = cancel_with_busy_workers(4);
    let mut times: Vec<Duration> = (0..REPETITIONS)
        .map(|_| cancel_with_busy_workers(4))
        .collect();
    times.sort_unstable();
    let at = |q: usize| times[(times.len() - 1) * q / 100];
    let cores = thread::available_parallelism().map_or(0, NonZeroUsize::get);
    println!(
        "P-02: {REPETITIONS} ripetizioni, {cores} core logici; p50 {:?}, p95 {:?}, massimo {:?}",
        at(50),
        at(95),
        at(100)
    );
    assert!(at(100) < CANCEL_BUDGET, "massimo {:?}", at(100));
}
