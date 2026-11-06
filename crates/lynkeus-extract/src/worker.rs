// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Ciclo del processo estrattore: legge richieste da stdin e scrive risposte su stdout, nel
//! [`protocol`](crate::protocol).
//!
//! L'app avvia lo stesso eseguibile con l'opzione nascosta `--extract-worker` seguita da
//! [`WorkerConfig::to_args`] (limite di memoria e pid del padre), e nel figlio chiama [`run`]
//! con stdin, stdout e ciò che va fatto se il padre muore (l'app termina il processo). Il
//! ciclo:
//!
//! 1. all'avvio, prima di leggere qualunque file, avvia la sorveglianza del padre
//!    ([`watch_parent`], con il pid ricevuto) e imposta il limite di memoria (`RLIMIT_AS` su
//!    Linux; su Windows lo impone il Job Object del padre). Senza limite configurato, o se
//!    una delle due cose non riesce, esce con errore: un estrattore senza limite non parte;
//! 2. per ogni [`Request::Extract`] chiama [`extract_path`] e manda gli eventi man mano
//!    (il buffer si svuota a ogni fine documento e alla fine della richiesta); per ogni
//!    [`Request::ExtractEntry`] chiama [`extract_entry`] e manda i byte della voce a pezzi,
//!    poi l'esito;
//! 3. esce con `Ok` quando stdin finisce tra una richiesta e l'altra (il padre lo chiude per
//!    fermarlo), con errore su un frame non valido (dopo aver mandato
//!    [`Response::Rejected`]), se stdout non si scrive più o se il padre è terminato. Quando
//!    il padre termina la sorveglianza chiama la funzione del chiamante, che nell'app
//!    termina subito il processo anche se un decodificatore è occupato; se la funzione
//!    torna, l'estrazione in corso si ferma al primo evento e le richieste in coda non si
//!    servono. Così un estrattore non resta orfano (Linux; su Windows il Job Object del
//!    padre lo termina).
//!
//! Il processo legge solo i file richiesti, non scrive su disco, non avvia programmi e non
//! usa la rete. Stdout è riservato al protocollo: niente `print!` in questo crate (i log, se
//! servono, vanno su stderr).

use std::ffi::OsString;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lynkeus_core::ByteSize;

use crate::event::{EventSink, ExtractEvent, Stopped};
use crate::extract::{EntrySink, extract_entry, extract_path};
use crate::platform;
use crate::protocol::{
    EntryRequest, ExtractRequest, FrameReader, FrameWriter, ProtocolError, Request, RequestId,
    Response,
};

pub use crate::platform::MemoryLimitOwner;

/// Opzione che porta il limite di memoria, in byte.
pub const MEMORY_LIMIT_FLAG: &str = "--memory-limit-bytes";

/// Limite di memoria proposto per un processo estrattore (1 GiB): un PDF da 200 MiB ne
/// occupa circa il doppio (la libreria copia i byte), più le strutture.
pub const DEFAULT_MEMORY_LIMIT: ByteSize = ByteSize::from_gib(1);

/// Limite minimo accettato: sotto, il processo non riuscirebbe nemmeno a partire.
pub const MIN_MEMORY_LIMIT: ByteSize = ByteSize::from_mib(64);

/// Opzione che porta il pid del processo principale.
pub const PARENT_PID_FLAG: &str = "--parent-pid";

/// Configurazione del processo estrattore, passata come argomenti dal padre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkerConfig {
    /// Limite di memoria che il processo impone a sé stesso dove può (Linux). Obbligatorio
    /// per [`run`]: con `None` il processo non parte. `None` resta possibile solo per chi
    /// avvia un processo di prova che non chiama [`run`].
    pub memory_limit: Option<ByteSize>,
    /// Pid del processo principale, che lo passa per sé: il figlio riconosce così la sua
    /// morte anche se avviene prima che inizi a sorvegliarlo. Con `None` vale il padre letto
    /// all'avvio della sorveglianza (un "processo" di prova nello stesso processo).
    pub parent_pid: Option<u32>,
}

