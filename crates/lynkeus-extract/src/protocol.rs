// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Protocollo tra il processo principale e il processo estrattore, su stdin e stdout.
//!
//! # Frame
//!
//! Ogni messaggio è un frame:
//!
//! ```text
//! lunghezza: u32 little endian   byte che seguono (versione + corpo), da 2 a MAX_FRAME_BYTES
//! versione:  u16 little endian   PROTOCOL_VERSION
//! corpo:     postcard            un Request (verso l'estrattore) o un Response (verso il padre)
//! ```
//!
//! Il lettore ([`FrameReader`]) controlla la lunghezza contro [`MAX_FRAME_BYTES`] prima di
//! allocare, poi la versione, poi decodifica il corpo in un tipo concreto e rifiuta i byte
//! in più. Fine del flusso prima del primo byte di un frame: fine normale (`Ok(None)`); a
//! metà frame: [`ProtocolError::Truncated`]. Ogni altra violazione è un errore: chi la
//! riceve chiude il canale (il padre termina e sostituisce il processo).
//!
//! # Dialogo
//!
//! 1. Il padre manda [`Request::Extract`] con un `id` scelto da lui.
//! 2. L'estrattore risponde con zero o più [`Response::Event`] con lo stesso `id`, nella
//!    sequenza di [`crate::event`], poi [`Response::Done`]. I problemi del file sono eventi
//!    (avvisi), non errori del protocollo.
//! 3. Una richiesta che l'estrattore non sa leggere riceve [`Response::Rejected`], poi
//!    l'estrattore esce.
//! 4. Per fermare l'estrattore il padre chiude il suo stdin: l'estrattore esce alla
//!    richiesta successiva (o viene terminato dopo 500 ms).
//! 5. Per aprire una voce d'archivio il padre manda [`Request::ExtractEntry`]: l'estrattore
//!    risponde con zero o più [`Response::EntryData`] (i byte della voce, in ordine, ognuno
//!    al più [`ENTRY_CHUNK_BYTES`](crate::ENTRY_CHUNK_BYTES)) e poi un solo
//!    [`Response::EntryEnd`] con l'esito; se l'esito non è "estratta" i byte già mandati non
//!    valgono.
//!
//! Nessun messaggio va dall'estrattore al padre se non in risposta a una richiesta. I tipi
//! dei messaggi (e quelli degli eventi) si cambiano solo aggiungendo varianti in fondo agli
//! enum; ogni altro cambiamento alza [`PROTOCOL_VERSION`], che fa parte anche della chiave
//! della cache.

use std::io::{self, Read, Write};
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::event::ExtractEvent;
use crate::extract::{EntryOutcome, ExtractOptions};

/// Versione del protocollo (e del formato degli eventi nella cache).
///
/// 2: richiesta di una voce d'archivio ([`Request::ExtractEntry`]) ed etichette uniche per
/// le voci con lo stesso nome (cambiano gli `inner` degli eventi già in cache).
///
/// 3: frame al più di [`MAX_FRAME_BYTES`] (1,25 MiB; prima 16 MiB).
pub const PROTOCOL_VERSION: u16 = 3;

/// Byte massimi di un frame dopo la lunghezza (1,25 MiB): il messaggio più grande è
/// un'unità di testo ([`MAX_UNIT_TEXT_BYTES`](crate::MAX_UNIT_TEXT_BYTES), 1 MiB) o un pezzo
/// di voce ([`ENTRY_CHUNK_BYTES`](crate::ENTRY_CHUNK_BYTES), 1 MiB), più posizione,
/// etichette o percorso, che stanno nel margine di 256 KiB. Chi legge non tiene di più per
/// messaggio, anche se l'altra parte non rispetta i suoi limiti.
pub const MAX_FRAME_BYTES: u32 = 1024 * 1024 + 256 * 1024;

/// Byte di versione in testa al corpo.
const VERSION_BYTES: usize = 2;

/// Identificatore di una richiesta, scelto dal padre.
pub type RequestId = u64;

