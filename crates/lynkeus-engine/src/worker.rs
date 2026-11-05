// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Il lavoro di una ricerca: scansione parallela delle radici e lettura dei file.
//!
//! Tutte le radici vanno in un'unica scansione parallela di `ignore` (filtri standard
//! spenti, link e junction non seguiti); i thread della scansione leggono anche i file, uno
//! alla volta ciascuno. `filter_entry` pota le cartelle da saltare e le radici annidate;
//! il visitatore filtra i file (contandoli) e li cerca. I filtri si preparano una volta per
//! ricerca ([`FileFilter`]).
//!
//! I documenti (formato riconosciuto da [`Format::for_path`]) non si leggono nei thread
//! della scansione: vanno in una coda limitata letta da tanti thread quanti sono i processi
//! del pool, che li mandano al [`DocumentSource`] (cache o processo estrattore) e cercano
//! negli eventi ([`DocSearch`]). Senza una fonte dei documenti si contano tra i saltati.
//!
//! I risultati non ancora sicuri (AND) trattenuti da tutti i thread insieme non superano il
//! limite dei risultati ([`HeldHits`]); un file o un documento che non trova posto si
//! rilegge alla fine con i risultati già sicuri (vedi [`crate::batch`]).
//!
//! Il tempo in cui un lettore di documenti aspetta la coda verso il chiamante (interfaccia
//! lenta o occupata) non conta nella scadenza del processo estrattore.
//!
//! Arresto: l'annullamento si controlla prima di ogni voce della scansione e a ogni passo
//! dentro un file; il primo thread che raggiunge il limite dei risultati o trova il canale
//! chiuso ferma tutti gli altri. [`SearchEvent::Finished`] parte per ultimo, quando tutti i
//! thread sono finiti.

use std::cell::Cell;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, SendTimeoutError, Sender};
use ignore::{DirEntry, WalkBuilder, WalkState};
use lynkeus_core::Query;
use lynkeus_extract::{ExtractError, ExtractOptions, Format, open_text};

use crate::batch::{HeldHits, HitBatcher, HitLabel, HitSink};
use crate::doc::DocPath;
use crate::docsearch::{DocSearch, Pass};
use crate::documents::DocumentSource;
use crate::event::{Hit, Notify, Outcome, SearchEvent, Summary, Warning, WarningKind};
use crate::filter::{EntryFacts, FileFilter};
use crate::halt::{Halt, Stop};
use crate::pool::{ExtractBudget, ExtractEnd};
use crate::progress::{Counter, Progress};
use crate::request::SearchRequest;
use crate::roots::{self, PlannedRoot, RootIndex};
use crate::textfile::{FileError, TextEnd, search_text};

/// Attesa massima su una coda piena prima di ricontrollare l'annullamento.
const SEND_RECHECK: Duration = Duration::from_millis(50);

/// Documenti in attesa di un processo estrattore, al massimo.
const DOCUMENT_QUEUE: usize = 64;

/// Ciò che il worker condivide con la [`SearchHandle`](crate::SearchHandle).
pub(crate) struct WorkerContext {
    pub(crate) sender: Sender<SearchEvent>,
    pub(crate) notify: Notify,
    pub(crate) progress: Arc<Progress>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) documents: Option<Arc<DocumentSource>>,
}

/// Un documento da leggere: radice di provenienza e file.
struct QueuedDocument {
    root: usize,
    path: PathBuf,
}

/// Esegue tutta la ricerca e manda [`SearchEvent::Finished`] per ultimo.
pub(crate) fn run(request: &SearchRequest, context: &WorkerContext) {
    let started = Instant::now();
    let search = Search {
        request,
        context,
        filter: FileFilter::new(&request.filters),
        halt: Halt::default(),
        budget: Budget::new(request.limits.max_results),
        // Oltre il limite un documento tiene solo i risultati con un termine nuovo: al più
        // uno per termine in più.
        held: HeldHits::new(request.limits.max_results.saturating_add(Query::MAX_TERMS)),
    };
    search.all_roots();
    let outcome = match search.halt.get() {
        None => Outcome::Completed,
        Some(Stop::Cancelled) => Outcome::Cancelled,
        Some(Stop::LimitReached) => Outcome::LimitReached,
        Some(Stop::Disconnected) => return,
    };
    context.progress.set_current(None);
    let summary = Summary {
        outcome,
        elapsed: started.elapsed(),
        counts: context.progress.snapshot(),
    };
    // Se il chiamante ha già chiuso la maniglia non c'è nessuno da avvisare: va bene così.
    if context.sender.send(SearchEvent::Finished(summary)).is_ok() {
        (context.notify)();
    }
}