/// Argomenti del processo estrattore non validi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ArgsError {
    /// Argomento sconosciuto.
    #[error("argomento sconosciuto")]
    Unknown,
    /// Opzione senza valore.
    #[error("valore mancante dopo {MEMORY_LIMIT_FLAG}")]
    MissingValue,
    /// Valore non numerico, limite sotto [`MIN_MEMORY_LIMIT`] o pid zero.
    #[error("valore non valido")]
    InvalidValue,
}

impl WorkerConfig {
    /// Gli argomenti da passare al figlio dopo `--extract-worker`.
    pub fn to_args(&self) -> Vec<OsString> {
        let mut args = Vec::new();
        if let Some(limit) = self.memory_limit {
            args.push(OsString::from(MEMORY_LIMIT_FLAG));
            args.push(OsString::from(limit.bytes().to_string()));
        }
        if let Some(pid) = self.parent_pid {
            args.push(OsString::from(PARENT_PID_FLAG));
            args.push(OsString::from(pid.to_string()));
        }
        args
    }

    /// Legge gli argomenti che seguono `--extract-worker` (quelli di [`WorkerConfig::to_args`]).
    /// Senza [`MEMORY_LIMIT_FLAG`] il limite resta `None`, e [`run`] non parte.
    pub fn from_args<I: IntoIterator<Item = OsString>>(args: I) -> Result<WorkerConfig, ArgsError> {
        let mut config = WorkerConfig::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let flag = if arg == MEMORY_LIMIT_FLAG {
                MEMORY_LIMIT_FLAG
            } else if arg == PARENT_PID_FLAG {
                PARENT_PID_FLAG
            } else {
                return Err(ArgsError::Unknown);
            };
            let value = args.next().ok_or(ArgsError::MissingValue)?;
            let number = value.to_str().and_then(|v| v.parse::<u64>().ok());
            if flag == MEMORY_LIMIT_FLAG {
                let bytes = number
                    .filter(|b| *b >= MIN_MEMORY_LIMIT.bytes())
                    .ok_or(ArgsError::InvalidValue)?;
                config.memory_limit = Some(ByteSize::from_bytes(bytes));
            } else {
                let pid = number
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|pid| *pid > 0)
                    .ok_or(ArgsError::InvalidValue)?;
                config.parent_pid = Some(pid);
            }
        }
        Ok(config)
    }
}

