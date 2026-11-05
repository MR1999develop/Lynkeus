// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Pool dei processi estrattori: pochi processi longevi (ADR 0001), avviati al primo
//! documento, ciascuno con un file alla volta.
//!
//! Per ogni file [`ExtractorPool::extract`] prende un processo libero (o ne avvia uno, fino a
//! [`ExtractorPool::size`]), gli manda una [`Request::Extract`] e passa gli eventi ricevuti a
//! un [`EventSink`] man mano che arrivano. Un thread per processo legge lo stdout e consegna
//! i messaggi già decodificati; chi chiama aspetta con scadenza, quindi può sempre terminare
//! il processo.
//!
//! Regole (il padre non si fida del figlio):
//!
//! - **tempo**: oltre il timeout il processo è terminato subito e sostituito al prossimo
//!   file ([`WorkerFailure::Timeout`]);
//! - **annulla** (o il destinatario che chiede di fermarsi): stdin chiuso, che per il figlio
//!   è la richiesta di uscire; dopo [`STOP_GRACE`] il processo è terminato;
//! - **protocollo**: un frame non valido (anche byte estranei su stdout), un `id` diverso da
//!   quello della richiesta, un [`Response::Rejected`] o una sequenza di eventi sbagliata
//!   ([`SequenceCheck`]) terminano il processo ([`WorkerFailure::ProtocolViolation`]);
//! - **morte** a metà file: [`WorkerFailure::Died`] con il motivo d'uscita, "memoria
//!   superata" compresa dove il sistema la riconosce;
//! - un processo libero con messaggi in sospeso, o già morto, non si riusa.
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

use std::io::{BufReader, BufWriter, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use lynkeus_extract::protocol::{
    EntryRequest, ExtractRequest, FrameReader, FrameWriter, ProtocolError, Request, RequestId,
    Response,
};
use lynkeus_extract::{
    ENTRY_CHUNK_BYTES, EntryOutcome, EntrySink, EventSink, ExtractOptions, LimitKind, SequenceCheck,
};

use crate::event::WarningKind;
use crate::launcher::{LaunchedWorker, WorkerExit, WorkerLauncher, WorkerProcess};

/// Processi al massimo per pool.
pub const MAX_POOL_SIZE: usize = 4;

/// Dopo la richiesta di fermarsi (stdin chiuso), quanto si aspetta prima di terminare.
pub const STOP_GRACE: Duration = Duration::from_millis(500);

/// Ogni quanto chi aspetta un messaggio ricontrolla l'annullamento.
const STOP_RECHECK: Duration = Duration::from_millis(25);

/// Messaggi letti e non ancora consumati per processo: con unità fino a 1 MiB, al più circa
/// 16 MiB in memoria.
const MESSAGE_QUEUE: usize = 16;

/// Tentativi di avvio per un file quando un processo muore prima di ricevere la richiesta.
const SEND_ATTEMPTS: usize = 2;

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

/// Un messaggio dal thread che legge lo stdout di un processo.
#[derive(Debug)]
enum FromWorker {
    Response(Response),
    /// Frame non valido: il thread smette di leggere.
    Violation,
    /// Stdout chiuso (anche a metà frame): il processo è uscito o sta uscendo.
    Closed,
}

/// Un processo del pool con i suoi canali.
struct Worker {
    /// `None` dopo la richiesta di fermarsi.
    stdin: Option<FrameWriter<BufWriter<Box<dyn Write + Send>>>>,
    messages: Receiver<FromWorker>,
    process: Box<dyn WorkerProcess>,
    next_id: RequestId,
}

impl Worker {
    /// Avvia il thread di lettura per un processo appena avviato.
    fn start(launched: LaunchedWorker) -> Option<Worker> {
        let LaunchedWorker {
            stdin,
            stdout,
            mut process,
        } = launched;
        let (tx, rx) = crossbeam_channel::bounded(MESSAGE_QUEUE);
        let spawned = thread::Builder::new()
            .name("lynkeus-estrattore".to_owned())
            .spawn(move || read_messages(stdout, &tx));
        if spawned.is_err() {
            let _killed = process.kill();
            return None;
        }
        Some(Worker {
            stdin: Some(FrameWriter::new(BufWriter::new(stdin))),
            messages: rx,
            process,
            next_id: 1,
        })
    }

    /// Vero se il processo, libero, ha mandato qualcosa (o è uscito): non va riusato.
    fn has_pending(&self) -> bool {
        !self.messages.is_empty()
    }

    /// Manda la richiesta costruita da `request` con un id nuovo; l'id usato, o `None` se
    /// il canale è chiuso.
    fn send(&mut self, request: impl FnOnce(RequestId) -> Request) -> Option<RequestId> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let request = request(id);
        let writer = self.stdin.as_mut()?;
        writer
            .write(&request)
            .and_then(|()| writer.flush())
            .ok()
            .map(|()| id)
    }

    /// Chiude stdin (richiesta di fermarsi), aspetta l'uscita fino a `deadline`, poi termina.
    fn retire_by(mut self, deadline: Instant) {
        self.stdin = None;
        let left = deadline.saturating_duration_since(Instant::now());
        if !matches!(self.process.wait_timeout(left), Ok(Some(_))) {
            self.kill();
        }
    }

    /// Termina subito e raccoglie il processo.
    fn kill(&mut self) {
        // Un errore qui vuol dire processo già uscito; il `Drop` del processo è la rete di
        // sicurezza.
        let _killed = self.process.kill();
        let _reaped = self.process.wait_timeout(STOP_GRACE);
    }

    /// Il motivo d'uscita di un processo che ha chiuso lo stdout a metà file.
    fn exit_after_close(&mut self) -> WorkerFailure {
        match self.process.wait_timeout(STOP_GRACE) {
            Ok(Some(WorkerExit::Success)) => WorkerFailure::ProtocolViolation,
            Ok(Some(exit)) => WorkerFailure::Died(exit),
            // Stdout chiuso ma processo vivo: non rispetta il protocollo.
            Ok(None) | Err(_) => {
                self.kill();
                WorkerFailure::ProtocolViolation
            }
        }
    }
}

