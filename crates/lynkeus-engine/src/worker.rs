// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Il lavoro di una ricerca, sul thread del worker.
//!
//! Provvisorio (fase 0): radici in sequenza, un file alla volta, annulla controllato tra
//! un file e l'altro. La fase 2 lo rende parallelo.

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::time::Instant;

use ignore::{DirEntry, WalkBuilder};
use lynkeus_core::{ByteSize, FileAccumulator, Location, find_words, snippet, word_text};
use lynkeus_extract::{ExtractError, read_text};

use crate::doc::DocPath;
use crate::event::{Hit, Outcome, SearchEvent, Summary, Warning, WarningKind};
use crate::handle::Notify;
use crate::platform;
use crate::progress::{Counter, Progress};
use crate::request::{Filters, HiddenPolicy, OnlineFilesPolicy, SearchRequest};

/// Ciò che il worker condivide con la [`SearchHandle`](crate::SearchHandle).
pub(crate) struct WorkerContext {
    pub(crate) sender: SyncSender<SearchEvent>,
    pub(crate) notify: Notify,
    pub(crate) progress: Arc<Progress>,
    pub(crate) cancel: Arc<AtomicBool>,
}

/// Perché la ricerca si ferma prima della fine.
enum Stop {
    Cancelled,
    LimitReached,
    /// Il chiamante ha chiuso la maniglia: non c'è più nessuno a cui consegnare eventi.
    Disconnected,
}

impl WorkerContext {
    /// Mette in coda un evento (aspettando se la coda è piena) e avvisa il chiamante.
    fn send(&self, event: SearchEvent) -> Result<(), Stop> {
        self.sender.send(event).map_err(|_| Stop::Disconnected)?;
        (self.notify)();
        Ok(())
    }

    fn check_cancel(&self) -> Result<(), Stop> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(Stop::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Esegue tutta la ricerca e manda [`SearchEvent::Finished`] per ultimo.
pub(crate) fn run(request: &SearchRequest, context: &WorkerContext) {
    let started = Instant::now();
    let mut search = Search {
        request,
        context,
        hits_sent: 0,
    };
    let outcome = match search.all_roots() {
        Ok(()) => Outcome::Completed,
        Err(Stop::Cancelled) => Outcome::Cancelled,
        Err(Stop::LimitReached) => Outcome::LimitReached,
        Err(Stop::Disconnected) => return,
    };
    context.progress.set_current(None);
    let summary = Summary {
        outcome,
        elapsed: started.elapsed(),
        counts: context.progress.snapshot(),
    };
    // Se il chiamante ha già chiuso la maniglia non c'è nessuno da avvisare: va bene così.
    if context.send(SearchEvent::Finished(summary)).is_err() {}
}

struct Search<'a> {
    request: &'a SearchRequest,
    context: &'a WorkerContext,
    hits_sent: usize,
}

impl Search<'_> {
    fn all_roots(&mut self) -> Result<(), Stop> {
        for (root_index, root) in self.request.roots.iter().enumerate() {
            self.context.check_cancel()?;
            self.one_root(root_index, root)?;
        }
        Ok(())
    }

    fn one_root(&mut self, root_index: usize, root: &Path) -> Result<(), Stop> {
        if let Err(error) = std::fs::symlink_metadata(root) {
            return self.warn(DocPath::file(root), warning_kind_for_io(&error));
        }
        let skip_dirs = self.request.filters.skip_dirs.clone();
        let hidden = self.request.filters.hidden;
        let walker = WalkBuilder::new(root)
            .standard_filters(false)
            .follow_links(false)
            .filter_entry(move |entry| {
                entry.depth() == 0 || !is_excluded_dir(entry, &skip_dirs, hidden)
            })
            .build();
        for result in walker {
            self.context.check_cancel()?;
            match result {
                Ok(entry) if entry.file_type().is_some_and(|t| t.is_file()) => {
                    self.one_file(root_index, &entry)?;
                }
                Ok(_) => {}
                Err(error) => {
                    let path = error_path(&error).unwrap_or(root);
                    let kind = error
                        .io_error()
                        .map_or(WarningKind::Io, warning_kind_for_io);
                    self.warn(DocPath::file(path), kind)?;
                }
            }
        }
        Ok(())
    }

