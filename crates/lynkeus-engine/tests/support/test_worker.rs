// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Processo estrattore per i test d'integrazione del motore.
//!
//! Avviato come l'app (`--extract-worker [--memory-limit-bytes N]`), legge le richieste come
//! `lynkeus_extract::worker::run` e per i file normali fa la stessa cosa (`extract_path`). Per
//! i file con un nome riservato si comporta male di proposito, per provare il pool:
//!
//! - `lento.*`: non risponde più (dorme a lungo, ignorando stdin);
//! - `muore.*`: inizia un documento ed esce con codice 3;
//! - `sporco.*`: scrive su stdout byte estranei al protocollo;
//! - `fuori-sequenza.*`: manda un'unità senza inizio documento, poi la fine;
//! - `altro-id.*`: risponde con un id diverso da quello della richiesta;
//! - `descrittori.*`: risponde con un'unità che elenca i descrittori aperti del processo
//!   (`/proc/self/fd`, solo Linux; altrove un testo che lo dice);
//! - `lucchetto.*`: apre il file, ne prende il lock esclusivo e lo tiene fino all'uscita del
//!   processo (il test sa così se il processo è ancora vivo), poi risponde come un documento
//!   vuoto.
//!
//! Con [`ORPHAN_PARENT_FLAG`] fa invece da padre intermedio: avvia un processo estrattore con
//! il lanciatore portabile, gli fa prendere il lock del file indicato, scrive `pronto` su
//! stdout e dorme; il test lo uccide e controlla che il figlio rimasto senza padre esca.
//!
//! Non fa parte del pacchetto distribuito.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::num::NonZeroUsize;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use lynkeus_core::{Location, TextUnit};
use lynkeus_engine::{CommandLauncher, ExtractEnd, ExtractorPool};
use lynkeus_extract::protocol::{
    EntryRequest, ExtractRequest, FrameReader, FrameWriter, ProtocolError, Request, RequestId,
    Response,
};
use lynkeus_extract::worker::WorkerConfig;
use lynkeus_extract::{
    EntrySink, EventSink, ExtractEvent, ExtractOptions, Format, Stopped, extract_entry,
    extract_path,
};