/// Stato di una ricerca, condiviso (per riferimento) dai thread della scansione.
struct Search<'a> {
    request: &'a SearchRequest,
    context: &'a WorkerContext,
    filter: FileFilter,
    halt: Halt,
    budget: Budget,
    held: HeldHits,
}

impl Search<'_> {
    fn all_roots(&self) {
        if let Err(stop) = self.check() {
            self.halt.record(stop);
            return;
        }
        let plan = roots::plan(&self.request.roots);
        for missing in plan.missing {
            let warning = self.warn(DocPath::file(missing.path), kind_for_io(&missing.error));
            if let Err(stop) = warning {
                self.halt.record(stop);
                return;
            }
        }
        let index = Arc::new(plan.index);
        let Some(documents) = &self.context.documents else {
            self.walk(&index, None);
            return;
        };
        let (queue, pending) = crossbeam_channel::bounded(DOCUMENT_QUEUE);
        thread::scope(|scope| {
            let mut readers = 0;
            for _ in 0..documents.pool().size() {
                let pending = pending.clone();
                let spawned = thread::Builder::new()
                    .name("lynkeus-documenti".to_owned())
                    .spawn_scoped(scope, move || self.read_documents(documents, &pending));
                if spawned.is_ok() {
                    readers += 1;
                }
            }
            drop(pending);
            // Senza lettori i documenti diventerebbero avvisi uno per uno: meglio la scansione
            // dei soli file di testo, con i documenti contati tra i saltati.
            self.walk(&index, (readers > 0).then_some(&queue));
            // La coda chiusa dice ai lettori che non arriva altro.
            drop(queue);
        });
    }

    /// Scansione parallela delle radici; i documenti vanno in `documents`, se c'è.
    fn walk(&self, index: &Arc<RootIndex>, documents: Option<&Sender<QueuedDocument>>) {
        let Some((first, others)) = index.roots().split_first() else {
            return;
        };
        let mut builder = WalkBuilder::new(&first.path);
        for root in others {
            builder.add(&root.path);
        }
        let dir_filter = DirFilter {
            filter: self.filter.clone(),
            roots: Arc::clone(index),
        };
        builder
            .standard_filters(false)
            .follow_links(false)
            .filter_entry(move |entry| dir_filter.accepts(entry))
            .build_parallel()
            .run(|| {
                let index = Arc::clone(index);
                Box::new(move |result| self.visit(&index, documents, result))
            });
    }

    /// Una voce della scansione, su uno dei suoi thread.
    fn visit(
        &self,
        index: &RootIndex,
        documents: Option<&Sender<QueuedDocument>>,
        result: Result<DirEntry, ignore::Error>,
    ) -> WalkState {
        let outcome = self.check().and_then(|()| match result {
            Ok(entry) => self.entry(index, documents, &entry),
            Err(error) => self.walk_error(&error),
        });
        match outcome {
            Ok(()) => WalkState::Continue,
            Err(stop) => {
                self.halt.record(stop);
                WalkState::Quit
            }
        }
    }

    fn entry(
        &self,
        index: &RootIndex,
        documents: Option<&Sender<QueuedDocument>>,
        entry: &DirEntry,
    ) -> Result<(), Stop> {
        // Cartelle: le visita la scansione. Link e junction: non si seguono.
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            return Ok(());
        }
        let Some(root) = index.owner(entry.path(), entry.depth()) else {
            return Ok(());
        };
        let progress = &self.context.progress;
        progress.add(Counter::FilesSeen, 1);
        let metadata = entry.metadata().ok();
        let facts = EntryFacts::read(entry.file_name(), metadata.as_ref());
        if self
            .filter
            .check_file(entry.path(), facts, entry.depth() == 0)
            .is_err()
        {
            progress.add(Counter::FilesSkipped, 1);
            return Ok(());
        }
        if Format::for_path(entry.path()).is_none() {
            return self.text_file(root, entry.path());
        }
        match documents {
            Some(queue) => self.queue_document(
                queue,
                QueuedDocument {
                    root: root.index,
                    path: entry.path().to_owned(),
                },
            ),
            None => {
                progress.add(Counter::FilesSkipped, 1);
                Ok(())
            }
        }
    }

    /// Mette un documento in coda; con la coda piena aspetta, ricontrollando l'arresto.
    fn queue_document(
        &self,
        queue: &Sender<QueuedDocument>,
        document: QueuedDocument,
    ) -> Result<(), Stop> {
        let mut document = document;
        loop {
            match queue.send_timeout(document, SEND_RECHECK) {
                Ok(()) => return Ok(()),
                Err(SendTimeoutError::Timeout(back)) => {
                    self.check()?;
                    document = back;
                }
                // Nessun lettore: si sono fermati tutti, quindi la ricerca si sta fermando.
                Err(SendTimeoutError::Disconnected(_)) => return self.check(),
            }
        }
    }

    /// Il ciclo di un lettore di documenti: uno alla volta, finché la coda si chiude o la
    /// ricerca si ferma.
    fn read_documents(&self, documents: &DocumentSource, pending: &Receiver<QueuedDocument>) {
        for document in pending {
            if let Err(stop) = self
                .check()
                .and_then(|()| self.document(documents, &document))
            {
                self.halt.record(stop);
                return;
            }
        }
    }

    /// Legge e cerca un documento; se qualche documento interno va riletto (vedi
    /// [`crate::batch`]), lo rilegge.
    fn document(&self, documents: &DocumentSource, document: &QueuedDocument) -> Result<(), Stop> {
        let progress = &self.context.progress;
        progress.set_current(Some(Arc::new(DocPath::file(&document.path))));
        let mut rereads = Vec::new();
        let end = self.extract_document(documents, document, Pass::First, &mut rereads)?;
        if end == ExtractEnd::Completed {
            progress.add(Counter::FilesSearched, 1);
            if !rereads.is_empty() {
                self.extract_document(
                    documents,
                    document,
                    Pass::Reread(&rereads),
                    &mut Vec::new(),
                )?;
            }
        }
        Ok(())
    }

    /// Una lettura del documento: gli eventi vanno a una [`DocSearch`] per `pass`; un
    /// fallimento diventa un avviso. Le voci da rileggere finiscono in `rereads`.
    fn extract_document(
        &self,
        documents: &DocumentSource,
        document: &QueuedDocument,
        pass: Pass<'_>,
        rereads: &mut Vec<Vec<String>>,
    ) -> Result<ExtractEnd, Stop> {
        let options = ExtractOptions {
            limits: self.request.limits.archive,
        };
        let stop = || self.check().is_err();
        let waited = Cell::new(Duration::ZERO);
        let paused = || waited.get();
        let budget = ExtractBudget::with_pause(self.request.limits.extract_timeout, &paused);
        let sink = SharedSink {
            search: self,
            waited: Some(&waited),
        };
        let mut events = DocSearch::new(
            self.request,
            &self.context.progress,
            &self.held,
            sink,
            &document.path,
            document.root,
            pass,
        );
        let (end, _) =
            documents.extract_within(&document.path, &options, &budget, &stop, &mut events);
        *rereads = events.take_rereads();
        match end {
            ExtractEnd::Completed => Ok(end),
            ExtractEnd::Stopped => match events.stopped() {
                Some(stop) => Err(stop),
                None => self.check().map(|()| end),
            },
            ExtractEnd::Failed(failure) => events
                .warn_interrupted(failure.warning_kind())
                .map(|()| end),
        }
    }

    /// Legge e cerca un file di testo; se il file va riletto (vedi [`crate::batch`]), lo
    /// rilegge con i risultati già sicuri.
    fn text_file(&self, root: &PlannedRoot, path: &Path) -> Result<(), Stop> {
        let progress = &self.context.progress;
        let doc = Arc::new(DocPath::file(path));
        progress.set_current(Some(Arc::clone(&doc)));
        let label = HitLabel {
            query: &self.request.query,
            limits: &self.request.limits,
            root: root.index,
            doc: Arc::clone(&doc),
        };
        let query = &self.request.query;
        let mut batcher = HitBatcher::new(query, &self.held);
        loop {
            let mut reader = match open_text(path, self.filter.max_read_size()) {
                Ok(reader) => reader,
                Err(error) => return self.unreadable(doc, &error),
            };
            let sink = &mut SharedSink {
                search: self,
                waited: None,
            };
            match search_text(&mut reader, &label, batcher, sink) {
                Ok(TextEnd::Searched) => {
                    progress.add(Counter::FilesSearched, 1);
                    return Ok(());
                }
                Ok(TextEnd::Reread) => batcher = HitBatcher::satisfied(query, &self.held),
                Err(FileError::Stop(stop)) => return Err(stop),
                Err(FileError::Read(error)) => return self.unreadable(doc, &error),
            }
        }
    }

    /// Un file che non si è potuto leggere: saltato e contato, oppure un avviso.
    fn unreadable(&self, doc: Arc<DocPath>, error: &ExtractError) -> Result<(), Stop> {
        let progress = &self.context.progress;
        match error {
            ExtractError::Binary => {
                progress.add(Counter::BinarySkipped, 1);
                Ok(())
            }
            ExtractError::TooLarge { .. } => {
                progress.add(Counter::FilesSkipped, 1);
                Ok(())
            }
            other => {
                let kind = WarningKind::from_extract_error(other).unwrap_or(WarningKind::Io);
                self.warn_shared(doc, kind)
            }
        }
    }

    /// Un errore della scansione (cartella illeggibile, sparita): un avviso.
    fn walk_error(&self, error: &ignore::Error) -> Result<(), Stop> {
        let path = error_path(error).map_or_else(PathBuf::new, Path::to_path_buf);
        let kind = error.io_error().map_or(WarningKind::Io, kind_for_io);
        self.warn(DocPath::file(path), kind)
    }

    fn warn(&self, doc: DocPath, kind: WarningKind) -> Result<(), Stop> {
        self.warn_shared(Arc::new(doc), kind)
    }

    fn warn_shared(&self, doc: Arc<DocPath>, kind: WarningKind) -> Result<(), Stop> {
        self.context.progress.add(Counter::Warnings, 1);
        self.send(SearchEvent::Warning(Warning { doc, kind }))
    }

    /// Errore se la ricerca deve fermarsi.
    fn check(&self) -> Result<(), Stop> {
        if let Some(stop) = self.halt.get() {
            return Err(stop);
        }
        if self.context.cancel.load(Ordering::Relaxed) {
            return Err(Stop::Cancelled);
        }
        Ok(())
    }

    /// Mette in coda un evento e avvisa il chiamante. Con la coda piena aspetta, ma
    /// ricontrolla l'annullamento ogni [`SEND_RECHECK`].
    fn send(&self, event: SearchEvent) -> Result<(), Stop> {
        let mut event = event;
        loop {
            match self.context.sender.send_timeout(event, SEND_RECHECK) {
                Ok(()) => {
                    (self.context.notify)();
                    return Ok(());
                }
                Err(SendTimeoutError::Timeout(back)) => {
                    self.check()?;
                    event = back;
                }
                Err(SendTimeoutError::Disconnected(_)) => return Err(Stop::Disconnected),
            }
        }
    }
}