/// Messaggio dal padre all'estrattore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Estrai il testo di un file.
    Extract(ExtractRequest),
    /// Estrai i byte di una voce d'archivio.
    ExtractEntry(EntryRequest),
}

/// Richiesta dei byte di una voce d'archivio, per aprirla.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryRequest {
    /// Identificatore, ripetuto in ogni risposta.
    pub id: RequestId,
    /// L'archivio più esterno (anche con caratteri non UTF-8).
    #[serde(with = "os_path")]
    pub path: PathBuf,
    /// Etichette delle voci dall'esterno all'interno, come negli eventi.
    pub inner: Vec<String>,
    /// Byte massimi della voce; oltre, l'esito è "limite superato".
    pub max_bytes: u64,
    /// Limiti degli archivi attraversati.
    pub options: ExtractOptions,
}

/// Richiesta di estrazione di un file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractRequest {
    /// Identificatore, ripetuto in ogni risposta.
    pub id: RequestId,
    /// Il file da leggere (anche con caratteri non UTF-8).
    #[serde(with = "os_path")]
    pub path: PathBuf,
    /// Limiti dell'estrazione.
    pub options: ExtractOptions,
}

/// Messaggio dall'estrattore al padre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    /// Un evento dell'estrazione `id`.
    Event {
        /// La richiesta.
        id: RequestId,
        /// L'evento.
        event: ExtractEvent,
    },
    /// Fine dell'estrazione `id`.
    Done {
        /// La richiesta.
        id: RequestId,
    },
    /// Richiesta illeggibile: l'estrattore esce subito dopo.
    Rejected {
        /// Perché.
        reason: RejectReason,
    },
    /// Un pezzo dei byte della voce chiesta con `id`.
    EntryData {
        /// La richiesta.
        id: RequestId,
        /// I byte, al più [`ENTRY_CHUNK_BYTES`](crate::ENTRY_CHUNK_BYTES).
        bytes: Vec<u8>,
    },
    /// Fine della richiesta di una voce `id`, con l'esito.
    EntryEnd {
        /// La richiesta.
        id: RequestId,
        /// Com'è finita.
        outcome: EntryOutcome,
    },
}

/// Perché l'estrattore ha rifiutato una richiesta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectReason {
    /// Versione del protocollo diversa.
    VersionMismatch {
        /// La versione ricevuta.
        found: u16,
    },
    /// Frame oltre il massimo.
    FrameTooLarge,
    /// Frame troncato o corpo non decodificabile.
    Malformed,
}

/// Errore del protocollo.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// Lettura o scrittura del canale fallita.
    #[error("errore di I/O sul canale")]
    Io(#[source] io::Error),
    /// Il flusso finisce a metà frame.
    #[error("frame troncato")]
    Truncated,
    /// Lunghezza dichiarata (o da scrivere) oltre [`MAX_FRAME_BYTES`].
    #[error("frame di {len} byte, oltre il massimo")]
    FrameTooLarge {
        /// La lunghezza.
        len: u64,
    },
    /// Versione diversa da [`PROTOCOL_VERSION`].
    #[error("versione del protocollo {found}, attesa {PROTOCOL_VERSION}")]
    VersionMismatch {
        /// La versione ricevuta.
        found: u16,
    },
    /// Lunghezza troppo corta, corpo non decodificabile o con byte in più.
    #[error("frame non valido")]
    Malformed,
}

impl ProtocolError {
    /// Il motivo da mandare in [`Response::Rejected`], se l'errore riguarda il frame.
    pub fn reject_reason(&self) -> Option<RejectReason> {
        match self {
            ProtocolError::Io(_) => None,
            ProtocolError::FrameTooLarge { .. } => Some(RejectReason::FrameTooLarge),
            ProtocolError::VersionMismatch { found } => {
                Some(RejectReason::VersionMismatch { found: *found })
            }
            ProtocolError::Truncated | ProtocolError::Malformed => Some(RejectReason::Malformed),
        }
    }
}