/// Opzione che avvia il padre intermedio del test sugli orfani, seguita dal file da bloccare.
const ORPHAN_PARENT_FLAG: &str = "--orphan-parent";

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first == Some(OsString::from(ORPHAN_PARENT_FLAG)) {
        return match args.next() {
            Some(lock) => orphan_parent(Path::new(&lock)),
            None => ExitCode::from(2),
        };
    }
    if first != Some(OsString::from("--extract-worker")) {
        return ExitCode::from(2);
    }
    if WorkerConfig::from_args(args).is_err() {
        return ExitCode::from(2);
    }
    let stdin = io::stdin().lock();
    let stdout = io::stdout().lock();
    match serve(stdin, stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

fn serve(input: impl io::Read, output: impl Write) -> Result<(), ProtocolError> {
    let mut reader = FrameReader::new(BufReader::new(input));
    let mut writer = FrameWriter::new(BufWriter::new(output));
    // I file bloccati da `lucchetto.*`: restano aperti fino all'uscita del processo.
    let mut locked: Vec<File> = Vec::new();
    while let Some(request) = reader.read::<Request>()? {
        let request = match request {
            Request::Extract(request) => request,
            Request::ExtractEntry(request) => {
                entry(&mut writer, &request)?;
                continue;
            }
        };
        let stem = request
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        match stem.as_str() {
            "lento" => std::thread::sleep(Duration::from_secs(3600)),
            "muore" => {
                writer.write(&event(request.id, start()))?;
                writer.flush()?;
                std::process::exit(3);
            }
            "sporco" => {
                writer.flush()?;
                let mut out = writer.into_inner();
                out.write_all(b"ciao, sono un messaggio di log\n")
                    .and_then(|()| out.flush())
                    .map_err(ProtocolError::Io)?;
                std::thread::sleep(Duration::from_secs(3600));
                return Ok(());
            }
            "fuori-sequenza" => {
                writer.write(&event(request.id, unit("carta")))?;
                writer.write(&Response::Done { id: request.id })?;
                writer.flush()?;
            }
            "altro-id" => {
                writer.write(&event(request.id.wrapping_add(7), start()))?;
                writer.flush()?;
            }
            "descrittori" => {
                writer.write(&event(request.id, start()))?;
                writer.write(&event(request.id, unit(&open_descriptors())))?;
                writer.write(&event(request.id, ExtractEvent::DocumentEnd))?;
                writer.write(&Response::Done { id: request.id })?;
                writer.flush()?;
            }
            "lucchetto" => {
                let file = File::options()
                    .read(true)
                    .write(true)
                    .open(&request.path)
                    .map_err(ProtocolError::Io)?;
                file.lock().map_err(ProtocolError::Io)?;
                locked.push(file);
                writer.write(&event(request.id, start()))?;
                writer.write(&event(request.id, ExtractEvent::DocumentEnd))?;
                writer.write(&Response::Done { id: request.id })?;
                writer.flush()?;
            }
            _ => normal(&mut writer, &request)?,
        }
    }
    Ok(())
}

/// I descrittori aperti, in ordine e separati da spazi, senza quello della cartella che si
/// sta leggendo.
#[cfg(target_os = "linux")]
fn open_descriptors() -> String {
    let Ok(listing) = std::fs::read_dir("/proc/self/fd") else {
        return "/proc/self/fd illeggibile".to_owned();
    };
    let mut fds: Vec<u32> = listing
        .flatten()
        .filter(|item| {
            // Il descrittore di `read_dir` punta a `/proc/<pid>/fd`.
            std::fs::read_link(item.path()).is_ok_and(|target| !target.ends_with("fd"))
        })
        .filter_map(|item| item.file_name().to_str()?.parse().ok())
        .collect();
    fds.sort_unstable();
    let names: Vec<String> = fds.iter().map(u32::to_string).collect();
    names.join(" ")
}

/// I descrittori aperti: solo Linux.
#[cfg(not(target_os = "linux"))]
fn open_descriptors() -> String {
    "descrittori non elencabili su questo sistema".to_owned()
}

/// Il padre intermedio: avvia un processo estrattore che blocca `lock`, dice `pronto` e
/// aspetta di essere ucciso.
fn orphan_parent(lock: &Path) -> ExitCode {
    let Ok(exe) = std::env::current_exe() else {
        return ExitCode::FAILURE;
    };
    let Ok(launcher) = CommandLauncher::new(exe, None) else {
        return ExitCode::FAILURE;
    };
    let pool = ExtractorPool::new(Arc::new(launcher), NonZeroUsize::MIN);
    let never = || false;
    let mut events: Vec<ExtractEvent> = Vec::new();
    let end = pool.extract(
        lock,
        &ExtractOptions::default(),
        Duration::from_secs(30),
        &never,
        &mut events,
    );
    if end != ExtractEnd::Completed {
        return ExitCode::FAILURE;
    }
    let mut out = io::stdout().lock();
    if out
        .write_all(
            b"pronto
",
        )
        .and_then(|()| out.flush())
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    // Il test uccide questo processo; il pool (e il figlio) non si chiudono da qui.
    std::thread::sleep(Duration::from_secs(3600));
    ExitCode::FAILURE
}

fn normal<W: Write>(
    writer: &mut FrameWriter<W>,
    request: &ExtractRequest,
) -> Result<(), ProtocolError> {
    let mut sink = Sink {
        writer: &mut *writer,
        id: request.id,
        error: None,
    };
    let _ = extract_path(&request.path, &request.options, &mut sink);
    if let Some(error) = sink.error {
        return Err(error);
    }
    writer.write(&Response::Done { id: request.id })?;
    writer.flush()
}

/// I byte di una voce, come il processo estrattore vero.
fn entry<W: Write>(
    writer: &mut FrameWriter<W>,
    request: &EntryRequest,
) -> Result<(), ProtocolError> {
    let mut sink = Chunks {
        writer: &mut *writer,
        id: request.id,
        error: None,
    };
    let outcome = extract_entry(
        &request.path,
        &request.inner,
        request.max_bytes,
        &request.options,
        &mut sink,
    );
    if let Some(error) = sink.error {
        return Err(error);
    }
    let Ok(outcome) = outcome else {
        return Err(ProtocolError::Malformed);
    };
    writer.write(&Response::EntryEnd {
        id: request.id,
        outcome,
    })?;
    writer.flush()
}

struct Chunks<'w, W: Write> {
    writer: &'w mut FrameWriter<W>,
    id: RequestId,
    error: Option<ProtocolError>,
}

impl<W: Write> EntrySink for Chunks<'_, W> {
    fn bytes(&mut self, chunk: &[u8]) -> Result<(), Stopped> {
        self.writer
            .write(&Response::EntryData {
                id: self.id,
                bytes: chunk.to_vec(),
            })
            .map_err(|error| {
                self.error = Some(error);
                Stopped
            })
    }
}

struct Sink<'w, W: Write> {
    writer: &'w mut FrameWriter<W>,
    id: RequestId,
    error: Option<ProtocolError>,
}

impl<W: Write> EventSink for Sink<'_, W> {
    fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped> {
        self.writer
            .write(&Response::Event { id: self.id, event })
            .map_err(|error| {
                self.error = Some(error);
                Stopped
            })
    }
}

fn event(id: RequestId, event: ExtractEvent) -> Response {
    Response::Event { id, event }
}

fn start() -> ExtractEvent {
    ExtractEvent::DocumentStart {
        inner: Vec::new(),
        format: Format::Pdf,
    }
}

fn unit(text: &str) -> ExtractEvent {
    ExtractEvent::Unit(TextUnit {
        location: Location::PageLine { page: 1, line: 1 },
        text: text.to_owned(),
    })
}
