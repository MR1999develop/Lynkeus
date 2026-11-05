// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Ciclo del processo estrattore: legge richieste da stdin e scrive risposte su stdout, nel
//! [`protocol`](crate::protocol).
//!
//! L'app avvia lo stesso eseguibile con l'opzione nascosta `--extract-worker` seguita da
//! [`WorkerConfig::to_args`], e nel figlio chiama [`run`] con stdin e stdout. Il ciclo:
//!
//! 1. all'avvio, prima di leggere qualunque file, imposta il limite di memoria se
//!    configurato (`RLIMIT_AS` su Linux; su Windows lo impone il Job Object del padre). Se
//!    non ci riesce esce con errore: un estrattore senza limite non parte;
//! 2. per ogni [`Request::Extract`] chiama [`extract_path`] e manda gli eventi man mano
//!    (il buffer si svuota a ogni fine documento e alla fine della richiesta); per ogni
//!    [`Request::ExtractEntry`] chiama [`extract_entry`] e manda i byte della voce a pezzi,
//!    poi l'esito;
//! 3. esce con `Ok` quando stdin finisce tra una richiesta e l'altra (il padre lo chiude per
//!    fermarlo), con errore su un frame non valido (dopo aver mandato
//!    [`Response::Rejected`]) o se stdout non si scrive più.
//!
//! Il processo legge solo i file richiesti, non scrive su disco, non avvia programmi e non
//! usa la rete. Stdout è riservato al protocollo: niente `print!` in questo crate (i log, se
//! servono, vanno su stderr).

use std::ffi::OsString;
use std::io::{self, BufReader, BufWriter, Read, Write};

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

/// Configurazione del processo estrattore, passata come argomenti dal padre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkerConfig {
    /// Limite di memoria che il processo impone a sé stesso dove può (Linux); `None` =
    /// nessun limite proprio.
    pub memory_limit: Option<ByteSize>,
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
    /// Valore non numerico o sotto [`MIN_MEMORY_LIMIT`].
    #[error("limite di memoria non valido")]
    InvalidValue,
}

impl WorkerConfig {
    /// Gli argomenti da passare al figlio dopo `--extract-worker`.
    pub fn to_args(&self) -> Vec<OsString> {
        match self.memory_limit {
            Some(limit) => vec![
                OsString::from(MEMORY_LIMIT_FLAG),
                OsString::from(limit.bytes().to_string()),
            ],
            None => Vec::new(),
        }
    }

    /// Legge gli argomenti che seguono `--extract-worker` (quelli di [`WorkerConfig::to_args`]).
    pub fn from_args<I: IntoIterator<Item = OsString>>(args: I) -> Result<WorkerConfig, ArgsError> {
        let mut config = WorkerConfig::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg != MEMORY_LIMIT_FLAG {
                return Err(ArgsError::Unknown);
            }
            let value = args.next().ok_or(ArgsError::MissingValue)?;
            let bytes = value
                .to_str()
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|b| *b >= MIN_MEMORY_LIMIT.bytes())
                .ok_or(ArgsError::InvalidValue)?;
            config.memory_limit = Some(ByteSize::from_bytes(bytes));
        }
        Ok(config)
    }
}