/// Legge frame da un flusso, riusando il buffer.
#[derive(Debug)]
pub struct FrameReader<R> {
    inner: R,
    buf: Vec<u8>,
}

impl<R: Read> FrameReader<R> {
    /// Un lettore su `inner` (meglio se bufferizzato).
    pub fn new(inner: R) -> FrameReader<R> {
        FrameReader {
            inner,
            buf: Vec::new(),
        }
    }

    /// Il prossimo messaggio, o `None` se il flusso finisce tra un frame e l'altro.
    pub fn read<T: DeserializeOwned>(&mut self) -> Result<Option<T>, ProtocolError> {
        let mut len_bytes = [0_u8; 4];
        if !read_exact_or_eof(&mut self.inner, &mut len_bytes)? {
            return Ok(None);
        }
        let len = u32::from_le_bytes(len_bytes);
        if len > MAX_FRAME_BYTES {
            return Err(ProtocolError::FrameTooLarge {
                len: u64::from(len),
            });
        }
        let len = usize::try_from(len).map_err(|_| ProtocolError::Malformed)?;
        if len < VERSION_BYTES {
            return Err(ProtocolError::Malformed);
        }
        self.buf.clear();
        let wanted = u64::try_from(len).unwrap_or(u64::MAX);
        (&mut self.inner)
            .take(wanted)
            .read_to_end(&mut self.buf)
            .map_err(ProtocolError::Io)?;
        if self.buf.len() < len {
            return Err(ProtocolError::Truncated);
        }
        decode_payload(&self.buf).map(Some)
    }

    /// Il flusso sottostante.
    pub fn into_inner(self) -> R {
        self.inner
    }
}

/// Scrive frame su un flusso.
#[derive(Debug)]
pub struct FrameWriter<W> {
    inner: W,
    buf: Vec<u8>,
}

impl<W: Write> FrameWriter<W> {
    /// Uno scrittore su `inner` (meglio se bufferizzato: si svuota con [`FrameWriter::flush`]).
    pub fn new(inner: W) -> FrameWriter<W> {
        FrameWriter {
            inner,
            buf: Vec::new(),
        }
    }

    /// Scrive un messaggio. Un messaggio oltre [`MAX_FRAME_BYTES`] non si scrive.
    pub fn write<T: Serialize>(&mut self, message: &T) -> Result<(), ProtocolError> {
        self.buf.clear();
        encode_frame(message, &mut self.buf)?;
        self.inner.write_all(&self.buf).map_err(ProtocolError::Io)
    }

    /// Svuota il buffer del flusso.
    pub fn flush(&mut self) -> Result<(), ProtocolError> {
        self.inner.flush().map_err(ProtocolError::Io)
    }

    /// Il flusso sottostante.
    pub fn into_inner(self) -> W {
        self.inner
    }
}

/// Aggiunge a `out` il frame di `message`; errore se oltre [`MAX_FRAME_BYTES`].
pub fn encode_frame<T: Serialize>(message: &T, out: &mut Vec<u8>) -> Result<(), ProtocolError> {
    let start = out.len();
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    let encoded = postcard::to_io(message, &mut *out);
    if encoded.is_err() {
        out.truncate(start);
        return Err(ProtocolError::Malformed);
    }
    let len = out.len().saturating_sub(start.saturating_add(4));
    let len_u64 = u64::try_from(len).unwrap_or(u64::MAX);
    let fits = u32::try_from(len)
        .ok()
        .filter(|len| *len <= MAX_FRAME_BYTES);
    let Some(len32) = fits else {
        out.truncate(start);
        return Err(ProtocolError::FrameTooLarge { len: len_u64 });
    };
    if let Some(prefix) = out.get_mut(start..start.saturating_add(4)) {
        prefix.copy_from_slice(&len32.to_le_bytes());
    }
    Ok(())
}

