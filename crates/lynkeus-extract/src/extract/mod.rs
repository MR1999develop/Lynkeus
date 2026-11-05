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
//!
//! [`extract_entry`] attraversa gli archivi allo stesso modo (stesse etichette, stessi
//! limiti) ma legge solo le voci sul percorso cercato e consegna i byte originali della voce
//! cercata, a pezzi di al più [`ENTRY_CHUNK_BYTES`].

pub(crate) mod archive;
pub(crate) mod budget;
pub(crate) mod cfbfile;
pub(crate) mod eml;
pub(crate) mod epub;
pub(crate) mod html;
pub(crate) mod ooxml;
pub(crate) mod opendoc;
pub(crate) mod pdf;
pub(crate) mod plain;
pub(crate) mod ppt;
pub(crate) mod rtf;
pub(crate) mod sevenz;
pub(crate) mod sheet;
pub(crate) mod sheet_memory;
pub(crate) mod tarball;
pub(crate) mod word97;
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
use crate::limits::{ArchiveLimits, LimitKind};
use crate::platform;
use crate::text::soft_cut;

use budget::{Budget, LimitedReader, classify_io, normalize, read_all};

/// Testo massimo di un'unità: le più lunghe si dividono.
pub const MAX_UNIT_TEXT_BYTES: usize = 1024 * 1024;

/// Byte massimi di un pezzo di una voce estratta con [`extract_entry`]: con l'intestazione
/// del messaggio resta ben sotto il massimo di un frame del protocollo.
pub const ENTRY_CHUNK_BYTES: usize = 1024 * 1024;

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
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut ctx = Ctx::new(&mut sink, options.limits, name);
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
    let mut ctx = Ctx::new(&mut sink, options.limits, name.to_owned());
    let declared = Format::for_name(name).unwrap_or(Format::Text);
    read_document(&mut ctx, declared, Input::Bytes(bytes))
}

/// Destinatario dei byte di una voce estratta con [`extract_entry`].
pub trait EntrySink {
    /// Riceve il pezzo seguente dei byte della voce (mai vuoto, al più
    /// [`ENTRY_CHUNK_BYTES`]); [`Stopped`] ferma l'estrazione.
    fn bytes(&mut self, chunk: &[u8]) -> Result<(), Stopped>;
}

impl EntrySink for Vec<u8> {
    fn bytes(&mut self, chunk: &[u8]) -> Result<(), Stopped> {
        self.extend_from_slice(chunk);
        Ok(())
    }
}

/// Com'è finita l'estrazione di una voce.
///
/// Viaggia nel protocollo: nuove varianti solo in fondo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryOutcome {
    /// Tutti i byte della voce sono stati consegnati.
    Extracted {
        /// Quanti.
        bytes: u64,
    },
    /// La voce non si è potuta estrarre: i byte già consegnati non valgono. `NotFound` se
    /// la voce non c'è (o non è un file regolare); altrimenti il motivo dell'avviso che la
    /// voce, o un archivio sul suo percorso, avrebbe nella ricerca.
    Failed(WarningKind),
}

/// Estrae i byte originali della voce `inner` (etichette dall'esterno all'interno, come in
/// [`ExtractEvent::DocumentStart`](crate::ExtractEvent::DocumentStart)) del file `path` e li
/// manda a `sink`.
///
/// Gli archivi si attraversano come in [`extract_path`], con gli stessi limiti di `options`;
/// si leggono solo le voci sul percorso di `inner`. Oltre `max_bytes` byte la voce non si
/// estrae ([`LimitKind::EntrySize`]). Restituisce [`Stopped`] solo se è `sink` a chiedere di
/// fermarsi. Legge solo `path` (e nulla scrive su disco).
pub fn extract_entry(
    path: &Path,
    inner: &[String],
    max_bytes: u64,
    options: &ExtractOptions,
    sink: &mut dyn EntrySink,
) -> Result<EntryOutcome, Stopped> {
    if inner.is_empty() {
        return Ok(EntryOutcome::Failed(WarningKind::NotFound));
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let declared = Format::for_path(path).unwrap_or(Format::Text);
    let mut watch = PathWarnings {
        target: inner,
        first: None,
    };
    let (result, outcome) = {
        let mut ctx = Ctx::new(&mut watch, options.limits, name);
        ctx.target = Some(Target {
            inner,
            max_bytes,
            out: sink,
            outcome: None,
        });
        let result = match platform::open_for_reading(path) {
            Ok(file) => read_document(&mut ctx, declared, Input::File(file)),
            Err(error) => ctx.report(ExtractError::from_io(error)),
        };
        (result, ctx.target.and_then(|target| target.outcome))
    };
    match (outcome, result) {
        (Some(outcome), _) => Ok(outcome),
        (None, Err(Stopped)) => Err(Stopped),
        (None, Ok(())) => Ok(EntryOutcome::Failed(
            watch.first.unwrap_or(WarningKind::NotFound),
        )),
    }
}

/// Il primo avviso per la voce cercata o per un archivio sul suo percorso: il motivo per
/// cui la voce non si è trovata.
struct PathWarnings<'a> {
    target: &'a [String],
    first: Option<WarningKind>,
}

