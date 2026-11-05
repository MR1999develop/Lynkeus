// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Pool dei processi estrattori: pochi processi longevi (ADR 0001), avviati al primo
//! documento, ciascuno con un file alla volta.
//!
//! Per ogni file [`ExtractorPool::extract`] prende un processo libero (o ne avvia uno, fino a
//! [`ExtractorPool::size`]), gli manda una [`Request::Extract`] e passa gli eventi ricevuti a
//! un [`EventSink`] man mano che arrivano. Ogni processo ha due thread: uno scrive le
//! richieste sul suo stdin, uno legge lo stdout e consegna i messaggi già decodificati; chi
//! chiama aspetta con scadenza su entrambi, quindi può sempre terminare il processo (anche
//! uno che non legge la richiesta).
//!
//! Il **thread del pool** (uno per pool, atteso da [`ExtractorPool::shutdown`]) avvia i
//! processi e raccoglie quelli tolti dal pool: chi chiama non aspetta mai la loro uscita.
//! Su Linux il figlio sorveglia il processo padre (`lynkeus_extract::worker::watch_parent`),
//! non il thread che l'ha avviato: la sua vita non dipende da quale thread lo avvia.
//!
//! Regole (il padre non si fida del figlio):
//!
//! - **tempo**: oltre il timeout il processo è terminato subito e sostituito
//!   ([`WorkerFailure::Timeout`]); il tempo in cui chi riceve gli eventi aspetta altri
//!   (l'interfaccia che non legge) non conta ([`ExtractBudget`]);
//! - **annulla** (o il destinatario che chiede di fermarsi): stdin chiuso, che per il figlio
//!   è la richiesta di uscire, e risposta subito [`ExtractEnd::Stopped`]; il thread del pool
//!   termina il processo se non esce entro [`STOP_GRACE`];
//! - **protocollo**: un frame non valido (anche byte estranei su stdout), un `id` diverso da
//!   quello della richiesta, un [`Response::Rejected`], una sequenza di eventi sbagliata
//!   ([`SequenceCheck`]), un'unità oltre [`MAX_UNIT_TEXT_BYTES`] o voci annidate oltre la
//!   profondità massima chiesta terminano il processo ([`WorkerFailure::ProtocolViolation`]):
//!   il padre rivaluta i limiti che il figlio dovrebbe rispettare;
//! - **morte** a metà file: [`WorkerFailure::Died`] con il motivo d'uscita, "memoria
//!   superata" compresa dove il sistema la riconosce;
//! - un processo libero con messaggi in sospeso, o già morto, non si riusa; se la prima
//!   risposta a una richiesta è di una richiesta precedente (ha parlato da libero), il
//!   processo si scarta e la richiesta va a un altro.
//!
//! [`ExtractorPool::extract_entry`] chiede invece i byte di una voce d'archivio
//! ([`Request::ExtractEntry`]), con le stesse regole: i pezzi devono avere l'`id` giusto,
//! stare entro [`ENTRY_CHUNK_BYTES`] e, sommati, entro il massimo chiesto e il totale
//! dichiarato alla fine.
//!
//! I percorsi si rendono assoluti prima di mandarli: il processo estrattore ha come cartella
//! corrente quella del suo eseguibile.
//!
//! Il pool non sa nulla della ricerca né della cache: riceve un percorso e restituisce
//! eventi (`documents` li usa entrambi).

use std::io::{self, BufReader, BufWriter, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, SendError, Sender};
use lynkeus_extract::protocol::{
    EntryRequest, ExtractRequest, FrameReader, FrameWriter, ProtocolError, Request, RequestId,
    Response,
};
use lynkeus_extract::{
    ENTRY_CHUNK_BYTES, EntryOutcome, EntrySink, EventSink, ExtractEvent, ExtractOptions, LimitKind,
    MAX_UNIT_TEXT_BYTES, SequenceCheck,
};

use crate::event::WarningKind;
use crate::launcher::{LaunchedWorker, WorkerExit, WorkerLauncher, WorkerProcess};

/// Processi al massimo per pool.
pub const MAX_POOL_SIZE: usize = 4;

/// Dopo la richiesta di fermarsi (stdin chiuso), quanto si aspetta prima di terminare; poi
/// altrettanto per raccogliere il processo terminato e i suoi thread.
pub const STOP_GRACE: Duration = Duration::from_millis(500);

/// Ogni quanto chi aspetta un messaggio ricontrolla l'annullamento.
const STOP_RECHECK: Duration = Duration::from_millis(25);

/// Ogni quanto il thread del pool ricontrolla i processi che sta chiudendo (la libreria
/// standard non ha un'attesa con scadenza su un processo).
const REAP_TICK: Duration = Duration::from_millis(10);

/// Messaggi letti e non ancora consumati per processo: con unità fino a 1 MiB, al più circa
/// 16 MiB in memoria.
const MESSAGE_QUEUE: usize = 16;

/// Tentativi per un file quando il processo non prende la richiesta (morto da libero, o la
/// prima risposta è di una richiesta precedente).
const SEND_ATTEMPTS: usize = 2;

/// Dopo [`ExtractorPool::shutdown`], quanto il thread del pool aspetta ancora i processi in
/// uso (che chi li usa toglie al primo controllo dell'annullamento) prima di uscire.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);