/// Decodifica versione e corpo di un frame (senza la lunghezza in testa).
fn decode_payload<T: DeserializeOwned>(payload: &[u8]) -> Result<T, ProtocolError> {
    let (version, body) = payload
        .split_first_chunk::<VERSION_BYTES>()
        .ok_or(ProtocolError::Malformed)?;
    let found = u16::from_le_bytes(*version);
    if found != PROTOCOL_VERSION {
        return Err(ProtocolError::VersionMismatch { found });
    }
    let (message, rest) =
        postcard::take_from_bytes::<T>(body).map_err(|_| ProtocolError::Malformed)?;
    if !rest.is_empty() {
        return Err(ProtocolError::Malformed);
    }
    Ok(message)
}

/// Riempie `buf`; `false` se il flusso finisce prima del primo byte, [`ProtocolError::Truncated`]
/// se finisce dopo.
fn read_exact_or_eof(reader: &mut impl Read, buf: &mut [u8]) -> Result<bool, ProtocolError> {
    let mut filled = 0;
    while filled < buf.len() {
        let Some(rest) = buf.get_mut(filled..) else {
            break;
        };
        match reader.read(rest) {
            Ok(0) if filled == 0 => return Ok(false),
            Ok(0) => return Err(ProtocolError::Truncated),
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(ProtocolError::Io(e)),
        }
    }
    Ok(true)
}

/// `PathBuf` come `OsString`: anche i percorsi non UTF-8 (possibili su Windows e Linux)
/// passano intatti. Padre ed estrattore sono lo stesso eseguibile sullo stesso sistema.
mod os_path {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        path.as_os_str().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
        OsString::deserialize(deserializer).map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use lynkeus_core::{Location, TextUnit};
    use proptest::prelude::*;

    use super::*;
    use crate::event::WarningKind;
    use crate::format::Format;
    use crate::limits::LimitKind;

    fn frame<T: Serialize>(message: &T) -> Vec<u8> {
        let mut out = Vec::new();
        assert!(encode_frame(message, &mut out).is_ok());
        out
    }

    fn read_one<T: DeserializeOwned>(bytes: &[u8]) -> Result<Option<T>, ProtocolError> {
        FrameReader::new(bytes).read::<T>()
    }

    fn sample_request() -> Request {
        Request::Extract(ExtractRequest {
            id: 42,
            path: PathBuf::from("dati/relazione.docx"),
            options: ExtractOptions::default(),
        })
    }

    fn sample_responses() -> Vec<Response> {
        vec![
            Response::Event {
                id: 7,
                event: ExtractEvent::DocumentStart {
                    inner: vec!["livello2.zip".to_owned(), "interno.docx".to_owned()],
                    format: Format::Docx,
                },
            },
            Response::Event {
                id: 7,
                event: ExtractEvent::Unit(TextUnit {
                    location: Location::Cell {
                        sheet: "Spesa".to_owned(),
                        row: 7,
                        col: 2,
                    },
                    text: "aragosta in città".to_owned(),
                }),
            },
            Response::Event {
                id: 7,
                event: ExtractEvent::Warning {
                    inner: Vec::new(),
                    kind: WarningKind::LimitExceeded(LimitKind::CompressionRatio),
                },
            },
            Response::Done { id: 7 },
            Response::Rejected {
                reason: RejectReason::VersionMismatch { found: 9 },
            },
            Response::EntryData {
                id: 8,
                bytes: vec![0, 1, 2, 255],
            },
            Response::EntryEnd {
                id: 8,
                outcome: EntryOutcome::Extracted { bytes: 4 },
            },
            Response::EntryEnd {
                id: 9,
                outcome: EntryOutcome::Failed(WarningKind::Encrypted),
            },
        ]
    }

