// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Avvio di una ricerca in background e maniglia per controllarla.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use crossbeam_channel::{Receiver, TryRecvError};

use crate::documents::DocumentSource;
use crate::event::SearchEvent;
use crate::progress::Progress;
use crate::request::SearchRequest;
use crate::worker::{self, WorkerContext};

/// Funzione chiamata dal worker dopo ogni evento inviato (es. `ctx.request_repaint()`).
///
/// Deve essere veloce e non bloccarsi: gira sul thread del worker.
pub type Notify = Arc<dyn Fn() + Send + Sync>;

/// Eventi in coda al massimo; oltre, il worker aspetta che il chiamante li legga.
const EVENT_QUEUE_CAPACITY: usize = 256;

/// Esito di [`SearchHandle::try_recv`].
#[derive(Debug)]
pub enum Received {
    /// Un evento.
    Event(SearchEvent),
    /// Nessun evento ora; la ricerca continua.
    Empty,
    /// La ricerca è finita e [`SearchEvent::Finished`] è già stato consegnato.
    Ended,
    /// Il worker è terminato senza mandare [`SearchEvent::Finished`]: un panic.
    WorkerLost,
}

/// Maniglia di una ricerca in corso.
///
/// Chiuderla (drop) annulla la ricerca senza aspettare il worker, che si ferma tra un file
/// e l'altro o al primo evento che non può più consegnare.
pub struct SearchHandle {
    events: Receiver<SearchEvent>,
    progress: Arc<Progress>,
    cancel: Arc<AtomicBool>,
    finished_delivered: bool,
}

impl SearchHandle {
    /// Avanzamento, da leggere quando si vuole (mai bloccante).
    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    /// Chiede al worker di fermarsi; la ricerca finirà con
    /// [`Outcome::Cancelled`](crate::Outcome::Cancelled). Non aspetta.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Vero se è stato chiesto l'annullamento.
    pub fn is_cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Il prossimo evento, senza aspettare.
    pub fn try_recv(&mut self) -> Received {
        match self.events.try_recv() {
            Ok(event) => {
                if matches!(event, SearchEvent::Finished(_)) {
                    self.finished_delivered = true;
                }
                Received::Event(event)
            }
            Err(TryRecvError::Empty) => Received::Empty,
            Err(TryRecvError::Disconnected) if self.finished_delivered => Received::Ended,
            Err(TryRecvError::Disconnected) => Received::WorkerLost,
        }
    }
}

impl Drop for SearchHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl std::fmt::Debug for SearchHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchHandle")
            .field("cancel_requested", &self.is_cancel_requested())
            .field("finished_delivered", &self.finished_delivered)
            .finish_non_exhaustive()
    }
}

/// Avvia la ricerca su un thread con nome e restituisce subito.
///
/// I documenti (PDF, Office, archivi) si leggono con `documents` (cache e processi
/// estrattori); con `None` si contano tra i file saltati. `notify` è chiamata dopo ogni
/// evento messo in coda. Errore solo se il sistema non riesce a creare il thread.
pub fn start_search(
    request: SearchRequest,
    documents: Option<Arc<DocumentSource>>,
    notify: Notify,
) -> io::Result<SearchHandle> {
    let (sender, events) = crossbeam_channel::bounded(EVENT_QUEUE_CAPACITY);
    let progress = Arc::new(Progress::default());
    let cancel = Arc::new(AtomicBool::new(false));
    let context = WorkerContext {
        sender,
        notify,
        progress: Arc::clone(&progress),
        cancel: Arc::clone(&cancel),
        documents,
    };
    thread::Builder::new()
        .name("lynkeus-search".to_owned())
        .spawn(move || worker::run(&request, &context))?;
    Ok(SearchHandle {
        events,
        progress,
        cancel,
        finished_delivered: false,
    })
}