    fn one_file(&mut self, root_index: usize, entry: &DirEntry) -> Result<(), Stop> {
        let progress = &self.context.progress;
        progress.add(Counter::FilesSeen, 1);
        let metadata = entry.metadata().ok();
        let is_root = entry.depth() == 0;
        if !is_root && !accepts_file(&self.request.filters, entry.path(), metadata.as_ref()) {
            progress.add(Counter::FilesSkipped, 1);
            return Ok(());
        }
        let max_size = self
            .request
            .filters
            .max_file_size
            .unwrap_or(ByteSize::from_bytes(u64::MAX));
        if metadata
            .as_ref()
            .is_some_and(|m| m.len() > max_size.bytes())
        {
            progress.add(Counter::FilesSkipped, 1);
            return Ok(());
        }

        let doc = Arc::new(DocPath::file(entry.path()));
        progress.set_current(Some(Arc::clone(&doc)));
        match read_text(entry.path(), max_size) {
            Ok(decoded) => {
                progress.add(Counter::FilesSearched, 1);
                let hits = self.collect_hits(root_index, &doc, &decoded.text);
                self.send_hits(hits)
            }
            Err(ExtractError::Binary) => {
                progress.add(Counter::BinarySkipped, 1);
                Ok(())
            }
            Err(ExtractError::TooLarge { .. }) => {
                progress.add(Counter::FilesSkipped, 1);
                Ok(())
            }
            Err(error) => {
                let kind = WarningKind::from_extract_error(&error).unwrap_or(WarningKind::Io);
                self.warn_shared(doc, kind)
            }
        }
    }

    /// Righe di un file di testo, già filtrate da OR/AND.
    fn collect_hits(&self, root_index: usize, doc: &Arc<DocPath>, text: &str) -> Vec<Hit> {
        let query = &self.request.query;
        let limits = &self.request.limits;
        let mut accumulator = FileAccumulator::new(query.term_count(), query.combine());
        for found in find_words(query, text) {
            let word = word_text(text, &found, limits.word_max_chars);
            let hit = Hit {
                doc: Arc::clone(doc),
                root: root_index,
                location: Location::Line(found.line.saturating_add(1)),
                word: word.text,
                word_truncated: word.truncated,
                snippet: snippet(text, &found, limits.context_chars),
                terms: found.terms,
            };
            accumulator.push(found.terms, hit);
        }
        accumulator.finish()
    }

    /// Manda i risultati rispettando il limite; lo raggiunge → [`Stop::LimitReached`].
    fn send_hits(&mut self, mut hits: Vec<Hit>) -> Result<(), Stop> {
        let remaining = self
            .request
            .limits
            .max_results
            .saturating_sub(self.hits_sent);
        let reached = hits.len() >= remaining;
        hits.truncate(remaining);
        if !hits.is_empty() {
            self.hits_sent += hits.len();
            let count = u64::try_from(hits.len()).unwrap_or(u64::MAX);
            self.context.progress.add(Counter::Hits, count);
            self.context.send(SearchEvent::Hits(hits))?;
        }
        if reached {
            Err(Stop::LimitReached)
        } else {
            Ok(())
        }
    }

    fn warn(&self, doc: DocPath, kind: WarningKind) -> Result<(), Stop> {
        self.warn_shared(Arc::new(doc), kind)
    }

    fn warn_shared(&self, doc: Arc<DocPath>, kind: WarningKind) -> Result<(), Stop> {
        self.context.progress.add(Counter::Warnings, 1);
        self.context
            .send(SearchEvent::Warning(Warning { doc, kind }))
    }
}

/// Vero se `entry` è una cartella da non visitare (nome in `skip_dirs` o nascosta).
fn is_excluded_dir(entry: &DirEntry, skip_dirs: &[String], hidden: HiddenPolicy) -> bool {
    if !entry.file_type().is_some_and(|t| t.is_dir()) {
        return false;
    }
    let name = entry.file_name();
    if skip_dirs
        .iter()
        .any(|skip| platform::dir_name_matches(name, skip))
    {
        return true;
    }
    hidden == HiddenPolicy::Skip && platform::is_hidden(name, entry.metadata().ok().as_ref())
}

/// Filtri sui file sotto una radice: nascosti, estensione, solo online.
fn accepts_file(filters: &Filters, path: &Path, metadata: Option<&std::fs::Metadata>) -> bool {
    let name = path.file_name().unwrap_or_default();
    if filters.hidden == HiddenPolicy::Skip && platform::is_hidden(name, metadata) {
        return false;
    }
    if !filters.extensions.accepts(path) {
        return false;
    }
    !(filters.online_only == OnlineFilesPolicy::Skip
        && metadata.is_some_and(platform::is_online_only))
}

fn warning_kind_for_io(error: &io::Error) -> WarningKind {
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