/// Un'attesa così lunga che nessuna estrazione la raggiunge: tetto delle scadenze.
const FAR_FUTURE: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// Processi del pool per questo computer: metà dei core, da 1 a [`MAX_POOL_SIZE`].
pub fn default_pool_size() -> NonZeroUsize {
    let cores = thread::available_parallelism().map_or(2, NonZeroUsize::get);
    NonZeroUsize::new((cores / 2).clamp(1, MAX_POOL_SIZE)).unwrap_or(NonZeroUsize::MIN)
}

/// Come è finita l'estrazione di un file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractEnd {
    /// Tutti gli eventi sono arrivati, in una sequenza valida.
    Completed,
    /// Chi chiama ha chiesto di fermarsi (annullamento o destinatario fermato).
    Stopped,
    /// Il processo non ha finito il file.
    Failed(WorkerFailure),
}

/// Come è finita l'estrazione dei byte di una voce d'archivio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryEnd {
    /// Tutti i byte della voce sono arrivati.
    Extracted {
        /// Quanti.
        bytes: u64,
    },
    /// Il processo estrattore non ha potuto estrarre la voce (assente, cifrata, oltre i
    /// limiti, …): i byte già consegnati non valgono.
    NotExtracted(lynkeus_extract::WarningKind),
    /// Chi chiama ha chiesto di fermarsi.
    Stopped,
    /// Il processo non ha finito la richiesta.
    Failed(WorkerFailure),
}

/// Perché un processo estrattore non ha finito un file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerFailure {
    /// Oltre il tempo massimo: terminato.
    Timeout,
    /// Uscito a metà file.
    Died(WorkerExit),
    /// Messaggi non validi o fuori sequenza: terminato.
    ProtocolViolation,
    /// Nessun processo avviabile (eseguibile mancante, risorse finite) o pool chiuso.
    Unavailable,
}

impl WorkerFailure {
    /// Il motivo dell'avviso per il documento.
    pub fn warning_kind(self) -> WarningKind {
        match self {
            WorkerFailure::Timeout => WarningKind::Timeout,
            WorkerFailure::Died(WorkerExit::MemoryExceeded) => {
                WarningKind::LimitExceeded(LimitKind::Memory)
            }
            WorkerFailure::Died(_) | WorkerFailure::ProtocolViolation => WarningKind::Corrupt,
            WorkerFailure::Unavailable => WarningKind::ExtractorUnavailable,
        }
    }
}

/// Il tempo concesso a un'estrazione: `timeout`, più il tempo (cumulativo, letto da
/// `paused`) in cui chi riceve gli eventi ha aspettato altri. Il processo non ne ha colpa:
/// la coda verso di lui era piena perché nessuno la svuotava.
#[derive(Clone, Copy)]
pub(crate) struct ExtractBudget<'a> {
    timeout: Duration,
    paused: &'a dyn Fn() -> Duration,
}

impl<'a> ExtractBudget<'a> {
    /// Solo `timeout`.
    pub(crate) fn new(timeout: Duration) -> ExtractBudget<'static> {
        ExtractBudget {
            timeout,
            paused: &NO_PAUSE,
        }
    }

    /// `timeout` più il tempo restituito da `paused`.
    pub(crate) fn with_pause(timeout: Duration, paused: &'a dyn Fn() -> Duration) -> Self {
        ExtractBudget { timeout, paused }
    }
}

/// Nessuna pausa: per chi non misura le attese del destinatario.
static NO_PAUSE: fn() -> Duration = || Duration::ZERO;

/// La scadenza di un'estrazione iniziata in `start`.
struct Deadline<'a> {
    start: Instant,
    budget: &'a ExtractBudget<'a>,
}

impl Deadline<'_> {
    /// Il tempo che resta ora (zero se scaduta).
    fn left(&self) -> Duration {
        let allowed = self
            .budget
            .timeout
            .saturating_add((self.budget.paused)())
            .min(FAR_FUTURE);
        let end = self.start.checked_add(allowed).unwrap_or(self.start);
        end.saturating_duration_since(Instant::now())
    }
}

/// Un messaggio dal thread che legge lo stdout di un processo.
#[derive(Debug)]
enum FromWorker {
    Response(Response),
    /// Frame non valido: il thread smette di leggere.
    Violation,
    /// Stdout chiuso (anche a metà frame): il processo è uscito o sta uscendo.
    Closed,
}

/// Esito della consegna di una richiesta al thread che scrive su stdin.
enum Sent {
    /// Scritta per intero: l'id della richiesta.
    Delivered(RequestId),
    /// Il canale verso il processo è rotto (morto da libero): si prova con un altro.
    Broken,
    /// Fermati o scaduti prima che la richiesta fosse scritta.
    Halted(Halt),
}

/// Un processo del pool con i suoi canali e i suoi due thread.
struct Worker {
    /// Richieste verso il thread che scrive su stdin; `None` dopo la richiesta di fermarsi
    /// (il thread chiude stdin appena finita la scrittura in corso).
    requests: Option<Sender<Request>>,
    /// Esito di ogni scrittura: vero se la richiesta è arrivata intera nel canale.
    written: Receiver<bool>,
    messages: Receiver<FromWorker>,
    process: Box<dyn WorkerProcess>,
    /// Il thread che scrive e quello che legge: li raccoglie il thread del pool.
    threads: Vec<JoinHandle<()>>,
    next_id: RequestId,
}