/// Perché il processo estrattore si è fermato con errore.
#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    /// Il limite di memoria manca nella configurazione o non si è potuto impostare.
    #[error("limite di memoria non impostato")]
    MemoryLimit(#[source] io::Error),
    /// I descrittori ereditati dal processo padre non si sono potuti elencare.
    #[error("descrittori ereditati non chiusi")]
    InheritedDescriptors(#[source] io::Error),
    /// La sorveglianza del processo principale non è partita.
    #[error("sorveglianza del processo principale non avviata")]
    ParentWatch(#[source] io::Error),
    /// Il processo principale è terminato: le richieste non si servono più.
    #[error("processo principale terminato")]
    ParentGone,
    /// Frame non valido in ingresso o stdout non scrivibile.
    #[error("errore del protocollo")]
    Protocol(#[source] ProtocolError),
}

/// Chiama `on_gone` quando il processo padre `expected` (il pid ricevuto con
/// [`PARENT_PID_FLAG`]; senza, il padre letto all'avvio) termina, anche se è terminato prima
/// della chiamata (Linux; su Windows non fa nulla, il Job Object del padre termina
/// l'estrattore). `on_gone` gira in un thread di sorveglianza e può terminare il processo.
/// [`run`] la chiama da sé; serve a parte solo a un processo di prova che non usa [`run`].
///
/// Errore se la sorveglianza non parte.
pub fn watch_parent(
    expected: Option<u32>,
    on_gone: impl FnOnce() + Send + 'static,
) -> io::Result<()> {
    platform::watch_parent(expected, on_gone)
}

/// Chiude i descrittori ereditati oltre stdin, stdout e stderr (Linux; altrove non fa nulla).
/// [`run`] la chiama da sé, per prima cosa; serve a parte solo a un processo di prova che non
/// usa [`run`], all'avvio, prima di aprire file o avviare thread.
///
/// Errore se i descrittori aperti non si leggono.
pub fn close_inherited_descriptors() -> io::Result<()> {
    platform::close_inherited_descriptors()
}

/// Il ciclo del processo estrattore su `input` (stdin) e `output` (stdout).
///
/// Prima di leggere qualunque richiesta chiude i descrittori ereditati, avvia la sorveglianza del padre di `config` e impone
/// il limite di memoria di `config`, obbligatorio: senza, [`WorkerError::MemoryLimit`] e
/// nessuna richiesta servita. Quando il padre termina chiama `on_parent_gone` (l'app termina
/// il processo: in questo crate nessun modulo gestisce i processi); se torna, l'estrazione in
/// corso si ferma al primo evento. Restituisce `Ok` quando `input` finisce tra una richiesta
/// e l'altra, [`WorkerError::ParentGone`] se il padre termina prima.
pub fn run<R: Read, W: Write>(
    input: R,
    output: W,
    config: &WorkerConfig,
    on_parent_gone: impl FnOnce() + Send + 'static,
) -> Result<(), WorkerError> {
    let Some(limit) = config.memory_limit else {
        return Err(WorkerError::MemoryLimit(io::Error::new(
            io::ErrorKind::InvalidInput,
            "limite di memoria mancante",
        )));
    };
    platform::close_inherited_descriptors().map_err(WorkerError::InheritedDescriptors)?;
    let parent_gone = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&parent_gone);
    watch_parent(config.parent_pid, move || {
        signal.store(true, Ordering::Relaxed);
        on_parent_gone();
    })
    .map_err(WorkerError::ParentWatch)?;
    platform::limit_address_space(limit.bytes()).map_err(WorkerError::MemoryLimit)?;
    serve_requests(input, output, &parent_gone)
}

/// Serve le richieste di `input` finché finisce tra una richiesta e l'altra, o finché
/// `parent_gone` diventa vero.
fn serve_requests<R: Read, W: Write>(
    input: R,
    output: W,
    parent_gone: &AtomicBool,
) -> Result<(), WorkerError> {
    let mut reader = FrameReader::new(BufReader::new(input));
    let mut writer = FrameWriter::new(BufWriter::new(output));
    loop {
        let request = reader.read::<Request>();
        if parent_gone.load(Ordering::Relaxed) {
            return Err(WorkerError::ParentGone);
        }
        match request {
            Ok(None) => return Ok(()),
            Ok(Some(Request::Extract(request))) => serve(&mut writer, &request, parent_gone)?,
            Ok(Some(Request::ExtractEntry(request))) => {
                serve_entry(&mut writer, &request, parent_gone)?;
            }
            Err(error) => {
                if let Some(reason) = error.reject_reason() {
                    // Il padre vede comunque l'uscita: se nemmeno il rifiuto si scrive,
                    // conta l'errore originale.
                    let sent = writer
                        .write(&Response::Rejected { reason })
                        .and_then(|()| writer.flush());
                    drop(sent);
                }
                return Err(WorkerError::Protocol(error));
            }
        }
    }
}

/// Perché un destinatario ha fermato l'estrazione.
#[derive(Debug)]
enum Halt {
    /// Stdout non si scrive più.
    Protocol(ProtocolError),
    /// Il processo principale è terminato.
    ParentGone,
}

impl From<Halt> for WorkerError {
    fn from(halt: Halt) -> WorkerError {
        match halt {
            Halt::Protocol(error) => WorkerError::Protocol(error),
            Halt::ParentGone => WorkerError::ParentGone,
        }
    }
}

/// Dove un destinatario scrive le risposte di una richiesta: lo scrittore, l'identificativo
/// della richiesta, il segnale del padre terminato e il motivo dell'arresto.
struct Outbox<'w, W: Write> {
    writer: &'w mut FrameWriter<W>,
    id: RequestId,
    parent_gone: &'w AtomicBool,
    halt: Option<Halt>,
}

impl<W: Write> Outbox<'_, W> {
    /// Scrive `response` (e svuota il buffer se `flush`), salvo che il padre sia
    /// terminato: allora, o se la scrittura fallisce, registra il motivo e ferma.
    fn send(&mut self, response: &Response, flush: bool) -> Result<(), Stopped> {
        if self.parent_gone.load(Ordering::Relaxed) {
            self.halt = Some(Halt::ParentGone);
            return Err(Stopped);
        }
        let written = self
            .writer
            .write(response)
            .and_then(|()| if flush { self.writer.flush() } else { Ok(()) });
        written.map_err(|error| {
            self.halt = Some(Halt::Protocol(error));
            Stopped
        })
    }
}