/// Perché il processo estrattore si è fermato con errore.
#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    /// Il limite di memoria non si è potuto impostare.
    #[error("limite di memoria non impostato")]
    MemoryLimit(#[source] io::Error),
    /// Frame non valido in ingresso o stdout non scrivibile.
    #[error("errore del protocollo")]
    Protocol(#[source] ProtocolError),
}

/// Il ciclo del processo estrattore su `input` (stdin) e `output` (stdout).
///
/// Restituisce `Ok` quando `input` finisce tra una richiesta e l'altra.
pub fn run<R: Read, W: Write>(
    input: R,
    output: W,
    config: &WorkerConfig,
) -> Result<(), WorkerError> {
    if let Some(limit) = config.memory_limit {
        platform::limit_address_space(limit.bytes()).map_err(WorkerError::MemoryLimit)?;
    }
    let mut reader = FrameReader::new(BufReader::new(input));
    let mut writer = FrameWriter::new(BufWriter::new(output));
    loop {
        match reader.read::<Request>() {
            Ok(None) => return Ok(()),
            Ok(Some(Request::Extract(request))) => serve(&mut writer, &request)?,
            Ok(Some(Request::ExtractEntry(request))) => serve_entry(&mut writer, &request)?,
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

/// Esegue una richiesta e manda le risposte, chiudendo con [`Response::Done`].
fn serve<W: Write>(
    writer: &mut FrameWriter<W>,
    request: &ExtractRequest,
) -> Result<(), WorkerError> {
    let mut sink = ResponseSink {
        writer: &mut *writer,
        id: request.id,
        error: None,
    };
    let outcome = extract_path(&request.path, &request.options, &mut sink);
    if let Some(error) = sink.error {
        return Err(WorkerError::Protocol(error));
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
) -> Result<(), WorkerError> {
    let mut sink = EntryDataSink {
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
        return Err(WorkerError::Protocol(error));
    }
    let Ok(outcome) = outcome else {
        // Solo questo destinatario può fermare, e lo fa solo con un errore già restituito.
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

/// Destinatario che scrive ogni pezzo di una voce come [`Response::EntryData`].
struct EntryDataSink<'w, W: Write> {
    writer: &'w mut FrameWriter<W>,
    id: RequestId,
    error: Option<ProtocolError>,
}

impl<W: Write> EntrySink for EntryDataSink<'_, W> {
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

/// Destinatario che scrive ogni evento come [`Response::Event`].
struct ResponseSink<'w, W: Write> {
    writer: &'w mut FrameWriter<W>,
    id: RequestId,
    error: Option<ProtocolError>,
}

impl<W: Write> EventSink for ResponseSink<'_, W> {
    fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped> {
        let end_of_document = event == ExtractEvent::DocumentEnd;
        let written = self
            .writer
            .write(&Response::Event { id: self.id, event })
            .and_then(|()| {
                if end_of_document {
                    self.writer.flush()
                } else {
                    Ok(())
                }
            });
        written.map_err(|error| {
            self.error = Some(error);
            Stopped
        })
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

    #[test]
    fn serves_requests_until_eof() {
        let mut input = Vec::new();
        let first = request(1, fixture("formati/spesa.txt"));
        let second = request(2, fixture("non-esiste.pdf"));
        assert!(encode_frame(&first, &mut input).is_ok());
        assert!(encode_frame(&second, &mut input).is_ok());
        let mut output = Vec::new();
        let result = run(input.as_slice(), &mut output, &WorkerConfig::default());
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
        let result = run(input.as_slice(), &mut output, &WorkerConfig::default());
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

    #[test]
    fn empty_input_exits_cleanly() {
        let mut output = Vec::new();
        assert!(run(&[][..], &mut output, &WorkerConfig::default()).is_ok());
        assert!(output.is_empty());
    }

    #[test]
    fn malformed_frame_is_rejected_then_exit() {
        let mut input = 3_u32.to_le_bytes().to_vec();
        input.extend_from_slice(&[0xFF, 0xFF, 0xFF]);
        let mut output = Vec::new();
        let result = run(input.as_slice(), &mut output, &WorkerConfig::default());
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
        let result = run(input.as_slice(), &mut output, &WorkerConfig::default());
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
        let result = run(input.as_slice(), ClosedPipe, &WorkerConfig::default());
        assert!(matches!(
            result,
            Err(WorkerError::Protocol(ProtocolError::Io(_)))
        ));
    }

    #[test]
    fn args_round_trip_and_partitions() {
        let config = WorkerConfig {
            memory_limit: Some(DEFAULT_MEMORY_LIMIT),
        };
        assert_eq!(WorkerConfig::from_args(config.to_args()), Ok(config));
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