impl Worker {
    /// Avvia i thread di scrittura e di lettura per un processo appena avviato.
    fn start(launched: LaunchedWorker) -> Option<Worker> {
        let LaunchedWorker {
            stdin,
            stdout,
            mut process,
        } = launched;
        let (messages_tx, messages) = crossbeam_channel::bounded(MESSAGE_QUEUE);
        let (requests, requests_rx) = crossbeam_channel::bounded(1);
        let (written_tx, written) = crossbeam_channel::bounded(1);
        let reader = thread::Builder::new()
            .name("lynkeus-estrattore-lettura".to_owned())
            .spawn(move || read_messages(stdout, &messages_tx));
        let writer = thread::Builder::new()
            .name("lynkeus-estrattore-scrittura".to_owned())
            .spawn(move || write_requests(stdin, &requests_rx, &written_tx));
        let (Ok(reader), Ok(writer)) = (reader, writer) else {
            // Il processo terminato chiude i canali: un thread già partito finisce da sé.
            let _killed = process.kill();
            return None;
        };
        Some(Worker {
            requests: Some(requests),
            written,
            messages,
            process,
            threads: vec![reader, writer],
            next_id: 1,
        })
    }

    /// Vero se il processo, libero, ha mandato qualcosa (o è uscito): non va riusato.
    fn has_pending(&self) -> bool {
        !self.messages.is_empty()
    }

    /// Manda la richiesta costruita da `request` con un id nuovo e aspetta che sia scritta,
    /// fino a `deadline` e finché `stop` è falso.
    fn send(
        &mut self,
        request: impl FnOnce(RequestId) -> Request,
        deadline: &Deadline<'_>,
        stop: &dyn Fn() -> bool,
    ) -> Sent {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let Some(requests) = &self.requests else {
            return Sent::Broken;
        };
        // Una richiesta alla volta: il canale è vuoto, l'invio non aspetta mai.
        if requests.try_send(request(id)).is_err() {
            return Sent::Broken;
        }
        loop {
            if stop() {
                return Sent::Halted(Halt::Stopped);
            }
            let left = deadline.left();
            if left.is_zero() {
                return Sent::Halted(Halt::Timeout);
            }
            match self.written.recv_timeout(left.min(STOP_RECHECK)) {
                Ok(true) => return Sent::Delivered(id),
                Ok(false) | Err(RecvTimeoutError::Disconnected) => return Sent::Broken,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    /// Il motivo d'uscita di un processo che ha chiuso lo stdout a metà file.
    fn exit_after_close(&mut self) -> WorkerFailure {
        match self.process.wait_timeout(STOP_GRACE) {
            // Uscita pulita a metà file, o stdout chiuso con il processo vivo: non rispetta
            // il protocollo.
            Ok(Some(WorkerExit::Success) | None) | Err(_) => WorkerFailure::ProtocolViolation,
            Ok(Some(exit)) => WorkerFailure::Died(exit),
        }
    }
}

/// Scrive le richieste su stdin finché il canale delle richieste resta aperto; dice per
/// ognuna se è arrivata. Alla fine stdin si chiude: per il processo è la richiesta di
/// fermarsi.
fn write_requests(
    stdin: Box<dyn Write + Send>,
    requests: &Receiver<Request>,
    written: &Sender<bool>,
) {
    let mut writer = FrameWriter::new(BufWriter::new(stdin));
    for request in requests {
        let delivered = writer.write(&request).and_then(|()| writer.flush()).is_ok();
        if written.send(delivered).is_err() || !delivered {
            return;
        }
    }
}

/// Legge i messaggi dello stdout di un processo finché il canale serve.
fn read_messages(stdout: Box<dyn io::Read + Send>, tx: &Sender<FromWorker>) {
    let mut reader = FrameReader::new(BufReader::new(stdout));
    loop {
        let message = match reader.read::<Response>() {
            Ok(Some(response)) => FromWorker::Response(response),
            // Fine del flusso, anche a metà frame o con la pipe rotta: il processo è uscito
            // (forse terminato dal sistema); il motivo si chiede al processo.
            Ok(None) | Err(ProtocolError::Io(_) | ProtocolError::Truncated) => FromWorker::Closed,
            Err(_) => FromWorker::Violation,
        };
        let last = !matches!(message, FromWorker::Response(_));
        if tx.send(message).is_err() || last {
            return;
        }
    }
}

/// Perché l'attesa di una risposta è finita senza un messaggio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Halt {
    /// Chi chiama ha chiesto di fermarsi.
    Stopped,
    /// Oltre la scadenza.
    Timeout,
    /// Frame non valido o messaggio fuori protocollo.
    Violation,
    /// Stdout chiuso: il processo è uscito o sta uscendo.
    Closed,
}

/// Esito del servizio di una richiesta.
enum Served<T> {
    /// La richiesta è finita così.
    Done(T),
    /// La prima risposta era di una richiesta precedente: il processo è stato scartato, la
    /// richiesta va a un altro.
    Retry,
}

/// Il percorso assoluto di `path`, senza accedere al disco: il processo estrattore ha
/// un'altra cartella corrente e risolverebbe un percorso relativo altrove. Un percorso che
/// non si può rendere assoluto (vuoto) resta com'è: l'estrattore risponderà "non trovato".
pub(crate) fn absolute_path(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}

/// Vero se l'evento sta nei limiti che il figlio deve rispettare: unità entro
/// [`MAX_UNIT_TEXT_BYTES`], voci annidate entro `max_depth` (la profondità chiesta).
fn event_fits(event: &ExtractEvent, max_depth: u32) -> bool {
    let depth_fits = |inner: &[String]| u32::try_from(inner.len()).is_ok_and(|d| d <= max_depth);
    match event {
        ExtractEvent::Unit(unit) => unit.text.len() <= MAX_UNIT_TEXT_BYTES,
        ExtractEvent::DocumentStart { inner, .. }
        | ExtractEvent::Warning { inner, .. }
        | ExtractEvent::Skipped { inner, .. } => depth_fits(inner),
        ExtractEvent::DocumentEnd => true,
    }
}

/// L'id di una risposta, se ne ha uno.
fn response_id(response: &Response) -> Option<RequestId> {
    match response {
        Response::Event { id, .. }
        | Response::Done { id }
        | Response::EntryData { id, .. }
        | Response::EntryEnd { id, .. } => Some(*id),
        Response::Rejected { .. } => None,
    }
}

/// Stato del pool condiviso tra chi chiama e il thread del pool.
struct Shared {
    state: Mutex<PoolState>,
    freed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Libera il posto di un processo uscito dal pool e sveglia chi aspetta.
    fn release_slot(&self) {
        {
            let mut state = self.lock();
            state.live = state.live.saturating_sub(1);
        }
        self.freed.notify_one();
    }
}

/// Stato del pool.
struct PoolState {
    idle: Vec<Worker>,
    /// Processi vivi: liberi, in uso o in chiusura.
    live: usize,
    closed: bool,
}

/// Come chiudere un processo tolto dal pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Retire {
    /// Stdin chiuso; terminato se non esce entro [`STOP_GRACE`].
    Gently,
    /// Terminato subito (scadenza, violazione, già uscito).
    Now,
}

/// Un lavoro per il thread del pool.
enum PoolCommand {
    /// Avvia un processo e manda il risultato.
    Launch(Sender<io::Result<LaunchedWorker>>),
    /// Chiude un processo e libera il suo posto.
    Retire(Worker, Retire),
    /// Finisce le chiusure in corso ed esce.
    Finish,
}

/// Pool di processi estrattori, condivisibile tra thread.
pub struct ExtractorPool {
    shared: Arc<Shared>,
    size: usize,
    commands: Sender<PoolCommand>,
    /// Il thread del pool, atteso da [`ExtractorPool::shutdown`].
    pool_thread: Mutex<Option<JoinHandle<()>>>,
    launches: AtomicU64,
}

impl std::fmt::Debug for ExtractorPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtractorPool")
            .field("size", &self.size)
            .field("launches", &self.launches())
            .finish_non_exhaustive()
    }
}

