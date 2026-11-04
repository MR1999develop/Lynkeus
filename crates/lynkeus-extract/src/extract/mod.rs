// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Estrazione del testo dai documenti e dagli archivi, nello stesso processo.
//!
//! [`extract_path`] legge un file e manda gli [`ExtractEvent`](crate::ExtractEvent) a un
//! [`EventSink`]: il processo estrattore lo chiama per ogni richiesta, i test lo chiamano
//! direttamente. Il processo principale non lo chiama mai sui file dell'utente: i decoder
//! girano solo nel processo isolato (ADR 0001).
//!
//! Flusso per ogni documento (anche le voci degli archivi):
//!
//! 1. formato dall'estensione ([`Format::for_name`]), testo se non è nel registro;
//! 2. verifica del contenuto dalla firma ([`crate::format`]);
//! 3. funzione d'estrazione del registro, con i limiti del [`budget`] e senza panic che
//!    escano: un panic di una libreria diventa l'avviso "danneggiato";
//! 4. ogni errore diventa un avviso (o un "saltato") per quel documento, e gli altri
//!    documenti dello stesso archivio proseguono.
//!
//! Le unità più lunghe di [`MAX_UNIT_TEXT_BYTES`] si dividono in più unità con la stessa
//! posizione (o la riga corretta, per le righe), così nessun messaggio supera il massimo
//! del protocollo.

pub(crate) mod archive;
pub(crate) mod budget;
pub(crate) mod ooxml;
pub(crate) mod opendoc;
pub(crate) mod pdf;
pub(crate) mod plain;
pub(crate) mod sheet;
pub(crate) mod xml;

use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::rc::Rc;

use lynkeus_core::{Location, TextUnit};
use serde::{Deserialize, Serialize};

use crate::error::ExtractError;
use crate::event::{EventSink, ExtractEvent, SkipReason, Stopped, WarningKind, line_location};
use crate::format::{self, Format, Kind, SNIFF_BYTES, Signature, Verdict};
use crate::limits::ArchiveLimits;
use crate::platform;
use crate::text::soft_cut;

use budget::{Budget, LimitedReader, classify_io, normalize, read_all};

/// Testo massimo di un'unità: le più lunghe si dividono.
pub const MAX_UNIT_TEXT_BYTES: usize = 1024 * 1024;

/// Opzioni di un'estrazione; viaggiano nella richiesta al processo estrattore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExtractOptions {
    /// Limiti su archivi e contenitori (anche il massimo per voce vale per il file letto).
    pub limits: ArchiveLimits,
}

/// Estrae il testo del file `path` e manda gli eventi a `sink`.
///
/// Non restituisce errori di lettura: ogni problema del file o delle sue voci diventa un
/// [`ExtractEvent::Warning`] o un [`ExtractEvent::Skipped`]. Restituisce [`Stopped`] solo se
/// è `sink` a chiedere di fermarsi. Legge solo `path` (e nulla scrive su disco).
pub fn extract_path<S: EventSink + ?Sized>(
    path: &Path,
    options: &ExtractOptions,
    sink: &mut S,
) -> Result<(), Stopped> {
    let mut sink = sink;
    let mut ctx = Ctx::new(&mut sink, options.limits);
    let declared = Format::for_path(path).unwrap_or(Format::Text);
    match platform::open_for_reading(path) {
        Ok(file) => read_document(&mut ctx, declared, Input::File(file)),
        Err(error) => ctx.report(ExtractError::from_io(error)),
    }
}

/// Come [`extract_path`], per un file già in memoria chiamato `name` (il nome decide il
/// formato, come l'estensione di un file).
pub fn extract_bytes<S: EventSink + ?Sized>(
    name: &str,
    bytes: Vec<u8>,
    options: &ExtractOptions,
    sink: &mut S,
) -> Result<(), Stopped> {
    let mut sink = sink;
    let mut ctx = Ctx::new(&mut sink, options.limits);
    let declared = Format::for_name(name).unwrap_or(Format::Text);
    read_document(&mut ctx, declared, Input::Bytes(bytes))
}

/// Perché un estrattore si è interrotto.
#[derive(Debug)]
pub(crate) enum Fail {
    /// Chi riceve gli eventi ha chiesto di fermarsi.
    Stopped,
    /// Il documento non si è potuto leggere (del tutto o in parte).
    Error(ExtractError),
}

impl From<Stopped> for Fail {
    fn from(_: Stopped) -> Fail {
        Fail::Stopped
    }
}

impl From<ExtractError> for Fail {
    fn from(error: ExtractError) -> Fail {
        Fail::Error(error)
    }
}

impl From<io::Error> for Fail {
    fn from(error: io::Error) -> Fail {
        Fail::Error(classify_io(error))
    }
}