/// Risultati che si possono ancora consegnare, condivisi tra i thread.
#[derive(Debug)]
struct Budget(AtomicUsize);

impl Budget {
    fn new(max_results: usize) -> Budget {
        Budget(AtomicUsize::new(max_results))
    }

    fn remaining(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }

    /// Prenota fino a `wanted` risultati: quanti se ne possono consegnare e se il limite
    /// è ora pieno (anche se è stato riempito proprio da questa prenotazione).
    fn reserve(&self, wanted: usize) -> (usize, bool) {
        let before = self
            .0
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                Some(left - left.min(wanted))
            })
            .unwrap_or_else(|left| left);
        (before.min(wanted), before <= wanted)
    }
}

/// I risultati e gli avvisi di un file verso il canale, nel rispetto del limite.
struct SharedSink<'a, 'b> {
    search: &'a Search<'b>,
    /// Se c'è, vi si somma il tempo passato ad aspettare la coda verso il chiamante.
    waited: Option<&'a Cell<Duration>>,
}

impl SharedSink<'_, '_> {
    /// Manda `event` contando l'attesa in `waited`.
    fn send_timed(&self, send: impl FnOnce() -> Result<(), Stop>) -> Result<(), Stop> {
        let started = Instant::now();
        let sent = send();
        if let Some(waited) = self.waited {
            waited.set(waited.get().saturating_add(started.elapsed()));
        }
        sent
    }
}

