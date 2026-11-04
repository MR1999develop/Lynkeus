// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Ricerca negli eventi d'estrazione di un documento: ogni documento (il file stesso o una
//! voce d'archivio) è un [`DocPath`] con le sue righe di risultato, e OR/AND valgono per
//! documento, come per un file sul disco.
//!
//! [`DocSearch`] riceve gli eventi man mano (dal pool o dalla cache) e consegna i risultati
//! a lotti come la ricerca nei file di testo: con OR appena ci sono, con AND da quando ogni
//! termine è comparso nel documento. Gli avvisi diventano [`SearchEvent::Warning`] con il
//! documento a cui si riferiscono; i documenti saltati si contano.

use std::path::Path;
use std::sync::Arc;

use lynkeus_core::{FileAccumulator, TextUnit, find_words, snippet, word_text};
use lynkeus_extract::{EventSink, ExtractEvent, SkipReason, Stopped, line_location};

use crate::doc::DocPath;
use crate::event::{Hit, WarningKind};
use crate::progress::Counter;
use crate::textfile::{BATCH_HITS, FileSearch, HitSink};
use crate::worker::{Search, Stop};

/// Il documento aperto tra un inizio e una fine.
struct OpenDocument {
    doc: Arc<DocPath>,
    accumulator: FileAccumulator<Hit>,
}

/// Destinatario degli eventi di un file che li trasforma in risultati e avvisi.
pub(crate) struct DocSearch<'s, 'r, S: HitSink> {
    search: &'s Search<'r>,
    sink: S,
    path: &'s Path,
    root: usize,
    open: Option<OpenDocument>,
    /// Perché la ricerca si è fermata durante la consegna, se è successo.
    stopped: Option<Stop>,
}

impl<'s, 'r, S: HitSink> DocSearch<'s, 'r, S> {
    pub(crate) fn new(
        search: &'s Search<'r>,
        sink: S,
        path: &'s Path,
        root: usize,
    ) -> DocSearch<'s, 'r, S> {
        DocSearch {
            search,
            sink,
            path,
            root,
            open: None,
            stopped: None,
        }
    }

    /// Il motivo d'arresto registrato durante la consegna.
    pub(crate) fn stopped(&self) -> Option<Stop> {
        self.stopped
    }

    /// Il documento di un evento: il file più le voci `inner`.
    fn doc(&self, inner: Vec<String>) -> Arc<DocPath> {
        Arc::new(DocPath {
            path: self.path.to_owned(),
            inner,
        })
    }

    /// Il documento a cui dare l'avviso di un'estrazione interrotta: quello aperto (i suoi
    /// risultati si scartano), altrimenti il file.
    pub(crate) fn interrupted_doc(&mut self) -> Arc<DocPath> {
        match self.open.take() {
            Some(open) => open.doc,
            None => self.doc(Vec::new()),
        }
    }

    fn handle(&mut self, event: ExtractEvent) -> Result<(), Stop> {
        let progress = &self.search.context().progress;
        match event {
            ExtractEvent::DocumentStart { inner, .. } => {
                let doc = self.doc(inner);
                progress.set_current(Some(Arc::clone(&doc)));
                let query = &self.search.request().query;
                self.open = Some(OpenDocument {
                    doc,
                    accumulator: FileAccumulator::new(query.term_count(), query.combine()),
                });
                Ok(())
            }
            ExtractEvent::Unit(unit) => self.unit(&unit),
            ExtractEvent::DocumentEnd => match self.open.take() {
                Some(open) => {
                    let hits = open.accumulator.finish();
                    if hits.is_empty() {
                        Ok(())
                    } else {
                        self.sink.deliver(hits)
                    }
                }
                None => Ok(()),
            },
            ExtractEvent::Warning { inner, kind } => {
                let doc = self.doc(inner);
                self.search
                    .warn_shared(doc, WarningKind::from_extract(kind))
            }
            ExtractEvent::Skipped { reason, .. } => {
                let counter = match reason {
                    SkipReason::Binary => Counter::BinarySkipped,
                    SkipReason::TooLarge | SkipReason::SpecialEntry => Counter::FilesSkipped,
                };
                progress.add(counter, 1);
                Ok(())
            }
        }
    }

    /// Cerca in un'unità del documento aperto.
    fn unit(&mut self, unit: &TextUnit) -> Result<(), Stop> {
        self.sink.check()?;
        let Some(open) = self.open.as_mut() else {
            return Ok(());
        };
        let request = self.search.request();
        let file = FileSearch {
            query: &request.query,
            limits: &request.limits,
            root: self.root,
            doc: Arc::clone(&open.doc),
        };
        let room = self.sink.remaining();
        for found in find_words(file.query, &unit.text) {
            // Come nei file di testo: oltre il limite solo i termini nuovi (per AND).
            if open.accumulator.len() >= room && open.accumulator.seen().contains_all(found.terms) {
                continue;
            }
            let word = word_text(&unit.text, &found, file.limits.word_max_chars);
            let hit = Hit {
                doc: Arc::clone(&file.doc),
                root: file.root,
                location: line_location(&unit.location, found.line),
                word: word.text,
                word_truncated_start: word.truncated_start,
                word_truncated_end: word.truncated_end,
                snippet: snippet(&unit.text, &found, file.limits.context_chars),
                terms: found.terms,
            };
            open.accumulator.push(found.terms, hit);
        }
        let held = open.accumulator.len();
        if open.accumulator.is_satisfied() && (held >= room || held >= BATCH_HITS) {
            let hits = open.accumulator.take_ready();
            if !hits.is_empty() {
                self.sink.deliver(hits)?;
            }
        }
        Ok(())
    }
}

impl<S: HitSink> EventSink for DocSearch<'_, '_, S> {
    fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped> {
        if self.stopped.is_some() {
            return Err(Stopped);
        }
        self.handle(event).map_err(|stop| {
            self.stopped = Some(stop);
            Stopped
        })
    }
}