    #[test]
    fn round_trip_of_every_message() {
        let entry = Request::ExtractEntry(EntryRequest {
            id: 8,
            path: PathBuf::from("musica/archivio.zip"),
            inner: vec!["livello2.zip".to_owned(), "elenco.pdf".to_owned()],
            max_bytes: 1 << 30,
            options: ExtractOptions::default(),
        });
        for request in [sample_request(), entry] {
            let decoded: Option<Request> = read_one(&frame(&request)).ok().flatten();
            assert_eq!(decoded, Some(request));
        }

        let responses = sample_responses();
        let mut stream = Vec::new();
        let mut writer = FrameWriter::new(&mut stream);
        for response in &responses {
            assert!(writer.write(response).is_ok());
        }
        let mut reader = FrameReader::new(stream.as_slice());
        for expected in &responses {
            let got = reader.read::<Response>().ok().flatten();
            assert_eq!(got.as_ref(), Some(expected));
        }
        assert!(matches!(reader.read::<Response>(), Ok(None)));
    }

    #[test]
    fn frame_layout_is_documented_one() {
        let bytes = frame(&Response::Done { id: 1 });
        let len = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert_eq!(usize::try_from(len).ok(), Some(bytes.len() - 4));
        assert_eq!(&bytes[4..6], &PROTOCOL_VERSION.to_le_bytes());
    }

    #[test]
    fn empty_stream_is_clean_end() {
        assert!(matches!(read_one::<Request>(&[]), Ok(None)));
    }

    #[test]
    fn every_truncation_is_detected() {
        let bytes = frame(&sample_request());
        for cut in 1..bytes.len() {
            let result = read_one::<Request>(&bytes[..cut]);
            assert!(
                matches!(result, Err(ProtocolError::Truncated)),
                "taglio a {cut}: {result:?}"
            );
        }
    }

    #[test]
    fn oversized_length_is_rejected_before_reading() {
        // Solo la lunghezza: se il lettore provasse a leggere o allocare 4 GiB fallirebbe
        // con Truncated, non con FrameTooLarge.
        let result = read_one::<Request>(&u32::MAX.to_le_bytes());
        assert!(matches!(result, Err(ProtocolError::FrameTooLarge { .. })));
        let just_over = (MAX_FRAME_BYTES + 1).to_le_bytes();
        assert!(matches!(
            read_one::<Request>(&just_over),
            Err(ProtocolError::FrameTooLarge { .. })
        ));
    }

    #[test]
    fn short_length_wrong_version_and_trailing_bytes() {
        let mut short = 1_u32.to_le_bytes().to_vec();
        short.push(1);
        assert!(matches!(
            read_one::<Request>(&short),
            Err(ProtocolError::Malformed)
        ));

        let mut wrong = frame(&sample_request());
        wrong[4] = wrong[4].wrapping_add(1);
        assert!(matches!(
            read_one::<Request>(&wrong),
            Err(ProtocolError::VersionMismatch { .. })
        ));

        let mut trailing = frame(&Response::Done { id: 1 });
        trailing.push(0);
        let len = u32::try_from(trailing.len() - 4).unwrap_or(0);
        trailing[..4].copy_from_slice(&len.to_le_bytes());
        assert!(matches!(
            read_one::<Response>(&trailing),
            Err(ProtocolError::Malformed)
        ));
    }

    /// Il frame massimo sta di poco sopra il messaggio più grande che il protocollo ammette:
    /// chi legge (il thread del pool, con 16 messaggi in coda) non tiene molto più di quanto
    /// serve, anche se il figlio non rispetta i suoi limiti. I messaggi più grandi ci stanno:
    /// un'unità di testo massima con un'etichetta massima, un inizio documento alla profondità
    /// massima con etichette massime, una richiesta di una voce con un percorso lungo.
    #[test]
    fn frame_limit_is_close_to_the_largest_message() {
        let limit = usize::try_from(MAX_FRAME_BYTES).unwrap_or(usize::MAX);
        assert!(limit <= crate::MAX_UNIT_TEXT_BYTES + 512 * 1024, "{limit}");
        let label = "è".repeat(crate::extract::archive::MAX_LABEL_BYTES / 2);
        let depth = usize::try_from(crate::extract::budget::MAX_DEPTH_CEILING).unwrap_or(0) + 1;
        let unit = Response::Event {
            id: u64::MAX,
            event: ExtractEvent::Unit(TextUnit {
                location: Location::Cell {
                    sheet: label.clone(),
                    row: u32::MAX,
                    col: u32::MAX,
                },
                text: "a".repeat(crate::MAX_UNIT_TEXT_BYTES),
            }),
        };
        let start = Response::Event {
            id: u64::MAX,
            event: ExtractEvent::DocumentStart {
                inner: vec![label.clone(); depth],
                format: Format::Docx,
            },
        };
        let entry = Request::ExtractEntry(EntryRequest {
            id: u64::MAX,
            path: PathBuf::from("è".repeat(32_767)),
            inner: vec![label; depth],
            max_bytes: u64::MAX,
            options: ExtractOptions::default(),
        });
        let mut out = Vec::new();
        assert!(encode_frame(&unit, &mut out).is_ok());
        assert!(encode_frame(&start, &mut out).is_ok());
        assert!(encode_frame(&entry, &mut out).is_ok());
    }