impl HitSink for SharedSink<'_, '_> {
    fn check(&self) -> Result<(), Stop> {
        self.search.check()
    }

    fn remaining(&self) -> usize {
        self.search.budget.remaining()
    }

    fn deliver(&mut self, mut hits: Vec<Hit>) -> Result<(), Stop> {
        let (granted, full) = self.search.budget.reserve(hits.len());
        hits.truncate(granted);
        if !hits.is_empty() {
            let count = u64::try_from(hits.len()).unwrap_or(u64::MAX);
            self.search.context.progress.add(Counter::Hits, count);
            self.send_timed(|| self.search.send(SearchEvent::Hits(hits)))?;
        }
        if full {
            Err(Stop::LimitReached)
        } else {
            Ok(())
        }
    }

    fn warn(&mut self, doc: Arc<DocPath>, kind: WarningKind) -> Result<(), Stop> {
        self.send_timed(|| self.search.warn_shared(doc, kind))
    }
}

/// Il filtro delle voci della scansione: pota cartelle e radici annidate.
struct DirFilter {
    filter: FileFilter,
    roots: Arc<RootIndex>,
}

impl DirFilter {
    /// Vero se la voce va visitata. Le radici passano sempre; una voce che è a sua volta una
    /// radice si lascia alla sua scansione; i file si filtrano (e si contano) nel visitatore.
    fn accepts(&self, entry: &DirEntry) -> bool {
        let depth = entry.depth();
        if depth == 0 {
            return true;
        }
        if let Some(owner) = self.roots.owner(entry.path(), depth)
            && self.roots.is_other_root(entry.path(), owner)
        {
            return false;
        }
        if !entry.file_type().is_some_and(|t| t.is_dir()) {
            return true;
        }
        let facts = EntryFacts::read(entry.file_name(), entry.metadata().ok().as_ref());
        self.filter.enter_dir(entry.file_name(), facts)
    }
}

fn kind_for_io(error: &io::Error) -> WarningKind {
    match error.kind() {
        io::ErrorKind::NotFound => WarningKind::NotFound,
        io::ErrorKind::PermissionDenied => WarningKind::AccessDenied,
        _ => WarningKind::Io,
    }
}

/// Il percorso a cui si riferisce un errore della scansione, se c'è.
fn error_path(error: &ignore::Error) -> Option<&Path> {
    match error {
        ignore::Error::WithPath { path, .. } => Some(path),
        ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => {
            error_path(err)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_reservations() {
        let budget = Budget::new(5);
        assert_eq!(budget.reserve(2), (2, false));
        assert_eq!(budget.reserve(3), (3, true));
        assert_eq!(budget.reserve(1), (0, true));
        assert_eq!(budget.remaining(), 0);
        let budget = Budget::new(5);
        assert_eq!(budget.reserve(9), (5, true));
        let empty = Budget::new(0);
        assert_eq!(empty.reserve(0), (0, true));
    }
}