impl EventSink for PathWarnings<'_> {
    fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped> {
        let kind = match event {
            ExtractEvent::Warning { inner, kind } if self.target.starts_with(&inner) => kind,
            ExtractEvent::Skipped {
                inner,
                reason: SkipReason::TooLarge,
            } if inner == self.target => WarningKind::LimitExceeded(LimitKind::EntrySize),
            _ => return Ok(()),
        };
        self.first.get_or_insert(kind);
        Ok(())
    }
}

/// Il motivo per cui una voce non si è potuta estrarre, da un errore di lettura.
fn entry_warning(error: ExtractError) -> WarningKind {
    let error = normalize(error);
    match SkipReason::from_error(&error) {
        Some(SkipReason::TooLarge) => WarningKind::LimitExceeded(LimitKind::EntrySize),
        Some(_) => WarningKind::Corrupt,
        None => WarningKind::from_error(&error).unwrap_or(WarningKind::Corrupt),
    }
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

/// Che cosa fare della voce corrente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryRole {
    /// Leggerla come documento (o archivio da attraversare).
    Read,
    /// Consegnarne i byte: è la voce cercata da [`extract_entry`].
    Capture,
    /// Non leggerla: fuori dal percorso cercato.
    Skip,
}

/// La voce cercata da [`extract_entry`].
struct Target<'s> {
    inner: &'s [String],
    max_bytes: u64,
    out: &'s mut dyn EntrySink,
    /// `Some` quando la voce è stata trovata (estratta o no): il resto non si legge.
    outcome: Option<EntryOutcome>,
}

/// Contesto di un'estrazione: destinatario, limiti, percorso dentro gli archivi.
pub(crate) struct Ctx<'s> {
    sink: &'s mut dyn EventSink,
    budget: Rc<Budget>,
    /// Nome del file radice (ultimo componente), per i formati che ne ricavano il nome del
    /// contenuto (`appunti.txt.gz` contiene `appunti.txt`).
    root_name: String,
    inner: Vec<String>,
    doc: DocState,
    /// `Some` con [`extract_entry`]: si cerca una voce invece di estrarre il testo.
    target: Option<Target<'s>>,
}