    #[test]
    fn largest_entry_chunk_fits_in_a_frame() {
        let chunk = Response::EntryData {
            id: u64::MAX,
            bytes: vec![0xFF; crate::ENTRY_CHUNK_BYTES],
        };
        let mut out = Vec::new();
        assert!(encode_frame(&chunk, &mut out).is_ok());
    }

    #[test]
    fn oversized_message_is_not_written() {
        let huge = Response::Event {
            id: 1,
            event: ExtractEvent::Unit(TextUnit {
                location: Location::Line(1),
                text: "a".repeat(usize::try_from(MAX_FRAME_BYTES).unwrap_or(0)),
            }),
        };
        let mut out = vec![9_u8];
        let result = encode_frame(&huge, &mut out);
        assert!(matches!(result, Err(ProtocolError::FrameTooLarge { .. })));
        assert_eq!(out, [9]);
    }

    #[test]
    fn reject_reasons() {
        assert_eq!(
            ProtocolError::Truncated.reject_reason(),
            Some(RejectReason::Malformed)
        );
        assert_eq!(
            ProtocolError::Io(io::Error::from(io::ErrorKind::BrokenPipe)).reject_reason(),
            None
        );
    }

    #[cfg(windows)]
    #[test]
    fn non_utf8_path_survives() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;
        let path = PathBuf::from(OsString::from_wide(&[0x61, 0xD800, 0x62]));
        assert!(path.to_str().is_none());
        let request = Request::Extract(ExtractRequest {
            id: 1,
            path: path.clone(),
            options: ExtractOptions::default(),
        });
        let decoded: Option<Request> = read_one(&frame(&request)).ok().flatten();
        assert_eq!(decoded, Some(request));
    }

    proptest! {
        /// Andata e ritorno di eventi con testo e nomi qualsiasi.
        #[test]
        fn events_round_trip(
            id in any::<u64>(),
            text in ".{0,200}",
            inner in proptest::collection::vec(".{0,40}", 0..4),
            line in any::<u32>(),
        ) {
            let messages = [
                Response::Event { id, event: ExtractEvent::DocumentStart { inner: inner.clone(), format: Format::Pdf } },
                Response::Event { id, event: ExtractEvent::Unit(TextUnit { location: Location::PageLine { page: 1, line }, text }) },
                Response::Event { id, event: ExtractEvent::DocumentEnd },
                Response::Event { id, event: ExtractEvent::Skipped { inner, reason: crate::event::SkipReason::Binary } },
            ];
            let mut stream = Vec::new();
            for message in &messages {
                prop_assert!(encode_frame(message, &mut stream).is_ok());
            }
            let mut reader = FrameReader::new(stream.as_slice());
            for message in &messages {
                let got = reader.read::<Response>().ok().flatten();
                prop_assert_eq!(got.as_ref(), Some(message));
            }
        }

        /// Byte qualsiasi: mai un panic, mai un messaggio inventato da un frame troppo corto.
        #[test]
        fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let mut reader = FrameReader::new(bytes.as_slice());
            for _ in 0..8 {
                match reader.read::<Response>() {
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => break,
                }
            }
        }
    }
}