/// Legge i messaggi dello stdout di un processo finché il canale serve.
fn read_messages(stdout: Box<dyn std::io::Read + Send>, tx: &Sender<FromWorker>) {
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

/// Il percorso assoluto di `path`, senza accedere al disco: il processo estrattore ha
/// un'altra cartella corrente e risolverebbe un percorso relativo altrove. Un percorso che
/// non si può rendere assoluto (vuoto) resta com'è: l'estrattore risponderà "non trovato".
pub(crate) fn absolute_path(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}

/// Stato condiviso del pool.
struct PoolState {
    idle: Vec<Worker>,
    /// Processi vivi, liberi o in uso.
    live: usize,
    closed: bool,
}

/// Pool di processi estrattori, condivisibile tra thread.
pub struct ExtractorPool {
    launcher: Arc<dyn WorkerLauncher>,
    size: usize,
    state: Mutex<PoolState>,
    freed: Condvar,
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
    /// del primo file.
    pub fn new(launcher: Arc<dyn WorkerLauncher>, size: NonZeroUsize) -> ExtractorPool {
        ExtractorPool {
            launcher,
            size: size.get(),
            state: Mutex::new(PoolState {
                idle: Vec::new(),
                live: 0,
                closed: false,
            }),
            freed: Condvar::new(),
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
    /// Blocca fino alla fine del file, al più `timeout` più [`STOP_GRACE`]. `stop` si
    /// controlla spesso: quando diventa vero il processo riceve la richiesta di fermarsi e
    /// la funzione restituisce [`ExtractEnd::Stopped`]. Con [`ExtractEnd::Failed`] gli
    /// eventi già mandati restano validi (documenti completi di un archivio), l'ultimo
    /// documento aperto no.
    pub fn extract(
        &self,
        path: &Path,
        options: &ExtractOptions,
        timeout: Duration,
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
        match self.send_to_worker(request, stop) {
            Ok((worker, id)) => self.serve(worker, id, timeout, stop, sink),
            Err(end) => end,
        }
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
        match self.send_to_worker(request, stop) {
            Ok((worker, id)) => self.serve_entry(worker, id, max_bytes, timeout, stop, sink),
            Err(ExtractEnd::Stopped) => EntryEnd::Stopped,
            Err(ExtractEnd::Failed(failure)) => EntryEnd::Failed(failure),
            Err(ExtractEnd::Completed) => EntryEnd::Failed(WorkerFailure::Unavailable),
        }
    }

    /// Prende un processo e gli manda la richiesta; se il processo è morto da libero ne
    /// prova un altro (al più [`SEND_ATTEMPTS`] volte).
    fn send_to_worker(
        &self,
        request: impl Fn(RequestId) -> Request,
        stop: &dyn Fn() -> bool,
    ) -> Result<(Worker, RequestId), ExtractEnd> {
        for _ in 0..SEND_ATTEMPTS {
            let mut worker = match self.checkout(stop) {
                Checkout::Ready(worker) => worker,
                Checkout::End(end) => return Err(end),
            };
            if let Some(id) = worker.send(&request) {
                return Ok((worker, id));
            }
            self.discard(worker);
        }
        Err(ExtractEnd::Failed(WorkerFailure::Unavailable))
    }

    /// Chiude il pool: i processi liberi ricevono la richiesta di fermarsi e sono terminati
    /// dopo [`STOP_GRACE`]; quelli in uso si chiudono alla fine del loro file. Le
    /// estrazioni successive finiscono con [`WorkerFailure::Unavailable`].
    pub fn shutdown(&self) {
        let idle = {
            let mut state = self.lock();
            state.closed = true;
            state.live = state.live.saturating_sub(state.idle.len());
            std::mem::take(&mut state.idle)
        };
        self.freed.notify_all();
        retire_all(idle);
    }

    fn lock(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Un processo libero e pulito, avviandone uno se c'è posto; aspetta se sono tutti in
    /// uso.
    fn checkout(&self, stop: &dyn Fn() -> bool) -> Checkout {
        let mut state = self.lock();
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
                state.live = state.live.saturating_sub(1);
                drop(state);
                retire_all(vec![worker]);
                state = self.lock();
                continue;
            }
            if state.live < self.size {
                state.live += 1;
                drop(state);
                return match self.launch() {
                    Some(worker) => Checkout::Ready(worker),
                    None => {
                        self.release_slot();
                        Checkout::End(ExtractEnd::Failed(WorkerFailure::Unavailable))
                    }
                };
            }
            state = match self.freed.wait_timeout(state, STOP_RECHECK) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    fn launch(&self) -> Option<Worker> {
        let launched = self.launcher.launch().ok()?;
        self.launches.fetch_add(1, Ordering::Relaxed);
        Worker::start(launched)
    }

    /// Rimette un processo tra i liberi (o lo chiude se il pool è chiuso).
    fn give_back(&self, worker: Worker) {
        let mut state = self.lock();
        if state.closed {
            state.live = state.live.saturating_sub(1);
            drop(state);
            retire_all(vec![worker]);
        } else {
            state.idle.push(worker);
            drop(state);
        }
        self.freed.notify_one();
    }

    /// Toglie dal pool un processo già terminato o da terminare subito.
    fn discard(&self, mut worker: Worker) {
        worker.kill();
        drop(worker);
        self.release_slot();
    }

    /// Toglie dal pool un processo in uso dopo la richiesta di fermarsi.
    fn stop(&self, worker: Worker) {
        worker.retire_by(Instant::now() + STOP_GRACE);
        self.release_slot();
    }

    /// Libera il posto di un processo uscito dal pool e sveglia chi aspetta.
    fn release_slot(&self) {
        {
            let mut state = self.lock();
            state.live = state.live.saturating_sub(1);
        }
        self.freed.notify_one();
    }

    /// La prossima risposta del processo, o perché non arriva (fermati, scadenza, frame
    /// non valido, processo uscito).
    fn next_response(
        worker: &Worker,
        deadline: Instant,
        stop: &dyn Fn() -> bool,
    ) -> Result<Response, Halt> {
        loop {
            if stop() {
                return Err(Halt::Stopped);
            }
            let left = deadline.saturating_duration_since(Instant::now());
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
                self.stop(worker);
                ExtractEnd::Stopped
            }
            Halt::Timeout => {
                self.discard(worker);
                ExtractEnd::Failed(WorkerFailure::Timeout)
            }
            Halt::Violation => {
                self.discard(worker);
                ExtractEnd::Failed(WorkerFailure::ProtocolViolation)
            }
            Halt::Closed => {
                let failure = worker.exit_after_close();
                self.discard(worker);
                ExtractEnd::Failed(failure)
            }
        }
    }

    /// Riceve le risposte alla richiesta `id` fino alla fine del file.
    fn serve(
        &self,
        worker: Worker,
        id: RequestId,
        timeout: Duration,
        stop: &dyn Fn() -> bool,
        sink: &mut dyn EventSink,
    ) -> ExtractEnd {
        let deadline = Instant::now() + timeout;
        let mut sequence = SequenceCheck::new();
        loop {
            let response = match Self::next_response(&worker, deadline, stop) {
                Ok(response) => response,
                Err(halt) => return self.end_by(worker, halt),
            };
            match response {
                Response::Event { id: got, event } if got == id => {
                    if sequence.accept(&event).is_err() {
                        return self.end_by(worker, Halt::Violation);
                    }
                    if sink.emit(event).is_err() {
                        return self.end_by(worker, Halt::Stopped);
                    }
                }
                Response::Done { id: got } if got == id => {
                    if sequence.finish().is_err() {
                        return self.end_by(worker, Halt::Violation);
                    }
                    self.give_back(worker);
                    return ExtractEnd::Completed;
                }
                _ => return self.end_by(worker, Halt::Violation),
            }
        }
    }

    /// Riceve i byte della voce chiesta con `id` fino all'esito.
    fn serve_entry(
        &self,
        worker: Worker,
        id: RequestId,
        max_bytes: u64,
        timeout: Duration,
        stop: &dyn Fn() -> bool,
        sink: &mut dyn EntrySink,
    ) -> EntryEnd {
        let deadline = Instant::now() + timeout;
        let mut received = 0_u64;
        let halted =
            |pool: &ExtractorPool, worker: Worker, halt: Halt| match pool.end_by(worker, halt) {
                ExtractEnd::Failed(failure) => EntryEnd::Failed(failure),
                ExtractEnd::Stopped | ExtractEnd::Completed => EntryEnd::Stopped,
            };
        loop {
            let response = match Self::next_response(&worker, deadline, stop) {
                Ok(response) => response,
                Err(halt) => return halted(self, worker, halt),
            };
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
                    return end;
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

/// Chiude insieme più processi: stdin chiusi tutti subito, poi un'unica attesa di
/// [`STOP_GRACE`] per tutti e la terminazione di chi resta.
fn retire_all(workers: Vec<Worker>) {
    let mut workers = workers;
    for worker in &mut workers {
        worker.stdin = None;
    }
    let deadline = Instant::now() + STOP_GRACE;
    for worker in workers {
        worker.retire_by(deadline);
    }
}

#[cfg(test)]
mod tests {
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
}
