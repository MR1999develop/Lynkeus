// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Il processo estrattore vero: questo stesso eseguibile, avviato come figlio, esegue
//! [`worker::run`] come fa `lynkeus-searcher --extract-worker`.
//!
//! - Un foglio xlsx con due celle lontanissime (A1 e XFD1048576) dentro uno zip non termina
//!   il processo e la voce che lo segue si legge.
//! - Un 7z con un dizionario dichiarato di 2 GiB si legge entro il limite di memoria.
//! - Su Linux, un estrattore occupato (con molto lavoro in coda) esce poco dopo la morte del
//!   suo padre, senza finire il lavoro.
//!
//! Senza l'harness dei test: lo stdout del figlio è riservato al protocollo, quindi `main`
//! sceglie il ruolo dalla variabile [`ROLE_VAR`] e nessuna intestazione dei test lo sporca.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use lynkeus_core::Location;
use lynkeus_extract::protocol::{ExtractRequest, FrameReader, Request, Response, encode_frame};
use lynkeus_extract::worker::{self, DEFAULT_MEMORY_LIMIT, WorkerConfig};
use lynkeus_extract::{ExtractEvent, ExtractOptions};

/// Variabile d'ambiente con il ruolo del processo.
const ROLE_VAR: &str = "LYNKEUS_TEST_ROLE";
/// Ruolo: processo estrattore su stdin e stdout.
const WORKER_ROLE: &str = "estrattore";
/// Ruolo (Linux): avvia un estrattore e muore subito.
#[cfg(target_os = "linux")]
const PARENT_ROLE: &str = "padre-intermedio";

/// Attesa massima della risposta a una richiesta (in debug, su una macchina lenta).
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);

fn main() -> ExitCode {
    match std::env::var(ROLE_VAR).as_deref() {
        Ok(WORKER_ROLE) => worker_role(),
        #[cfg(target_os = "linux")]
        Ok(PARENT_ROLE) => linux::parent_role(),
        _ => {
            run(
                "sparse_xlsx_does_not_crash_the_worker",
                sparse_xlsx_does_not_crash_the_worker,
            );
            run(
                "huge_7z_dictionary_fits_the_memory_limit",
                huge_7z_dictionary_fits_the_memory_limit,
            );
            #[cfg(target_os = "linux")]
            run(
                "busy_orphan_exits_with_its_parent",
                linux::busy_orphan_exits_with_its_parent,
            );
            ExitCode::SUCCESS
        }
    }
}

/// Esegue un test; un fallimento è un panic, che termina il programma con errore.
fn run(name: &str, test: fn()) {
    eprintln!("test {name} ...");
    test();
    eprintln!("test {name} ... ok");
}

/// Il processo estrattore, con il limite di memoria di produzione.
fn worker_role() -> ExitCode {
    let config = WorkerConfig {
        memory_limit: Some(DEFAULT_MEMORY_LIMIT),
    };
    match worker::run(io::stdin().lock(), io::stdout().lock(), &config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// Questo eseguibile avviato con il ruolo `role`.
fn this_exe(role: &str) -> Command {
    let exe = std::env::current_exe().expect("eseguibile del test");
    let mut command = Command::new(exe);
    command.env(ROLE_VAR, role);
    command
}

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(relative)
}

/// Un'unità ricevuta: documento (`inner`), posizione, testo.
type Received = (Vec<String>, Location, String);

/// Fa estrarre `relative` (sotto `fixtures/`) a un processo estrattore vero e ne raccoglie
/// le unità; il processo deve rispondere fino in fondo e uscire bene.
fn extract_in_worker(relative: &str) -> Vec<Received> {
    let mut child = this_exe(WORKER_ROLE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("avvio dell'estrattore");
    let request = Request::Extract(ExtractRequest {
        id: 1,
        path: fixture(relative),
        options: ExtractOptions::default(),
    });
    let mut frame = Vec::new();
    encode_frame(&request, &mut frame).expect("richiesta codificata");
    {
        // Lo stdin chiuso dopo la richiesta: l'estrattore esce quando ha finito.
        let mut stdin = child.stdin.take().expect("stdin dell'estrattore");
        io::Write::write_all(&mut stdin, &frame).expect("richiesta inviata");
    }
    let stdout = child.stdout.take().expect("stdout dell'estrattore");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = FrameReader::new(stdout);
        let mut responses = Vec::new();
        while let Ok(Some(response)) = reader.read::<Response>() {
            responses.push(response);
        }
        let _sent = tx.send(responses);
    });
    let responses = rx.recv_timeout(REPLY_TIMEOUT).unwrap_or_else(|_| {
        let _killed = child.kill();
        panic!("{relative}: nessuna risposta completa entro {REPLY_TIMEOUT:?}");
    });
    let status = child.wait().expect("uscita dell'estrattore");
    assert!(
        status.success(),
        "{relative}: estrattore terminato: {status:?}"
    );
    assert_eq!(
        responses.last(),
        Some(&Response::Done { id: 1 }),
        "{relative}"
    );
    let mut inner = Vec::new();
    let mut units = Vec::new();
    for response in responses {
        match response {
            Response::Event {
                event: ExtractEvent::DocumentStart { inner: doc, .. },
                ..
            } => inner = doc,
            Response::Event {
                event: ExtractEvent::Unit(unit),
                ..
            } => units.push((inner.clone(), unit.location, unit.text)),
            _ => {}
        }
    }
    units
}