/// Esito della ricerca di un processo libero.
enum Checkout {
    Ready(Worker),
    End(ExtractEnd),
}

impl ExtractorPool {
    /// Un pool di al più `size` processi avviati con `launcher`; nessun processo parte prima
    /// del primo file. Avvia il thread del pool; se il sistema non lo crea, ogni estrazione
    /// finisce con [`WorkerFailure::Unavailable`].
    pub fn new(launcher: Arc<dyn WorkerLauncher>, size: NonZeroUsize) -> ExtractorPool {
        let shared = Arc::new(Shared {
            state: Mutex::new(PoolState {
                idle: Vec::new(),
                live: 0,
                closed: false,
            }),
            freed: Condvar::new(),
        });
        // Al più un avvio e una chiusura per posto, più la fine.
        let (commands, received) =
            crossbeam_channel::bounded(size.get().saturating_mul(2).saturating_add(1));
        let pool_thread = {
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name("lynkeus-pool".to_owned())
                .spawn(move || run_pool_thread(&*launcher, &received, &shared))
                .ok()
        };
        ExtractorPool {
            shared,
            size: size.get(),
            commands,
            pool_thread: Mutex::new(pool_thread),
            launches: AtomicU64::new(0),
        }
    }

    /// Processi al massimo.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Processi avviati finora (diagnostica e test).
    pub fn launches(&self) -> u64 {
        self.launches.load(Ordering::Relaxed)
    }

    /// Estrae il testo di `path` in un processo del pool e manda gli eventi a `sink` man
    /// mano che arrivano.
    ///
    /// Blocca fino alla fine del file, al più `timeout` da quando il processo è pronto
    /// (l'avvio non conta). `stop` si controlla spesso: quando diventa vero il processo
    /// riceve la richiesta di fermarsi e la funzione restituisce subito
    /// [`ExtractEnd::Stopped`]. Con [`ExtractEnd::Failed`] gli eventi già mandati restano
    /// validi (documenti completi di un archivio), l'ultimo documento aperto no.
    pub fn extract(
        &self,
        path: &Path,
        options: &ExtractOptions,
        timeout: Duration,
        stop: &dyn Fn() -> bool,
        sink: &mut dyn EventSink,
    ) -> ExtractEnd {
        self.extract_within(path, options, &ExtractBudget::new(timeout), stop, sink)
    }

