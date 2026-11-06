// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Processo estrattore per i test d'integrazione del motore.
//!
//! Avviato come l'app (`--extract-worker [--memory-limit-bytes N]`), legge le richieste come
//! `lynkeus_extract::worker::run` e per i file normali fa la stessa cosa (`extract_path`). Per
//! i file con un nome riservato si comporta male di proposito, per provare il pool:
//!
//! - `lento.*`: inizia un documento (così chi chiama sa che la richiesta è arrivata) e non
//!   risponde più (dorme a lungo, ignorando stdin);
//! - `muore.*`: inizia un documento ed esce con codice 3;
//! - `sporco.*`: scrive su stdout byte estranei al protocollo;
//! - `fuori-sequenza.*`: manda un'unità senza inizio documento, poi la fine;
//! - `altro-id.*`: risponde con un id diverso (successivo) da quello della richiesta;
//! - `descrittori.*`: risponde con un'unità che elenca i descrittori aperti del processo
//!   (`/proc/self/fd`, solo Linux; altrove un testo che lo dice);
//! - `lucchetto.*`: apre il file, ne prende il lock esclusivo e lo tiene fino all'uscita del
//!   processo (il test sa così se il processo è ancora vivo), poi risponde come un documento
//!   vuoto;
//! - `lucchetto-occupato.*`: come `lucchetto`, ma dopo l'inizio del documento gira a vuoto
//!   sulla CPU per sempre, senza leggere stdin;
//! - `unita-grande.*`: un documento con un'unità di 1 MiB e 64 KiB (oltre il massimo di
//!   un'unità, dentro un frame valido);
//! - `frame-enorme.*`: l'intestazione di un frame che dichiara 16 MiB (oltre il massimo del
//!   protocollo), poi non risponde più;
//! - `troppo-profondo.*`: un avviso con più voci annidate della profondità massima;
//! - `inonda.*`: un documento con unità valide senza fine;
//! - `chiacchierone.*`: un documento vuoto, poi un secondo `Done` non richiesto;
//! - `dati-fuori-sequenza.*`: risponde a un'estrazione con i byte di una voce;
//! - `finestra.*`: un documento con cinque righe, poi non risponde più;
//! - `molti.*`: [`MANY_DOCUMENTS`] voci, ciascuna con una riga "carta".
//!
//! Con [`DEAF_FLAG`] (dopo `--extract-worker`) non legge mai stdin e dorme: prova le
//! scritture verso un processo che non legge.
//!
//! Con [`ORPHAN_PARENT_FLAG`] fa invece da padre intermedio: avvia un processo estrattore con
//! il lanciatore portabile, gli fa prendere il lock del file indicato, scrive `pronto` su
//! stdout e dorme; il test lo uccide e controlla che il figlio rimasto senza padre esca. Con
//! [`BUSY_ORPHAN_PARENT_FLAG`] il figlio, dopo il lock, resta occupato (`lucchetto-occupato`).
//!
//! Come il processo estrattore vero, all'avvio sorveglia il padre con
//! `lynkeus_extract::worker::watch_parent`, con il pid ricevuto dal lanciatore, ed esce
//! quando il padre non c'è più (Linux; su Windows lo chiude il Job Object o il pool): anche
//! chi dorme o gira a vuoto senza controllare nulla, come un decodificatore occupato. Così
//! nessun processo di prova resta orfano.
//!
//! Non fa parte del pacchetto distribuito.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::num::NonZeroUsize;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lynkeus_core::{Location, TextUnit};
use lynkeus_engine::{CommandLauncher, ExtractEnd, ExtractorPool};
use lynkeus_extract::protocol::{
    EntryRequest, ExtractRequest, FrameReader, FrameWriter, ProtocolError, Request, RequestId,
    Response,
};
use lynkeus_extract::worker::{self, WorkerConfig, watch_parent};
use lynkeus_extract::{
    EntrySink, EventSink, ExtractEvent, ExtractOptions, Format, Stopped, WarningKind,
    extract_entry, extract_path,
};

/// Opzione che avvia il padre intermedio del test sugli orfani, seguita dal file da bloccare.
const ORPHAN_PARENT_FLAG: &str = "--orphan-parent";

/// Come [`ORPHAN_PARENT_FLAG`], con il figlio occupato dopo il lock.
const BUSY_ORPHAN_PARENT_FLAG: &str = "--orphan-parent-busy";

/// Opzione (subito dopo `--extract-worker`) di un processo che non legge mai stdin.
const DEAF_FLAG: &str = "--deaf";

/// Voci del file `molti.*`.
const MANY_DOCUMENTS: usize = 400;