/// Un 7z che dichiara un dizionario LZMA2 di 2 GiB per una riga di testo: con il limite di
/// memoria di produzione (su Linux nel processo stesso) la voce si legge, perché il
/// dizionario cresce con i dati e non si alloca in anticipo.
fn huge_7z_dictionary_fits_the_memory_limit() {
    let units = extract_in_worker("malevoli/dizionario-enorme.7z");
    assert!(
        units
            .iter()
            .any(|(inner, _, text)| inner == &["nota.txt"] && text.contains("aragosta")),
        "{units:?}"
    );
}

/// `sparso.zip`: `sparso.xlsx` (A1 e XFD1048576; un lettore che riserva righe × colonne
/// chiederebbe 412 GB e il processo abortirebbe) seguito da `nota.txt`. Il processo serve la
/// richiesta fino in fondo ed esce bene; le due celle e la nota arrivano.
fn sparse_xlsx_does_not_crash_the_worker() {
    let units = extract_in_worker("malevoli/sparso.zip");
    let cell = |row, col| Location::Cell {
        sheet: "Sparso".to_owned(),
        row,
        col,
    };
    let xlsx = vec!["sparso.xlsx".to_owned()];
    assert!(
        units.contains(&(xlsx.clone(), cell(1, 1), "aragosta".to_owned())),
        "{units:?}"
    );
    assert!(
        units.contains(&(xlsx, cell(1_048_576, 16_384), "città lontana".to_owned())),
        "{units:?}"
    );
    assert!(
        units
            .iter()
            .any(|(inner, _, text)| inner == &["nota.txt"] && text.contains("aragosta in nota")),
        "{units:?}"
    );
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io::Read;

    use super::*;

    /// Tempo entro cui un estrattore orfano deve uscire.
    const ORPHAN_EXIT: Duration = Duration::from_secs(5);
    /// Richieste lasciate in coda all'estrattore: ognuna legge [`BUSY_FILE_BYTES`] di testo,
    /// e tutte insieme lo terrebbero occupato ben oltre [`ORPHAN_EXIT`]. Stanno nel buffer
    /// del tubo (circa 150 byte l'una), così il padre non aspetta l'estrattore.
    const QUEUED_REQUESTS: u64 = 300;
    /// Dimensione del file di testo che le richieste leggono.
    const BUSY_FILE_BYTES: usize = 32 * 1024 * 1024;
    /// Variabile d'ambiente con il percorso di quel file, per il padre intermedio.
    const BUSY_FILE_VAR: &str = "LYNKEUS_TEST_FILE";

    /// Avvia un estrattore che eredita lo stderr, gli lascia in coda [`QUEUED_REQUESTS`]
    /// richieste lente ed esce subito, lasciandolo orfano e occupato.
    pub fn parent_role() -> ExitCode {
        let Some(busy_file) = std::env::var_os(BUSY_FILE_VAR) else {
            return ExitCode::FAILURE;
        };
        let spawned = this_exe(WORKER_ROLE)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn();
        let Ok(mut orphan) = spawned else {
            return ExitCode::FAILURE;
        };
        let mut frames = Vec::new();
        for id in 0..QUEUED_REQUESTS {
            let request = Request::Extract(ExtractRequest {
                id,
                path: PathBuf::from(&busy_file),
                options: ExtractOptions::default(),
            });
            if encode_frame(&request, &mut frames).is_err() {
                return ExitCode::FAILURE;
            }
        }
        let sent = orphan
            .stdin
            .take()
            .is_some_and(|mut stdin| io::Write::write_all(&mut stdin, &frames).is_ok());
        if sent {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }

    /// Il padre intermedio muore mentre l'estrattore ha in coda lavoro per molti secondi:
    /// l'estrattore si accorge che il padre non c'è più, si ferma ed esce entro
    /// [`ORPHAN_EXIT`]. La sua uscita chiude l'ultima copia della scrittura del tubo che il
    /// test legge al posto dello stderr.
    pub fn busy_orphan_exits_with_its_parent() {
        // Un file di testo grande, nuovo, nella cartella temporanea: righe con "aragosta",
        // così l'estrattore manda eventi di continuo.
        let busy_file =
            std::env::temp_dir().join(format!("lynkeus-orfano-{}.txt", std::process::id()));
        let line = b"aragosta in una riga qualsiasi\n";
        let text: Vec<u8> = line.iter().copied().cycle().take(BUSY_FILE_BYTES).collect();
        {
            let mut file = std::fs::File::create_new(&busy_file).expect("file di prova nuovo");
            io::Write::write_all(&mut file, &text).expect("file di prova scritto");
        }
        let (mut reader, writer) = io::pipe().expect("tubo");
        let mut parent = {
            let mut command = this_exe(PARENT_ROLE);
            command
                .env(BUSY_FILE_VAR, &busy_file)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(writer);
            command.spawn().expect("avvio del padre intermedio")
        };
        let status = parent.wait().expect("uscita del padre intermedio");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut sink = Vec::new();
            let _sent = tx.send(reader.read_to_end(&mut sink).is_ok());
        });
        let exited = rx.recv_timeout(ORPHAN_EXIT);
        let _removed = std::fs::remove_file(&busy_file);
        assert!(status.success(), "padre intermedio: {status:?}");
        assert_eq!(
            exited,
            Ok(true),
            "l'estrattore orfano non è uscito entro {ORPHAN_EXIT:?}"
        );
    }
}