/// I byte di un documento, nella forma più comoda per chi li legge.
pub(crate) enum Input<'a> {
    /// Un file su disco (il file radice).
    File(File),
    /// Byte in memoria (voce d'archivio già letta entro i limiti).
    Bytes(Vec<u8>),
    /// Un flusso già limitato (voce di testo d'archivio, letta senza tenerla in memoria).
    Stream(&'a mut dyn Read),
}

/// Sorgente con posizionamento, per zip e calamine.
pub(crate) enum Seekable {
    File(BufReader<File>),
    Bytes(Cursor<Vec<u8>>),
}

impl Read for Seekable {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Seekable::File(file) => file.read(buf),
            Seekable::Bytes(bytes) => bytes.read(buf),
        }
    }
}

impl Seek for Seekable {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match self {
            Seekable::File(file) => file.seek(pos),
            Seekable::Bytes(bytes) => bytes.seek(pos),
        }
    }
}

impl<'a> Input<'a> {
    /// La firma del contenuto; un flusso non si esamina (vale solo per il testo).
    fn signature(&mut self) -> Result<Signature, ExtractError> {
        match self {
            Input::File(file) => {
                let mut head = Vec::with_capacity(SNIFF_BYTES);
                let limit = u64::try_from(SNIFF_BYTES).unwrap_or(u64::MAX);
                (&mut *file)
                    .take(limit)
                    .read_to_end(&mut head)
                    .map_err(ExtractError::from_io)?;
                file.seek(SeekFrom::Start(0))
                    .map_err(ExtractError::from_io)?;
                Ok(Signature::sniff(&head))
            }
            Input::Bytes(bytes) => Ok(Signature::sniff(bytes)),
            Input::Stream(_) => Ok(Signature::Unknown),
        }
    }

    /// Una sorgente con posizionamento e la sua lunghezza in byte; un flusso si legge in
    /// memoria entro i limiti.
    pub fn into_seekable(self, ctx: &Ctx<'_>) -> Result<(Seekable, u64), ExtractError> {
        match self {
            Input::File(file) => {
                let len = file.metadata().map_err(ExtractError::from_io)?.len();
                Ok((Seekable::File(BufReader::new(file)), len))
            }
            Input::Bytes(bytes) => {
                let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                Ok((Seekable::Bytes(Cursor::new(bytes)), len))
            }
            Input::Stream(_) => {
                let bytes = self.into_bytes(ctx)?;
                let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                Ok((Seekable::Bytes(Cursor::new(bytes)), len))
            }
        }
    }

    /// Tutti i byte in memoria, al più il massimo per voce (contati nel totale del file).
    pub fn into_bytes(self, ctx: &Ctx<'_>) -> Result<Vec<u8>, ExtractError> {
        match self {
            Input::Bytes(bytes) => Ok(bytes),
            Input::File(file) => read_all(LimitedReader::new(file, ctx.budget(), None)),
            // Il flusso è già limitato da chi lo ha creato.
            Input::Stream(stream) => read_all(stream),
        }
    }

    /// Un lettore sequenziale.
    pub fn into_reader(self) -> Box<dyn Read + 'a> {
        match self {
            Input::File(file) => Box::new(BufReader::new(file)),
            Input::Bytes(bytes) => Box::new(Cursor::new(bytes)),
            Input::Stream(stream) => Box::new(stream),
        }
    }
}

/// Stato del documento corrente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocState {
    /// Nessun documento.
    Idle,
    /// Documento iniziato ma senza eventi ancora (l'inizio si manda col primo testo).
    Pending(Format),
    /// Inizio già mandato.
    Open,
}

/// Contesto di un'estrazione: destinatario, limiti, percorso dentro gli archivi.
pub(crate) struct Ctx<'s> {
    sink: &'s mut dyn EventSink,
    budget: Rc<Budget>,
    inner: Vec<String>,
    doc: DocState,
}