    /// Come [`ExtractorPool::extract`], con il tempo di `budget`.
    pub(crate) fn extract_within(
        &self,
        path: &Path,
        options: &ExtractOptions,
        budget: &ExtractBudget<'_>,
        stop: &dyn Fn() -> bool,
        sink: &mut dyn EventSink,
    ) -> ExtractEnd {
        let path = absolute_path(path);
        let request = |id| {
            Request::Extract(ExtractRequest {
                id,
                path: path.clone(),
                options: *options,
            })
        };
        let max_depth = options.limits.max_depth;
        let mut serve = |worker: Worker, id: RequestId, deadline: &Deadline<'_>| {
            self.serve(worker, id, deadline, max_depth, stop, sink)
        };
        self.run_request(&request, budget, stop, &mut serve)
            .unwrap_or_else(|end| end)
    }

    /// Estrae i byte della voce d'archivio `inner` del file `path` in un processo del pool
    /// e li manda a `sink` man mano che arrivano, con i limiti di `options` e al più
    /// `max_bytes` byte.
    ///
    /// Tempo, annullamento e processi che si comportano male come in
    /// [`ExtractorPool::extract`]. Con un esito diverso da [`EntryEnd::Extracted`] i byte già
    /// consegnati non valgono.
    #[expect(
        clippy::too_many_arguments,
        reason = "gli stessi parametri di `extract` più la voce e il massimo"
    )]
    pub fn extract_entry(
        &self,
        path: &Path,
        inner: &[String],
        max_bytes: u64,
        options: &ExtractOptions,
        timeout: Duration,
        stop: &dyn Fn() -> bool,
        sink: &mut dyn EntrySink,
    ) -> EntryEnd {
        let path = absolute_path(path);
        let request = |id| {
            Request::ExtractEntry(EntryRequest {
                id,
                path: path.clone(),
                inner: inner.to_vec(),
                max_bytes,
                options: *options,
            })
        };
        let budget = ExtractBudget::new(timeout);
        let mut serve = |worker: Worker, id: RequestId, deadline: &Deadline<'_>| {
            self.serve_entry(worker, id, max_bytes, deadline, stop, sink)
        };
        match self.run_request(&request, &budget, stop, &mut serve) {
            Ok(end) => end,
            Err(ExtractEnd::Stopped) => EntryEnd::Stopped,
            Err(ExtractEnd::Failed(failure)) => EntryEnd::Failed(failure),
            Err(ExtractEnd::Completed) => EntryEnd::Failed(WorkerFailure::Unavailable),
        }
    }

    /// Prende un processo, gli manda la richiesta e la serve con `serve`; se il processo non
    /// prende la richiesta ne prova un altro (al più [`SEND_ATTEMPTS`] volte). La scadenza
    /// di `budget` parte quando il processo è pronto: il suo avvio non conta. `Err` con
    /// l'esito se la richiesta non è arrivata a nessun processo.
    fn run_request<T>(
        &self,
        request: &dyn Fn(RequestId) -> Request,
        budget: &ExtractBudget<'_>,
        stop: &dyn Fn() -> bool,
        serve: &mut dyn FnMut(Worker, RequestId, &Deadline<'_>) -> Served<T>,
    ) -> Result<T, ExtractEnd> {
        let mut failure = WorkerFailure::Unavailable;
        for _ in 0..SEND_ATTEMPTS {
            let mut worker = match self.checkout(stop) {
                Checkout::Ready(worker) => worker,
                Checkout::End(end) => return Err(end),
            };
            let deadline = Deadline {
                start: Instant::now(),
                budget,
            };
            match worker.send(request, &deadline, stop) {
                Sent::Delivered(id) => match serve(worker, id, &deadline) {
                    Served::Done(end) => return Ok(end),
                    Served::Retry => failure = WorkerFailure::ProtocolViolation,
                },
                Sent::Broken => {
                    self.retire(worker, Retire::Now);
                    failure = WorkerFailure::Unavailable;
                }
                Sent::Halted(halt) => return Err(self.end_by(worker, halt)),
            }
        }
        Err(ExtractEnd::Failed(failure))
    }

    /// Chiude il pool: i processi liberi ricevono la richiesta di fermarsi e sono terminati
    /// dopo [`STOP_GRACE`]; quelli in uso si chiudono alla fine del loro file. Aspetta il
    /// thread del pool, che chiude anche i processi lasciati entro poco dalle estrazioni in
    /// corso (annullate): al più circa tre secondi. Le estrazioni successive finiscono con
    /// [`WorkerFailure::Unavailable`].
    pub fn shutdown(&self) {
        let idle = {
            let mut state = self.shared.lock();
            state.closed = true;
            std::mem::take(&mut state.idle)
        };
        self.shared.freed.notify_all();
        for worker in idle {
            self.retire(worker, Retire::Gently);
        }
        // Se il thread del pool è già uscito il comando non serve.
        let _asked = self.commands.send(PoolCommand::Finish);
        let pool_thread = self
            .pool_thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        // Un panic del thread del pool lascia solo processi da raccogliere al `Drop`.
        if let Some(pool_thread) = pool_thread {
            let _ended = pool_thread.join();
        }
    }

    /// Un processo libero e pulito, avviandone uno se c'è posto; aspetta se sono tutti in
    /// uso.
    fn checkout(&self, stop: &dyn Fn() -> bool) -> Checkout {
        let mut state = self.shared.lock();
        loop {
            if state.closed {
                return Checkout::End(ExtractEnd::Failed(WorkerFailure::Unavailable));
            }
            if stop() {
                return Checkout::End(ExtractEnd::Stopped);
            }
            if let Some(worker) = state.idle.pop() {
                if !worker.has_pending() {
                    return Checkout::Ready(worker);
                }
                // Ha parlato da libero o è morto: via, e si riprova.
                drop(state);
                self.retire(worker, Retire::Now);
                state = self.shared.lock();
                continue;
            }
            if state.live < self.size {
                state.live += 1;
                drop(state);
                return match self.launch() {
                    Some(worker) => Checkout::Ready(worker),
                    None => {
                        self.shared.release_slot();
                        Checkout::End(ExtractEnd::Failed(WorkerFailure::Unavailable))
                    }
                };
            }
            state = match self.shared.freed.wait_timeout(state, STOP_RECHECK) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    /// Avvia un processo dal thread del pool.
    fn launch(&self) -> Option<Worker> {
        let (reply, launched) = crossbeam_channel::bounded(1);
        self.commands.send(PoolCommand::Launch(reply)).ok()?;
        // Il thread del pool risponde sempre, o chiude il canale uscendo.
        let launched = launched.recv().ok()?.ok()?;
        self.launches.fetch_add(1, Ordering::Relaxed);
        Worker::start(launched)
    }

    /// Rimette un processo tra i liberi (o lo chiude se il pool è chiuso).
    fn give_back(&self, worker: Worker) {
        let mut state = self.shared.lock();
        if state.closed {
            drop(state);
            self.retire(worker, Retire::Gently);
        } else {
            state.idle.push(worker);
            drop(state);
            self.shared.freed.notify_one();
        }
    }

    /// Toglie un processo dal pool: stdin chiuso subito, il resto (attesa, terminazione,
    /// raccolta, posto liberato) lo fa il thread del pool; se non c'è più, chi chiama.
    fn retire(&self, mut worker: Worker, how: Retire) {
        worker.requests = None;
        if let Err(SendError(PoolCommand::Retire(worker, how))) =
            self.commands.send(PoolCommand::Retire(worker, how))
        {
            close_blocking(worker, how, &self.shared);
        }
    }

    /// La prossima risposta del processo, o perché non arriva (fermati, scadenza, frame
    /// non valido, processo uscito).
    fn next_response(
        worker: &Worker,
        deadline: &Deadline<'_>,
        stop: &dyn Fn() -> bool,
    ) -> Result<Response, Halt> {
        loop {
            if stop() {
                return Err(Halt::Stopped);
            }
            let left = deadline.left();
            if left.is_zero() {
                return Err(Halt::Timeout);
            }
            match worker.messages.recv_timeout(left.min(STOP_RECHECK)) {
                Ok(FromWorker::Response(response)) => return Ok(response),
                Ok(FromWorker::Violation) => return Err(Halt::Violation),
                Ok(FromWorker::Closed) | Err(RecvTimeoutError::Disconnected) => {
                    return Err(Halt::Closed);
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    /// Toglie dal pool un processo la cui richiesta finisce per `halt`.
    fn end_by(&self, mut worker: Worker, halt: Halt) -> ExtractEnd {
        match halt {
            Halt::Stopped => {
                self.retire(worker, Retire::Gently);
                ExtractEnd::Stopped
            }
            Halt::Timeout => {
                self.retire(worker, Retire::Now);
                ExtractEnd::Failed(WorkerFailure::Timeout)
            }
            Halt::Violation => {
                self.retire(worker, Retire::Now);
                ExtractEnd::Failed(WorkerFailure::ProtocolViolation)
            }
            Halt::Closed => {
                let failure = worker.exit_after_close();
                self.retire(worker, Retire::Now);
                ExtractEnd::Failed(failure)
            }
        }
    }

    /// Vero se `response`, prima di ogni risposta valida a `id`, è di una richiesta
    /// precedente: il processo ha parlato da libero.
    fn is_stale(response: &Response, id: RequestId, answered: bool) -> bool {
        !answered && response_id(response).is_some_and(|got| got < id)
    }

    /// Riceve le risposte alla richiesta `id` fino alla fine del file.
    fn serve(
        &self,
        worker: Worker,
        id: RequestId,
        deadline: &Deadline<'_>,
        max_depth: u32,
        stop: &dyn Fn() -> bool,
        sink: &mut dyn EventSink,
    ) -> Served<ExtractEnd> {
        let mut sequence = SequenceCheck::new();
        let mut answered = false;
        loop {
            let response = match Self::next_response(&worker, deadline, stop) {
                Ok(response) => response,
                Err(halt) => return Served::Done(self.end_by(worker, halt)),
            };
            if Self::is_stale(&response, id, answered) {
                self.retire(worker, Retire::Now);
                return Served::Retry;
            }
            answered = true;
            match response {
                Response::Event { id: got, event } if got == id => {
                    if !event_fits(&event, max_depth) || sequence.accept(&event).is_err() {
                        return Served::Done(self.end_by(worker, Halt::Violation));
                    }
                    if sink.emit(event).is_err() {
                        return Served::Done(self.end_by(worker, Halt::Stopped));
                    }
                }
                Response::Done { id: got } if got == id => {
                    if sequence.finish().is_err() {
                        return Served::Done(self.end_by(worker, Halt::Violation));
                    }
                    self.give_back(worker);
                    return Served::Done(ExtractEnd::Completed);
                }
                _ => return Served::Done(self.end_by(worker, Halt::Violation)),
            }
        }
    }

    /// Riceve i byte della voce chiesta con `id` fino all'esito.
    fn serve_entry(
        &self,
        worker: Worker,
        id: RequestId,
        max_bytes: u64,
        deadline: &Deadline<'_>,
        stop: &dyn Fn() -> bool,
        sink: &mut dyn EntrySink,
    ) -> Served<EntryEnd> {
        let mut received = 0_u64;
        let mut answered = false;
        let halted = |pool: &ExtractorPool, worker: Worker, halt: Halt| {
            Served::Done(match pool.end_by(worker, halt) {
                ExtractEnd::Failed(failure) => EntryEnd::Failed(failure),
                ExtractEnd::Stopped | ExtractEnd::Completed => EntryEnd::Stopped,
            })
        };
        loop {
            let response = match Self::next_response(&worker, deadline, stop) {
                Ok(response) => response,
                Err(halt) => return halted(self, worker, halt),
            };
            if Self::is_stale(&response, id, answered) {
                self.retire(worker, Retire::Now);
                return Served::Retry;
            }
            answered = true;
            match response {
                Response::EntryData { id: got, bytes }
                    if got == id && !bytes.is_empty() && bytes.len() <= ENTRY_CHUNK_BYTES =>
                {
                    received =
                        received.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                    if received > max_bytes {
                        return halted(self, worker, Halt::Violation);
                    }
                    if sink.bytes(&bytes).is_err() {
                        return halted(self, worker, Halt::Stopped);
                    }
                }
                Response::EntryEnd { id: got, outcome } if got == id => {
                    let end = match outcome {
                        EntryOutcome::Extracted { bytes } if bytes == received => {
                            EntryEnd::Extracted { bytes }
                        }
                        EntryOutcome::Extracted { .. } => {
                            return halted(self, worker, Halt::Violation);
                        }
                        EntryOutcome::Failed(kind) => EntryEnd::NotExtracted(kind),
                    };
                    self.give_back(worker);
                    return Served::Done(end);
                }
                _ => return halted(self, worker, Halt::Violation),
            }
        }
    }
}

impl Drop for ExtractorPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Il ciclo del thread del pool: avvia processi su richiesta e porta avanti le chiusure,
/// controllandole ogni [`REAP_TICK`] finché ce ne sono. Dopo [`PoolCommand::Finish`] (o con
/// il canale chiuso) esce quando non resta nulla da chiudere e nessun processo è in uso, o
/// dopo [`SHUTDOWN_WAIT`].
fn run_pool_thread(
    launcher: &dyn WorkerLauncher,
    commands: &Receiver<PoolCommand>,
    shared: &Shared,
) {
    let mut closing: Vec<Closing> = Vec::new();
    let mut finish_by: Option<Instant> = None;
    loop {
        let now = Instant::now();
        closing.retain_mut(|item| !item.advance(now, shared));
        if let Some(finish_by) = finish_by
            && closing.is_empty()
            && (shared.lock().live == 0 || now >= finish_by)
        {
            return;
        }
        let command = if closing.is_empty() && finish_by.is_none() {
            commands.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            commands.recv_timeout(REAP_TICK)
        };
        match command {
            Ok(PoolCommand::Launch(reply)) => {
                // Chi ha chiesto non aspetta più: il processo avviato si chiude con il drop.
                let _delivered = reply.send(launcher.launch());
            }
            Ok(PoolCommand::Retire(worker, how)) => closing.push(Closing::new(worker, how, now)),
            Ok(PoolCommand::Finish) | Err(RecvTimeoutError::Disconnected) => {
                finish_by.get_or_insert(now + SHUTDOWN_WAIT);
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// Chiude un processo nel thread di chi chiama, quando il thread del pool non c'è più.
fn close_blocking(worker: Worker, how: Retire, shared: &Shared) {
    let mut closing = Closing::new(worker, how, Instant::now());
    while !closing.advance(Instant::now(), shared) {
        thread::sleep(REAP_TICK);
    }
}

/// Fase della chiusura di un processo.
#[derive(Debug, Clone, Copy)]
enum Phase {
    /// Stdin chiuso: si aspetta l'uscita fino a `kill_at`.
    Asked { kill_at: Instant },
    /// Terminato: si aspetta la raccolta fino a `give_up_at`.
    Killed { give_up_at: Instant },
    /// Uscito: si aspettano i due thread fino a `give_up_at`.
    Joining { give_up_at: Instant },
}

/// Un processo in chiusura nel thread del pool.
struct Closing {
    worker: Worker,
    phase: Phase,
}

impl Closing {
    fn new(worker: Worker, how: Retire, now: Instant) -> Closing {
        let kill_at = match how {
            Retire::Gently => now + STOP_GRACE,
            Retire::Now => now,
        };
        Closing {
            worker,
            phase: Phase::Asked { kill_at },
        }
    }

    /// Porta avanti la chiusura; vero quando è finita (posto liberato).
    fn advance(&mut self, now: Instant, shared: &Shared) -> bool {
        loop {
            match self.phase {
                Phase::Asked { kill_at } => {
                    if self.has_exited() {
                        self.phase = Phase::Joining {
                            give_up_at: now + STOP_GRACE,
                        };
                    } else if now >= kill_at {
                        // Un errore vuol dire processo già uscito: lo dice la prossima attesa.
                        let _killed = self.worker.process.kill();
                        self.phase = Phase::Killed {
                            give_up_at: now + STOP_GRACE,
                        };
                        return false;
                    } else {
                        return false;
                    }
                }
                Phase::Killed { give_up_at } => {
                    if self.has_exited() || now >= give_up_at {
                        self.phase = Phase::Joining {
                            give_up_at: now + STOP_GRACE,
                        };
                    } else {
                        return false;
                    }
                }
                Phase::Joining { give_up_at } => {
                    // Il lettore non resta fermo su una coda piena.
                    while self.worker.messages.try_recv().is_ok() {}
                    let finished = self.worker.threads.iter().all(JoinHandle::is_finished);
                    if !finished && now < give_up_at {
                        return false;
                    }
                    for thread in self.worker.threads.drain(..) {
                        // Un thread ancora fermo su un canale tenuto aperto da altri (un
                        // nipote del processo) si lascia andare; uno finito si raccoglie.
                        if thread.is_finished() {
                            let _ended = thread.join();
                        }
                    }
                    shared.release_slot();
                    return true;
                }
            }
        }
    }

    /// Vero se il processo è uscito (o non si può più aspettare).
    fn has_exited(&mut self) -> bool {
        !matches!(self.worker.process.wait_timeout(Duration::ZERO), Ok(None))
    }
}

#[cfg(test)]
mod tests {
    use lynkeus_core::{Location, TextUnit};
    use lynkeus_extract::Format;

    use super::*;

    #[test]
    fn relative_paths_become_absolute() {
        let relative = Path::new("cartella/nota.txt");
        let absolute = absolute_path(relative);
        assert!(absolute.is_absolute());
        assert!(absolute.ends_with(relative));
        let already = std::env::temp_dir().join("x.pdf");
        assert_eq!(absolute_path(&already), already);
    }

    #[test]
    fn pool_size_is_half_the_cores_within_bounds() {
        let size = default_pool_size().get();
        assert!((1..=MAX_POOL_SIZE).contains(&size), "{size}");
    }

    #[test]
    fn failures_map_to_warnings() {
        assert_eq!(WorkerFailure::Timeout.warning_kind(), WarningKind::Timeout);
        assert_eq!(
            WorkerFailure::Died(WorkerExit::MemoryExceeded).warning_kind(),
            WarningKind::LimitExceeded(LimitKind::Memory)
        );
        assert_eq!(
            WorkerFailure::Died(WorkerExit::Failed { code: Some(3) }).warning_kind(),
            WarningKind::Corrupt
        );
        assert_eq!(
            WorkerFailure::ProtocolViolation.warning_kind(),
            WarningKind::Corrupt
        );
        assert_eq!(
            WorkerFailure::Unavailable.warning_kind(),
            WarningKind::ExtractorUnavailable
        );
    }

    /// Il padre rivaluta i limiti del figlio: on e off point della dimensione di un'unità e
    /// della profondità delle voci.
    #[test]
    fn event_limit_partitions() {
        let unit = |len: usize| {
            ExtractEvent::Unit(TextUnit {
                location: Location::Line(1),
                text: "a".repeat(len),
            })
        };
        assert!(event_fits(&unit(MAX_UNIT_TEXT_BYTES), 5));
        assert!(!event_fits(&unit(MAX_UNIT_TEXT_BYTES + 1), 5));
        let start = |depth: usize| ExtractEvent::DocumentStart {
            inner: vec!["voce".to_owned(); depth],
            format: Format::Text,
        };
        assert!(event_fits(&start(0), 0));
        assert!(event_fits(&start(5), 5));
        assert!(!event_fits(&start(6), 5));
        let warning = ExtractEvent::Warning {
            inner: vec!["voce".to_owned(); 2],
            kind: lynkeus_extract::WarningKind::Corrupt,
        };
        assert!(!event_fits(&warning, 1));
        assert!(event_fits(&ExtractEvent::DocumentEnd, 0));
    }

    /// Una risposta di una richiesta precedente, prima di ogni risposta valida, vuol dire
    /// che il processo ha parlato da libero; una con un id futuro è una violazione.
    #[test]
    fn stale_responses_partitions() {
        let done = |id| Response::Done { id };
        assert!(ExtractorPool::is_stale(&done(1), 2, false));
        assert!(!ExtractorPool::is_stale(&done(1), 2, true));
        assert!(!ExtractorPool::is_stale(&done(2), 2, false));
        assert!(!ExtractorPool::is_stale(&done(9), 2, false));
    }

    /// Una scadenza enorme non trabocca e il tempo in pausa la allunga.
    #[test]
    fn deadlines_never_overflow_and_pauses_extend_them() {
        let huge = ExtractBudget::new(Duration::MAX);
        let deadline = Deadline {
            start: Instant::now(),
            budget: &huge,
        };
        assert!(deadline.left() > Duration::from_secs(60));
        let paused = || Duration::from_secs(10);
        let extended = ExtractBudget::with_pause(Duration::ZERO, &paused);
        let deadline = Deadline {
            start: Instant::now(),
            budget: &extended,
        };
        assert!(deadline.left() > Duration::from_secs(5));
        let none = ExtractBudget::new(Duration::ZERO);
        let deadline = Deadline {
            start: Instant::now(),
            budget: &none,
        };
        assert!(deadline.left().is_zero());
    }
}