/// Esegue una richiesta e manda le risposte, chiudendo con [`Response::Done`].
fn serve<W: Write>(
    writer: &mut FrameWriter<W>,
    request: &ExtractRequest,
    parent_gone: &AtomicBool,
) -> Result<(), WorkerError> {
    let mut outbox = Outbox {
        writer: &mut *writer,
        id: request.id,
        parent_gone,
        halt: None,
    };
    let outcome = extract_path(&request.path, &request.options, &mut outbox);
    if let Some(halt) = outbox.halt {
        return Err(halt.into());
    }
    debug_assert!(outcome.is_ok(), "solo questo destinatario può fermare");
    writer
        .write(&Response::Done { id: request.id })
        .and_then(|()| writer.flush())
        .map_err(WorkerError::Protocol)
}

/// Esegue una richiesta di voce: i byte come [`Response::EntryData`], poi
/// [`Response::EntryEnd`].
fn serve_entry<W: Write>(
    writer: &mut FrameWriter<W>,
    request: &EntryRequest,
    parent_gone: &AtomicBool,
) -> Result<(), WorkerError> {
    let mut outbox = Outbox {
        writer: &mut *writer,
        id: request.id,
        parent_gone,
        halt: None,
    };
    let outcome = extract_entry(
        &request.path,
        &request.inner,
        request.max_bytes,
        &request.options,
        &mut outbox,
    );
    if let Some(halt) = outbox.halt {
        return Err(halt.into());
    }
    let Ok(outcome) = outcome else {
        // Solo questo destinatario può fermare, e lo fa solo con un motivo già registrato.
        return Err(WorkerError::Protocol(ProtocolError::Malformed));
    };
    writer
        .write(&Response::EntryEnd {
            id: request.id,
            outcome,
        })
        .and_then(|()| writer.flush())
        .map_err(WorkerError::Protocol)
}

/// Ogni pezzo di una voce come [`Response::EntryData`].
impl<W: Write> EntrySink for Outbox<'_, W> {
    fn bytes(&mut self, chunk: &[u8]) -> Result<(), Stopped> {
        let response = Response::EntryData {
            id: self.id,
            bytes: chunk.to_vec(),
        };
        self.send(&response, false)
    }
}

