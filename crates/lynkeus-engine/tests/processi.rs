// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Vita e confini del processo estrattore avviato con il lanciatore portabile
//! (`CommandLauncher`, usato su Linux e nei test): canali e descrittori che il figlio
//! eredita, uscita alla chiusura dello stdin, nessun orfano quando il padre muore.
//!
//! Su Windows l'app usa un lanciatore suo (`CreateProcessW` con `HANDLE_LIST` dentro un Job
//! Object, in `lynkeus-app`), non raggiungibile da qui: questi test provano il lanciatore
//! portabile. Su Windows `Command` fa ereditare al figlio tutti gli handle ereditabili del
//! padre; gli handle che `std` crea (file, capi dei canali tenuti dal padre) non lo sono, ed è
//! ciò che `closing_stdin_stops_a_worker_while_a_sibling_lives` verifica in modo indiretto.

mod common;

use std::fs::{File, TryLockError};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::{launcher, test_worker};
use lynkeus_engine::WorkerExit;

/// Tempo massimo perché un processo esca dopo la chiusura del suo stdin.
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Intervallo dei controlli sul lock (non c'è un'attesa con scadenza sul lock di un file).
const LOCK_POLL: Duration = Duration::from_millis(20);

/// Chiudere lo stdin di un processo lo ferma anche se un fratello avviato dopo è vivo: il
/// fratello non ha ereditato il capo di scrittura di quel canale (che lo terrebbe aperto).
#[test]
fn closing_stdin_stops_a_worker_while_a_sibling_lives() {
    let launcher = launcher();
    let first = launcher.launch().expect("primo processo");
    let mut second = launcher.launch().expect("secondo processo");
    let mut first_process = first.process;
    let _first_stdout = first.stdout;
    drop(first.stdin);
    assert_eq!(
        first_process.wait_timeout(EXIT_TIMEOUT).expect("attesa"),
        Some(WorkerExit::Success)
    );
    assert_eq!(
        second.process.wait_timeout(Duration::ZERO).expect("attesa"),
        None,
        "il fratello deve essere ancora vivo"
    );
    drop(second.stdin);
    assert_eq!(
        second.process.wait_timeout(EXIT_TIMEOUT).expect("attesa"),
        Some(WorkerExit::Success)
    );
}

/// Su Linux il figlio vede solo 0, 1 e 2, anche se il padre tiene aperti file e i canali di
/// un altro processo estrattore.
#[cfg(target_os = "linux")]
#[test]
fn worker_inherits_only_the_standard_descriptors() {
    use std::num::NonZeroUsize;

    use lynkeus_engine::{ExtractEnd, ExtractorPool};
    use lynkeus_extract::{ExtractEvent, ExtractOptions};

    let dir = tempfile::tempdir().expect("cartella temporanea");
    let probe = dir.path().join("descrittori.txt");
    std::fs::write(&probe, b"").expect("file");
    // Descrittori aperti nel padre al momento dell'avvio.
    let _held: Vec<File> = (0..3)
        .map(|_| File::open(&probe).expect("apertura"))
        .collect();
    let _sibling = launcher().launch().expect("fratello");

    let pool = ExtractorPool::new(launcher(), NonZeroUsize::MIN);
    let never = || false;
    let mut events: Vec<ExtractEvent> = Vec::new();
    let end = pool.extract(
        &probe,
        &ExtractOptions::default(),
        Duration::from_secs(30),
        &never,
        &mut events,
    );
    assert_eq!(end, ExtractEnd::Completed);
    let listed: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            ExtractEvent::Unit(unit) => Some(unit.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(listed, ["0 1 2"]);
}

/// Un processo figlio che viene ucciso alla fine del test, anche se il test fallisce.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        // Già uscito o già ucciso: nulla da fare.
        let _killed = self.0.kill();
        let _reaped = self.0.wait();
    }
}

/// Vero se il file è bloccato da un altro processo.
fn is_locked(path: &Path) -> bool {
    let file = File::options()
        .read(true)
        .write(true)
        .open(path)
        .expect("apertura del lock");
    match file.try_lock() {
        Ok(()) => false,
        Err(TryLockError::WouldBlock) => true,
        Err(TryLockError::Error(error)) => panic!("lock: {error}"),
    }
}

/// Il padre (un processo intermedio, al posto dell'app) muore di colpo: il processo
/// estrattore resta senza chi scrive nel suo stdin, vede la fine del flusso ed esce entro
/// [`EXIT_TIMEOUT`]. Su Linux è l'unica difesa contro gli orfani; su Windows c'è anche il Job
/// Object dell'app. Il figlio tiene un lock sul file finché vive.
#[test]
fn an_orphaned_worker_exits_when_its_parent_dies() {
    let dir = tempfile::tempdir().expect("cartella temporanea");
    let lock = dir.path().join("lucchetto.txt");
    std::fs::write(&lock, b"").expect("file");

    let mut parent = KillOnDrop(
        Command::new(test_worker())
            .arg("--orphan-parent")
            .arg(&lock)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("padre intermedio"),
    );
    let stdout = parent.0.stdout.take().expect("stdout del padre");
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let read = BufReader::new(stdout).read_line(&mut line);
        // Il test può essere già fallito: un messaggio perso non conta.
        if ready_tx.send(read.map(|_| line)).is_err() {}
    });
    let line = ready_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("il padre non ha detto «pronto»")
        .expect("lettura dal padre");
    assert_eq!(line.trim_end(), "pronto");
    assert!(is_locked(&lock), "il figlio deve tenere il lock");

    let killed = Instant::now();
    parent.0.kill().expect("uccisione del padre");
    parent.0.wait().expect("attesa del padre");
    while is_locked(&lock) {
        assert!(
            killed.elapsed() < EXIT_TIMEOUT,
            "il figlio è ancora vivo dopo {EXIT_TIMEOUT:?}"
        );
        std::thread::sleep(LOCK_POLL);
    }
}