impl<'s> Ctx<'s> {
    fn new(sink: &'s mut dyn EventSink, limits: ArchiveLimits, root_name: String) -> Ctx<'s> {
        Ctx {
            sink,
            budget: Rc::new(Budget::new(limits)),
            root_name,
            inner: Vec::new(),
            doc: DocState::Idle,
            target: None,
        }
    }

    /// Che cosa fare della voce corrente (sempre [`EntryRole::Read`] fuori da
    /// [`extract_entry`]).
    pub fn entry_role(&self) -> EntryRole {
        let Some(target) = &self.target else {
            return EntryRole::Read;
        };
        if target.outcome.is_some() {
            EntryRole::Skip
        } else if self.inner.as_slice() == target.inner {
            EntryRole::Capture
        } else if target.inner.starts_with(&self.inner) {
            EntryRole::Read
        } else {
            EntryRole::Skip
        }
    }

    /// Consegna i byte di `reader` (la voce cercata, già limitata come le altre voci) e
    /// ferma l'attraversamento con [`Stopped`]: [`extract_entry`] distingue questo arresto
    /// da quello chiesto dal destinatario perché l'esito è registrato.
    pub fn capture(&mut self, reader: &mut dyn Read) -> Result<(), Stopped> {
        let Some(target) = self.target.as_mut() else {
            return Ok(());
        };
        let mut buf = vec![0_u8; ENTRY_CHUNK_BYTES];
        let mut total = 0_u64;
        let outcome = loop {
            match reader.read(&mut buf) {
                Ok(0) => break EntryOutcome::Extracted { bytes: total },
                Ok(read) => {
                    total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
                    if total > target.max_bytes {
                        break EntryOutcome::Failed(WarningKind::LimitExceeded(
                            LimitKind::EntrySize,
                        ));
                    }
                    target.out.bytes(buf.get(..read).unwrap_or_default())?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => break EntryOutcome::Failed(entry_warning(classify_io(error))),
            }
        };
        target.outcome = Some(outcome);
        Err(Stopped)
    }

    /// Il nome del documento corrente: l'etichetta della voce, o il nome del file radice.
    pub fn current_name(&self) -> &str {
        self.inner
            .last()
            .map_or(self.root_name.as_str(), String::as_str)
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

    /// Esegue `read` dentro la voce `label` di un archivio e ne esce comunque.
    ///
    /// Un panic in `read` (una libreria che legge la voce, la voce stessa) diventa l'avviso
    /// "danneggiato" di quella voce: l'etichetta si toglie sempre, così le voci seguenti
    /// hanno il loro percorso giusto. Quando si estrae una sola voce
    /// ([`extract_entry`]), le voci fuori dal percorso cercato non si leggono: `read` non si
    /// chiama e il risultato è `T::default()`.
    pub fn within<T: Default>(
        &mut self,
        label: String,
        read: impl FnOnce(&mut Ctx<'s>) -> Result<T, Stopped>,
    ) -> Result<T, Stopped> {
        let depth = self.inner.len();
        self.inner.push(label);
        let result = if self.entry_role() == EntryRole::Skip {
            Ok(T::default())
        } else {
            match catch_unwind(AssertUnwindSafe(|| read(&mut *self))) {
                Ok(result) => result,
                Err(_) => {
                    self.inner.truncate(depth + 1);
                    self.report(panic_error()).map(|()| T::default())
                }
            }
        };
        self.inner.truncate(depth);
        result
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
    // documenti. Le voci rimaste aperte dal panic si tolgono: l'avviso va a questo documento.
    let depth = ctx.inner.len();
    let extracted = catch_unwind(AssertUnwindSafe(|| (spec.extract)(ctx, input)));
    match extracted {
        Ok(result) => result?,
        Err(_) => {
            ctx.inner.truncate(depth);
            return Err(Fail::Error(panic_error()));
        }
    }
    ctx.finish()?;
    Ok(())
}

/// L'errore di un panic intercettato durante la lettura di un documento o di una voce.
fn panic_error() -> ExtractError {
    ExtractError::Corrupt {
        reason: "panic durante l'estrazione".to_owned(),
    }
}

/// Oracolo dei target di fuzzing che danno paragrafi: ognuno ha testo oltre agli spazi,
/// nessun carattere di controllo salvo tabulazione e a capo, e in tutto al più `max_chars`
/// caratteri (un parser non inventa testo: ogni carattere viene da almeno un'unità
/// dell'input). La prima violazione trovata, se c'è.
#[cfg(any(test, feature = "fuzzing"))]
pub(crate) fn paragraph_violation(paragraphs: &[String], max_chars: usize) -> Option<&'static str> {
    if paragraphs.iter().any(|p| p.trim().is_empty()) {
        return Some("paragrafo vuoto");
    }
    let control = |c: char| c.is_control() && c != '\t' && c != '\n';
    if paragraphs.iter().any(|p| p.chars().any(control)) {
        return Some("carattere di controllo in un paragrafo");
    }
    let chars = paragraphs
        .iter()
        .map(|p| p.chars().count())
        .fold(0_usize, usize::saturating_add);
    if chars > max_chars {
        return Some("più caratteri di quanti l'input ne possa dare");
    }
    None
}

/// Punto d'iniezione dei test: un panic forzato quando si legge la voce con un certo nome,
/// per provare che un panic dentro un archivio non sposta le etichette delle voci seguenti.
#[cfg(test)]
pub(crate) mod test_hook {
    use std::cell::RefCell;

    thread_local! {
        static PANIC_ON: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    /// Da ora, in questo thread, la lettura della voce `name` va in panic (`None` = mai).
    pub fn panic_on(name: Option<&str>) {
        PANIC_ON.with(|cell| *cell.borrow_mut() = name.map(str::to_owned));
    }

    /// Va in panic se `name` è la voce scelta con [`panic_on`].
    pub fn check(name: &str) {
        let hit = PANIC_ON.with(|cell| cell.borrow().as_deref() == Some(name));
        assert!(!hit, "panic forzato dal test sulla voce {name}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::SequenceCheck;

    /// L'oracolo dei paragrafi riconosce ogni violazione (e un oracolo che non scatta mai
    /// non prova nulla).
    #[test]
    fn paragraph_oracle_partitions() {
        let owned = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(paragraph_violation(&owned(&["città", "a\tb\nc"]), 11), None);
        assert_eq!(paragraph_violation(&owned(&[]), 0), None);
        assert!(paragraph_violation(&owned(&["testo", " \t"]), 100).is_some());
        assert!(paragraph_violation(&owned(&["a\rb"]), 100).is_some());
        assert!(paragraph_violation(&owned(&["a\u{0}b"]), 100).is_some());
        // On e off point del numero di caratteri (non di byte: "città" ne ha 5).
        assert_eq!(paragraph_violation(&owned(&["città"]), 5), None);
        assert!(paragraph_violation(&owned(&["città"]), 4).is_some());
    }

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
        // Un .docx che in realtà è RTF: il caso innocuo vale solo per i .doc.
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
        let mut ctx = Ctx::new(&mut sink, ArchiveLimits::default(), String::new());
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
        let mut ctx = Ctx::new(&mut sink, ArchiveLimits::default(), String::new());
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