/// Ogni evento come [`Response::Event`]; il buffer si svuota a ogni fine documento.
impl<W: Write> EventSink for Outbox<'_, W> {
    fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped> {
        let end_of_document = event == ExtractEvent::DocumentEnd;
        let response = Response::Event { id: self.id, event };
        self.send(&response, end_of_document)
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::event::WarningKind;
    use crate::extract::ExtractOptions;
    use crate::protocol::{RejectReason, encode_frame};

    fn fixture(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(relative)
    }

    fn request(id: RequestId, path: PathBuf) -> Request {
        Request::Extract(ExtractRequest {
            id,
            path,
            options: ExtractOptions::default(),
        })
    }

    fn responses(bytes: &[u8]) -> Vec<Response> {
        let mut reader = FrameReader::new(bytes);
        let mut out = Vec::new();
        while let Ok(Some(response)) = reader.read::<Response>() {
            out.push(response);
        }
        out
    }

    /// Serve `input` con il padre vivo.
    fn serve_all<W: Write>(input: &[u8], output: W) -> Result<(), WorkerError> {
        serve_requests(input, output, &AtomicBool::new(false))
    }

    #[test]
    fn serves_requests_until_eof() {
        let mut input = Vec::new();
        let first = request(1, fixture("formati/spesa.txt"));
        let second = request(2, fixture("non-esiste.pdf"));
        assert!(encode_frame(&first, &mut input).is_ok());
        assert!(encode_frame(&second, &mut input).is_ok());
        let mut output = Vec::new();
        let result = serve_all(&input, &mut output);
        assert!(result.is_ok(), "{result:?}");
        let got = responses(&output);
        // Prima richiesta: inizio, almeno un'unità, fine documento, fine richiesta.
        let first_done = got
            .iter()
            .position(|r| *r == Response::Done { id: 1 })
            .unwrap_or(usize::MAX);
        assert!(first_done >= 3, "{got:?}");
        assert!(
            got[..first_done]
                .iter()
                .all(|r| matches!(r, Response::Event { id: 1, .. }))
        );
        // Seconda: solo l'avviso e la fine.
        assert_eq!(
            &got[first_done + 1..],
            &[
                Response::Event {
                    id: 2,
                    event: ExtractEvent::Warning {
                        inner: Vec::new(),
                        kind: WarningKind::NotFound
                    }
                },
                Response::Done { id: 2 }
            ]
        );
    }

    #[test]
    fn serves_an_archive_entry() {
        let path = fixture("demo/musica/archivio.zip");
        let mut input = Vec::new();
        let entry = Request::ExtractEntry(EntryRequest {
            id: 5,
            path: path.clone(),
            inner: vec!["elenco.pdf".to_owned()],
            max_bytes: u64::MAX,
            options: ExtractOptions::default(),
        });
        let missing = Request::ExtractEntry(EntryRequest {
            id: 6,
            path,
            inner: vec!["manca.pdf".to_owned()],
            max_bytes: u64::MAX,
            options: ExtractOptions::default(),
        });
        assert!(encode_frame(&entry, &mut input).is_ok());
        assert!(encode_frame(&missing, &mut input).is_ok());
        let mut output = Vec::new();
        let result = serve_all(&input, &mut output);
        assert!(result.is_ok(), "{result:?}");
        let got = responses(&output);
        let mut bytes = Vec::new();
        for response in &got[..got.len() - 2] {
            match response {
                Response::EntryData {
                    id: 5,
                    bytes: chunk,
                } => bytes.extend_from_slice(chunk),
                other => panic!("risposta inattesa: {other:?}"),
            }
        }
        assert!(bytes.starts_with(b"%PDF"));
        let size = u64::try_from(bytes.len()).unwrap_or(0);
        assert_eq!(
            &got[got.len() - 2..],
            &[
                Response::EntryEnd {
                    id: 5,
                    outcome: crate::extract::EntryOutcome::Extracted { bytes: size }
                },
                Response::EntryEnd {
                    id: 6,
                    outcome: crate::extract::EntryOutcome::Failed(WarningKind::NotFound)
                }
            ]
        );
    }

    /// Senza limite di memoria il processo estrattore non parte: nessuna richiesta servita.
    #[test]
    fn worker_refuses_missing_memory_limit() {
        let mut input = Vec::new();
        assert!(encode_frame(&request(1, fixture("formati/spesa.txt")), &mut input).is_ok());
        let mut output = Vec::new();
        let result = run(
            input.as_slice(),
            &mut output,
            &WorkerConfig::default(),
            || {},
        );
        assert!(
            matches!(result, Err(WorkerError::MemoryLimit(_))),
            "{result:?}"
        );
        assert!(output.is_empty());
    }

    /// Padre terminato: le richieste in coda non si servono, e un'estrazione in corso si
    /// ferma al primo evento senza mandarlo.
    #[test]
    fn orphaned_worker_stops_at_the_next_event() {
        let mut input = Vec::new();
        assert!(encode_frame(&request(1, fixture("formati/spesa.txt")), &mut input).is_ok());
        let mut output = Vec::new();
        let gone = AtomicBool::new(true);
        let result = serve_requests(input.as_slice(), &mut output, &gone);
        assert!(matches!(result, Err(WorkerError::ParentGone)), "{result:?}");
        assert!(output.is_empty());
        let mut writer = FrameWriter::new(Vec::new());
        let mut outbox = Outbox {
            writer: &mut writer,
            id: 1,
            parent_gone: &gone,
            halt: None,
        };
        assert_eq!(outbox.emit(ExtractEvent::DocumentEnd), Err(Stopped));
        assert!(matches!(outbox.halt, Some(Halt::ParentGone)));
        assert!(writer.into_inner().is_empty());
    }

    #[test]
    fn empty_input_exits_cleanly() {
        let mut output = Vec::new();
        assert!(serve_all(&[], &mut output).is_ok());
        assert!(output.is_empty());
    }

    #[test]
    fn malformed_frame_is_rejected_then_exit() {
        let mut input = 3_u32.to_le_bytes().to_vec();
        input.extend_from_slice(&[0xFF, 0xFF, 0xFF]);
        let mut output = Vec::new();
        let result = serve_all(&input, &mut output);
        assert!(matches!(
            result,
            Err(WorkerError::Protocol(ProtocolError::VersionMismatch { .. }))
        ));
        assert_eq!(
            responses(&output),
            [Response::Rejected {
                reason: RejectReason::VersionMismatch { found: 0xFFFF }
            }]
        );
    }

    #[test]
    fn truncated_request_is_rejected() {
        let mut input = Vec::new();
        assert!(encode_frame(&request(1, fixture("formati/spesa.txt")), &mut input).is_ok());
        input.truncate(input.len() - 1);
        let mut output = Vec::new();
        let result = serve_all(&input, &mut output);
        assert!(matches!(
            result,
            Err(WorkerError::Protocol(ProtocolError::Truncated))
        ));
        assert_eq!(
            responses(&output),
            [Response::Rejected {
                reason: RejectReason::Malformed
            }]
        );
    }

    /// Uno stdout che non accetta più nulla (padre sparito).
    struct ClosedPipe;

    impl Write for ClosedPipe {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }

    #[test]
    fn closed_output_stops_the_worker() {
        let mut input = Vec::new();
        assert!(encode_frame(&request(1, fixture("formati/spesa.txt")), &mut input).is_ok());
        let result = serve_all(&input, ClosedPipe);
        assert!(matches!(
            result,
            Err(WorkerError::Protocol(ProtocolError::Io(_)))
        ));
    }

    #[test]
    fn args_round_trip_and_partitions() {
        let config = WorkerConfig {
            memory_limit: Some(DEFAULT_MEMORY_LIMIT),
            parent_pid: Some(4321),
        };
        assert_eq!(WorkerConfig::from_args(config.to_args()), Ok(config));
        let parent = |value: &str| {
            WorkerConfig::from_args([OsString::from(PARENT_PID_FLAG), OsString::from(value)])
        };
        assert_eq!(
            parent("77"),
            Ok(WorkerConfig {
                memory_limit: None,
                parent_pid: Some(77),
            })
        );
        assert_eq!(parent("0"), Err(ArgsError::InvalidValue));
        assert_eq!(parent("-1"), Err(ArgsError::InvalidValue));
        assert_eq!(parent("4294967296"), Err(ArgsError::InvalidValue));
        assert_eq!(
            WorkerConfig::from_args([OsString::from(PARENT_PID_FLAG)]),
            Err(ArgsError::MissingValue)
        );
        assert_eq!(
            WorkerConfig::from_args(Vec::new()),
            Ok(WorkerConfig::default())
        );
        assert_eq!(
            WorkerConfig::from_args([OsString::from("--altro")]),
            Err(ArgsError::Unknown)
        );
        assert_eq!(
            WorkerConfig::from_args([OsString::from(MEMORY_LIMIT_FLAG)]),
            Err(ArgsError::MissingValue)
        );
        let too_small = (MIN_MEMORY_LIMIT.bytes() - 1).to_string();
        assert_eq!(
            WorkerConfig::from_args([OsString::from(MEMORY_LIMIT_FLAG), OsString::from(too_small)]),
            Err(ArgsError::InvalidValue)
        );
        assert_eq!(
            WorkerConfig::from_args([OsString::from(MEMORY_LIMIT_FLAG), OsString::from("1e9")]),
            Err(ArgsError::InvalidValue)
        );
    }
}