impl<'s> Ctx<'s> {
    fn new(sink: &'s mut dyn EventSink, limits: ArchiveLimits) -> Ctx<'s> {
        Ctx {
            sink,
            budget: Rc::new(Budget::new(limits)),
            inner: Vec::new(),
            doc: DocState::Idle,
        }
    }

    /// I contatori del file radice.
    pub fn budget(&self) -> Rc<Budget> {
        Rc::clone(&self.budget)
    }

    /// I limiti configurati.
    pub fn limits(&self) -> &ArchiveLimits {
        self.budget.limits()
    }

    /// Numero di archivi attraversati per arrivare al documento corrente.
    pub fn depth(&self) -> usize {
        self.inner.len()
    }

    /// Entra nella voce `label` di un archivio.
    pub fn enter(&mut self, label: String) {
        self.inner.push(label);
    }

    /// Esce dalla voce corrente.
    pub fn leave(&mut self) {
        self.inner.pop();
    }

    /// Un'unità di testo del documento corrente; il testo vuoto o di soli spazi si ignora.
    /// Un testo oltre [`MAX_UNIT_TEXT_BYTES`] si divide in più unità.
    pub fn unit(&mut self, location: Location, text: &str) -> Result<(), Stopped> {
        if text.trim().is_empty() {
            return Ok(());
        }
        self.ensure_started()?;
        let mut rest = text;
        let mut location = location;
        while !rest.is_empty() {
            let cut = soft_cut(rest, MAX_UNIT_TEXT_BYTES);
            let (piece, tail) = rest.split_at(cut);
            self.sink.emit(ExtractEvent::Unit(TextUnit {
                location: location.clone(),
                text: piece.to_owned(),
            }))?;
            let newlines = memchr::memchr_iter(b'\n', piece.as_bytes()).count();
            location = line_location(&location, u32::try_from(newlines).unwrap_or(u32::MAX));
            rest = tail;
        }
        Ok(())
    }

    fn begin(&mut self, format: Format) {
        self.doc = DocState::Pending(format);
    }

    fn ensure_started(&mut self) -> Result<(), Stopped> {
        if let DocState::Pending(format) = self.doc {
            self.sink.emit(ExtractEvent::DocumentStart {
                inner: self.inner.clone(),
                format,
            })?;
            self.doc = DocState::Open;
        }
        Ok(())
    }

    /// Chiude il documento letto per intero (anche se vuoto).
    fn finish(&mut self) -> Result<(), Stopped> {
        if self.doc == DocState::Idle {
            return Ok(());
        }
        self.ensure_started()?;
        self.doc = DocState::Idle;
        self.sink.emit(ExtractEvent::DocumentEnd)
    }

    /// Chiude il documento interrotto: la fine solo se l'inizio è già partito.
    fn abandon(&mut self) -> Result<(), Stopped> {
        let was_open = self.doc == DocState::Open;
        self.doc = DocState::Idle;
        if was_open {
            self.sink.emit(ExtractEvent::DocumentEnd)?;
        }
        Ok(())
    }

    /// Un avviso per il documento corrente, fuori da ogni coppia inizio-fine.
    pub fn warn(&mut self, kind: WarningKind) -> Result<(), Stopped> {
        self.abandon()?;
        self.sink.emit(ExtractEvent::Warning {
            inner: self.inner.clone(),
            kind,
        })
    }

    /// Il documento corrente è saltato.
    pub fn skip(&mut self, reason: SkipReason) -> Result<(), Stopped> {
        self.abandon()?;
        self.sink.emit(ExtractEvent::Skipped {
            inner: self.inner.clone(),
            reason,
        })
    }

    /// L'avviso o il "saltato" di un errore, per il documento corrente.
    pub fn report(&mut self, error: ExtractError) -> Result<(), Stopped> {
        let error = normalize(error);
        if let Some(reason) = SkipReason::from_error(&error) {
            return self.skip(reason);
        }
        let kind = WarningKind::from_error(&error).unwrap_or(WarningKind::Corrupt);
        self.warn(kind)
    }
}

/// Legge un documento dichiarato `declared` dall'estensione: verifica il contenuto, chiama
/// l'estrattore del registro, trasforma gli errori in avvisi. Solo [`Stopped`] risale.
pub(crate) fn read_document(
    ctx: &mut Ctx<'_>,
    declared: Format,
    input: Input<'_>,
) -> Result<(), Stopped> {
    match run_document(ctx, declared, input) {
        Ok(()) => Ok(()),
        Err(Fail::Stopped) => Err(Stopped),
        Err(Fail::Error(error)) => ctx.report(error),
    }
}

fn run_document(ctx: &mut Ctx<'_>, declared: Format, mut input: Input<'_>) -> Result<(), Fail> {
    let found = input.signature()?;
    let format = match format::verify(declared, found) {
        Verdict::Read(format) => format,
        Verdict::EncryptedOoxmlCandidate => {
            return Err(Fail::Error(ooxml::classify_compound(ctx, input)));
        }
        Verdict::Mismatch => return Err(Fail::Error(ExtractError::ContentMismatch)),
    };
    let spec = format::spec(format);
    if spec.kind == Kind::Document {
        ctx.begin(format);
    }
    // Le librerie dei formati ricevono input ostile: un loro panic non deve fermare gli altri
    // documenti. Lo stato del contesto resta coerente (al più un documento da chiudere).
    let extracted = catch_unwind(AssertUnwindSafe(|| (spec.extract)(ctx, input)));
    match extracted {
        Ok(result) => result?,
        Err(_) => {
            return Err(Fail::Error(ExtractError::Corrupt {
                reason: "panic durante l'estrazione".to_owned(),
            }));
        }
    }
    ctx.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::SequenceCheck;

    fn run(name: &str, bytes: &[u8]) -> Vec<ExtractEvent> {
        let mut events = Vec::new();
        let result = extract_bytes(
            name,
            bytes.to_vec(),
            &ExtractOptions::default(),
            &mut events,
        );
        assert_eq!(result, Ok(()));
        let mut check = SequenceCheck::new();
        for event in &events {
            assert_eq!(check.accept(event), Ok(()), "{events:?}");
        }
        assert_eq!(check.finish(), Ok(()));
        events
    }

    #[test]
    fn text_by_default_with_line_units() {
        let events = run("note", b"prima\nseconda aragosta\n");
        assert_eq!(
            events,
            [
                ExtractEvent::DocumentStart {
                    inner: Vec::new(),
                    format: Format::Text
                },
                ExtractEvent::Unit(TextUnit {
                    location: Location::Line(1),
                    text: "prima\nseconda aragosta\n".to_owned()
                }),
                ExtractEvent::DocumentEnd,
            ]
        );
    }

    #[test]
    fn binary_text_is_skipped() {
        let events = run("dati.bin", b"\x00\x01\x02\x00");
        assert_eq!(
            events,
            [ExtractEvent::Skipped {
                inner: Vec::new(),
                reason: SkipReason::Binary
            }]
        );
    }

    #[test]
    fn content_mismatch_is_a_warning() {
        let events = run("finta.pdf", b"\x89PNG\r\n\x1a\nresto");
        assert_eq!(
            events,
            [ExtractEvent::Warning {
                inner: Vec::new(),
                kind: WarningKind::ContentMismatch
            }]
        );
        // Un .docx che in realtà è RTF: caso innocuo solo quando ci sarà l'estrattore RTF.
        let events = run("lettera.docx", b"{\\rtf1\\ansi aragosta}");
        assert_eq!(
            events,
            [ExtractEvent::Warning {
                inner: Vec::new(),
                kind: WarningKind::ContentMismatch
            }]
        );
    }

    #[test]
    fn empty_document_is_still_a_document() {
        assert_eq!(
            run("vuoto.txt", b""),
            [
                ExtractEvent::DocumentStart {
                    inner: Vec::new(),
                    format: Format::Text
                },
                ExtractEvent::DocumentEnd
            ]
        );
    }

    #[test]
    fn missing_file_is_not_found() {
        let mut events = Vec::new();
        let path = Path::new("questo-file-non-esiste.pdf");
        let result = extract_path(path, &ExtractOptions::default(), &mut events);
        assert_eq!(result, Ok(()));
        assert_eq!(
            events,
            [ExtractEvent::Warning {
                inner: Vec::new(),
                kind: WarningKind::NotFound
            }]
        );
    }

    #[test]
    fn long_units_are_split_with_line_numbers() {
        let mut events: Vec<ExtractEvent> = Vec::new();
        let mut sink: &mut dyn EventSink = &mut events;
        let mut ctx = Ctx::new(&mut sink, ArchiveLimits::default());
        ctx.begin(Format::Text);
        let line = "a".repeat(MAX_UNIT_TEXT_BYTES - 1);
        let text = format!("{line}\n{line}\nfine");
        assert_eq!(ctx.unit(Location::Line(10), &text), Ok(()));
        assert_eq!(ctx.finish(), Ok(()));
        let units: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ExtractEvent::Unit(u) => Some((u.location.clone(), u.text.len())),
                _ => None,
            })
            .collect();
        assert!(units.iter().all(|(_, len)| *len <= MAX_UNIT_TEXT_BYTES));
        assert_eq!(units.first().map(|u| u.0.clone()), Some(Location::Line(10)));
        assert_eq!(units.last().map(|u| u.0.clone()), Some(Location::Line(12)));
        let total: usize = units.iter().map(|u| u.1).sum();
        assert_eq!(total, text.len());
    }

    /// Un estrattore che va in panic: avviso "danneggiato", sequenza valida.
    #[test]
    fn panic_becomes_corrupt() {
        fn boom(ctx: &mut Ctx<'_>, _input: Input<'_>) -> Result<(), Fail> {
            ctx.unit(Location::Paragraph(1), "prima del panic")?;
            panic!("libreria in panic");
        }
        let mut events: Vec<ExtractEvent> = Vec::new();
        let mut sink: &mut dyn EventSink = &mut events;
        let mut ctx = Ctx::new(&mut sink, ArchiveLimits::default());
        ctx.begin(Format::Docx);
        let caught = catch_unwind(AssertUnwindSafe(|| {
            boom(&mut ctx, Input::Bytes(Vec::new()))
        }));
        assert!(caught.is_err());
        let reported = ctx.report(ExtractError::Corrupt {
            reason: String::new(),
        });
        assert_eq!(reported, Ok(()));
        assert_eq!(events.len(), 4);
        assert_eq!(events[2], ExtractEvent::DocumentEnd);
    }
}