/// Byte dell'unità di `unita-grande.*`: oltre il massimo di un'unità (1 MiB), dentro il
/// massimo di un frame.
const OVERSIZED_UNIT_BYTES: usize = 1024 * 1024 + 64 * 1024;

/// Lunghezza dichiarata dal frame di `frame-enorme.*`: il vecchio massimo del protocollo.
const HUGE_FRAME_BYTES: u32 = 16 * 1024 * 1024;

/// Byte di ogni unità di `inonda.*`.
const FLOOD_UNIT_BYTES: usize = 64 * 1024;

/// Un'attesa che il test non vede mai finire. Anche il limite di vita di un processo che
/// dorme o gira a vuoto: su Windows `watch_parent` non fa nulla (lì i processi li chiude il
/// pool), e un processo di prova rimasto senza padre esce comunque.
const FOREVER: Duration = Duration::from_secs(3600);

/// Codice d'uscita di un processo rimasto senza padre, o che non riesce a sorvegliarlo.
const ORPHAN_EXIT: i32 = 4;

fn main() -> ExitCode {
    // Come il `main` dell'app con `--extract-worker`, per prima cosa.
    // SAFETY: inizio del `main`, un solo thread, nessun file aperto dal programma.
    #[allow(unsafe_code, reason = "contratto di close_inherited_descriptors")]
    let closed = unsafe { worker::close_inherited_descriptors() };
    if closed.is_err() {
        return ExitCode::FAILURE;
    }
    let mut args = std::env::args_os().skip(1).peekable();
    let first = args.next();
    if first == Some(OsString::from(ORPHAN_PARENT_FLAG))
        || first == Some(OsString::from(BUSY_ORPHAN_PARENT_FLAG))
    {
        let busy = first == Some(OsString::from(BUSY_ORPHAN_PARENT_FLAG));
        return match args.next() {
            Some(lock) => orphan_parent(Path::new(&lock), busy),
            None => ExitCode::from(2),
        };
    }
    if first != Some(OsString::from("--extract-worker")) {
        return ExitCode::from(2);
    }
    let deaf = args.peek() == Some(&OsString::from(DEAF_FLAG));
    if deaf {
        args.next();
    }
    let Ok(config) = WorkerConfig::from_args(args) else {
        return ExitCode::from(2);
    };
    if watch_parent(config.parent_pid, || std::process::exit(ORPHAN_EXIT)).is_err() {
        std::process::exit(ORPHAN_EXIT);
    }
    if deaf {
        idle();
    }
    let stdin = io::stdin().lock();
    let stdout = io::stdout().lock();
    match serve(stdin, stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// Dorme (per il test, per sempre; al più [`FOREVER`]), poi esce: un processo che non
/// risponde più. Se il padre muore lo termina la sorveglianza.
fn idle() -> ! {
    std::thread::sleep(FOREVER);
    std::process::exit(ORPHAN_EXIT);
}

/// Gira a vuoto sulla CPU senza leggere stdin e senza controllare nulla (al più
/// [`FOREVER`]), come un decodificatore occupato: se il padre muore lo termina la
/// sorveglianza.
fn spin() -> ! {
    let deadline = Instant::now() + FOREVER;
    let mut counter = 0_u64;
    while Instant::now() < deadline {
        counter = std::hint::black_box(counter.wrapping_add(1));
    }
    std::process::exit(ORPHAN_EXIT);
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
        let id = request.id;
        match stem.as_str() {
            "lento" => {
                writer.write(&event(id, start()))?;
                writer.flush()?;
                idle();
            }
            "muore" => {
                writer.write(&event(id, start()))?;
                writer.flush()?;
                std::process::exit(3);
            }
            "sporco" => {
                writer.flush()?;
                let mut out = writer.into_inner();
                out.write_all(b"ciao, sono un messaggio di log\n")
                    .and_then(|()| out.flush())
                    .map_err(ProtocolError::Io)?;
                idle();
            }
            "fuori-sequenza" => {
                writer.write(&event(id, unit(1, "carta")))?;
                writer.write(&Response::Done { id })?;
                writer.flush()?;
            }
            "altro-id" => {
                writer.write(&event(id.wrapping_add(7), start()))?;
                writer.flush()?;
            }
            "descrittori" => {
                writer.write(&event(id, start()))?;
                writer.write(&event(id, unit(1, &open_descriptors())))?;
                finish(&mut writer, id)?;
            }
            "lucchetto" | "lucchetto-occupato" => {
                let file = File::options()
                    .read(true)
                    .write(true)
                    .open(&request.path)
                    .map_err(ProtocolError::Io)?;
                file.lock().map_err(ProtocolError::Io)?;
                locked.push(file);
                writer.write(&event(id, start()))?;
                if stem == "lucchetto-occupato" {
                    writer.flush()?;
                    spin();
                }
                finish(&mut writer, id)?;
            }
            "frame-enorme" => {
                writer.flush()?;
                let mut out = writer.into_inner();
                let mut frame = HUGE_FRAME_BYTES.to_le_bytes().to_vec();
                frame.extend_from_slice(&[0; 64 * 1024]);
                out.write_all(&frame)
                    .and_then(|()| out.flush())
                    .map_err(ProtocolError::Io)?;
                idle();
            }
            "unita-grande" => {
                writer.write(&event(id, start()))?;
                let text = "carta ".repeat(OVERSIZED_UNIT_BYTES / 6);
                writer.write(&event(id, unit(1, &text)))?;
                finish(&mut writer, id)?;
            }
            "troppo-profondo" => {
                let depth = ExtractOptions::default().limits.max_depth;
                let inner = (0..=depth).map(|i| format!("livello{i}.zip")).collect();
                writer.write(&event(
                    id,
                    ExtractEvent::Warning {
                        inner,
                        kind: WarningKind::Corrupt,
                    },
                ))?;
                writer.write(&Response::Done { id })?;
                writer.flush()?;
            }
            "inonda" => {
                writer.write(&event(id, start()))?;
                let text = "aragosta ".repeat(FLOOD_UNIT_BYTES / 9);
                for line in 1.. {
                    writer.write(&event(id, unit(line, &text)))?;
                }
            }
            "chiacchierone" => {
                writer.write(&event(id, start()))?;
                writer.write(&event(id, ExtractEvent::DocumentEnd))?;
                writer.write(&Response::Done { id })?;
                writer.write(&Response::Done { id })?;
                writer.flush()?;
            }
            "dati-fuori-sequenza" => {
                writer.write(&Response::EntryData {
                    id,
                    bytes: b"carta".to_vec(),
                })?;
                writer.write(&Response::Done { id })?;
                writer.flush()?;
            }
            "finestra" => {
                writer.write(&event(id, start()))?;
                for line in 1..=5 {
                    writer.write(&event(id, unit(line, &format!("riga {line} carta"))))?;
                }
                writer.flush()?;
                idle();
            }
            "molti" => {
                for n in 0..MANY_DOCUMENTS {
                    writer.write(&event(
                        id,
                        ExtractEvent::DocumentStart {
                            inner: vec![format!("voce{n}.txt")],
                            format: Format::Text,
                        },
                    ))?;
                    writer.write(&event(id, unit(1, "carta")))?;
                    writer.write(&event(id, ExtractEvent::DocumentEnd))?;
                }
                writer.write(&Response::Done { id })?;
                writer.flush()?;
            }
            _ => normal(&mut writer, &request)?,
        }
    }
    Ok(())
}

/// Chiude il documento e la richiesta `id`.
fn finish<W: Write>(writer: &mut FrameWriter<W>, id: RequestId) -> Result<(), ProtocolError> {
    writer.write(&event(id, ExtractEvent::DocumentEnd))?;
    writer.write(&Response::Done { id })?;
    writer.flush()
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

/// Dice `pronto` su stdout al primo evento.
struct ReadySignal;

impl EventSink for ReadySignal {
    fn emit(&mut self, _event: ExtractEvent) -> Result<(), Stopped> {
        let mut out = io::stdout().lock();
        // Senza `pronto` il test fallisce per scadenza: un errore qui non ha altro rimedio.
        let _said = out.write_all(b"pronto\n").and_then(|()| out.flush());
        Ok(())
    }
}

/// Il padre intermedio: avvia un processo estrattore che blocca `lock` (e con `busy` resta
/// occupato), dice `pronto` e aspetta di essere ucciso.
fn orphan_parent(lock: &Path, busy: bool) -> ExitCode {
    let Ok(exe) = std::env::current_exe() else {
        return ExitCode::FAILURE;
    };
    let Ok(launcher) = CommandLauncher::new(exe, None) else {
        return ExitCode::FAILURE;
    };
    let pool = ExtractorPool::new(Arc::new(launcher), NonZeroUsize::MIN);
    let never = || false;
    if busy {
        // L'estrazione non finisce mai: `pronto` parte dall'inizio del documento.
        let _never_ends = pool.extract(
            lock,
            &ExtractOptions::default(),
            FOREVER,
            &never,
            &mut ReadySignal,
        );
        return ExitCode::FAILURE;
    }
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
        .write_all(b"pronto\n")
        .and_then(|()| out.flush())
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    // Il test uccide questo processo; il pool (e il figlio) non si chiudono da qui.
    std::thread::sleep(FOREVER);
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

fn unit(line: u32, text: &str) -> ExtractEvent {
    ExtractEvent::Unit(TextUnit {
        location: Location::PageLine { page: 1, line },
        text: text.to_owned(),
    })
}
