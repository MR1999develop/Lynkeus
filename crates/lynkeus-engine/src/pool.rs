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
//! Il pool non sa nulla della ricerca né della cache: riceve un percorso e restituisce
//! eventi (`documents` li usa entrambi).

use std::io::{BufReader, BufWriter, Write};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use lynkeus_extract::protocol::{
    ExtractRequest, FrameReader, FrameWriter, ProtocolError, Request, RequestId, Response,
};
use lynkeus_extract::{EventSink, ExtractOptions, LimitKind, SequenceCheck};

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

    /// Manda la richiesta; l'id usato, o `None` se il canale è chiuso.
    fn send(&mut self, path: &Path, options: &ExtractOptions) -> Option<RequestId> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let request = Request::Extract(ExtractRequest {
            id,
            path: path.to_owned(),
            options: *options,
        });
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
        for _ in 0..SEND_ATTEMPTS {
            let mut worker = match self.checkout(stop) {
                Checkout::Ready(worker) => worker,
                Checkout::End(end) => return end,
            };
            if let Some(id) = worker.send(path, options) {
                return self.serve(worker, id, timeout, stop, sink);
            }
            // Il processo è morto da libero: se ne prova un altro.
            self.discard(worker);
        }
        ExtractEnd::Failed(WorkerFailure::Unavailable)
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

    /// Riceve le risposte alla richiesta `id` fino alla fine del file.
    fn serve(
        &self,
        mut worker: Worker,
        id: RequestId,
        timeout: Duration,
        stop: &dyn Fn() -> bool,
        sink: &mut dyn EventSink,
    ) -> ExtractEnd {
        let deadline = Instant::now() + timeout;
        let mut sequence = SequenceCheck::new();
        loop {
            if stop() {
                self.stop(worker);
                return ExtractEnd::Stopped;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                self.discard(worker);
                return ExtractEnd::Failed(WorkerFailure::Timeout);
            }
            let message = match worker.messages.recv_timeout(left.min(STOP_RECHECK)) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => FromWorker::Closed,
            };
            match message {
                FromWorker::Response(Response::Event { id: got, event }) if got == id => {
                    if sequence.accept(&event).is_err() {
                        self.discard(worker);
                        return ExtractEnd::Failed(WorkerFailure::ProtocolViolation);
                    }
                    if sink.emit(event).is_err() {
                        self.stop(worker);
                        return ExtractEnd::Stopped;
                    }
                }
                FromWorker::Response(Response::Done { id: got }) if got == id => {
                    if sequence.finish().is_err() {
                        self.discard(worker);
                        return ExtractEnd::Failed(WorkerFailure::ProtocolViolation);
                    }
                    self.give_back(worker);
                    return ExtractEnd::Completed;
                }
                FromWorker::Response(_) | FromWorker::Violation => {
                    self.discard(worker);
                    return ExtractEnd::Failed(WorkerFailure::ProtocolViolation);
                }
                FromWorker::Closed => {
                    let failure = worker.exit_after_close();
                    self.discard(worker);
                    return ExtractEnd::Failed(failure);
                }
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
