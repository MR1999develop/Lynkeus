// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Ricerca negli eventi d'estrazione di un documento: ogni documento (il file stesso o una
//! voce d'archivio) è un [`DocPath`] con le sue righe di risultato, e OR/AND valgono per
//! documento, come per un file sul disco.
//!
//! [`DocSearch`] riceve gli eventi man mano (dal pool o dalla cache) e passa i risultati a un
//! [`HitBatcher`] per documento, come la ricerca nei file di testo. Gli avvisi diventano
//! avvisi della ricerca con il documento a cui si riferiscono; i documenti saltati si
//! contano. I documenti che il batcher vuole rileggere ([`Verdict::Reread`]) si raccolgono:
//! chi chiama rilegge il file con [`Pass::Reread`], che guarda solo quei documenti.

use std::path::Path;
use std::sync::Arc;

use lynkeus_core::{TextUnit, find_words};
use lynkeus_extract::{EventSink, ExtractEvent, SkipReason, Stopped, line_location};

use crate::batch::{HeldHits, HitBatcher, HitLabel, HitSink, Verdict};
use crate::doc::DocPath;
use crate::event::WarningKind;
use crate::halt::Stop;
use crate::progress::{Counter, Progress};
use crate::request::SearchRequest;

/// Quale lettura del file.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Pass<'p> {
    /// La prima: ogni documento, con avvisi e saltati.
    First,
    /// La seconda: solo i documenti con queste voci, che hanno già tutti i termini; avvisi e
    /// saltati sono già stati contati.
    Reread(&'p [Vec<String>]),
}

/// Il documento aperto tra un inizio e una fine.
struct OpenDocument<'h> {
    doc: Arc<DocPath>,
    batcher: HitBatcher<'h>,
}

/// Destinatario degli eventi di un file che li trasforma in risultati e avvisi.
pub(crate) struct DocSearch<'a, S: HitSink> {
    request: &'a SearchRequest,
    progress: &'a Progress,
    held: &'a HeldHits,
    sink: S,
    path: &'a Path,
    root: usize,
    pass: Pass<'a>,
    open: Option<OpenDocument<'a>>,
    /// Voci dei documenti da rileggere.
    rereads: Vec<Vec<String>>,
    /// Perché la ricerca si è fermata durante la consegna, se è successo.
    stopped: Option<Stop>,
}

impl<'a, S: HitSink> DocSearch<'a, S> {
    /// La ricerca negli eventi del file `path` (dalla radice `root`) per `request`;
    /// risultati e avvisi vanno a `sink`.
    pub(crate) fn new(
        request: &'a SearchRequest,
        progress: &'a Progress,
        held: &'a HeldHits,
        sink: S,
        path: &'a Path,
        root: usize,
        pass: Pass<'a>,
    ) -> DocSearch<'a, S> {
        DocSearch {
            request,
            progress,
            held,
            sink,
            path,
            root,
            pass,
            open: None,
            rereads: Vec::new(),
            stopped: None,
        }
    }

    /// Il motivo d'arresto registrato durante la consegna.
    pub(crate) fn stopped(&self) -> Option<Stop> {
        self.stopped
    }

    /// Le voci dei documenti da rileggere (vuoto se nessuno).
    pub(crate) fn take_rereads(&mut self) -> Vec<Vec<String>> {
        std::mem::take(&mut self.rereads)
    }

    /// Avvisa che l'estrazione si è interrotta: l'avviso va al documento aperto (i suoi
    /// risultati si scartano), altrimenti al file.
    pub(crate) fn warn_interrupted(&mut self, kind: WarningKind) -> Result<(), Stop> {
        let doc = match self.open.take() {
            Some(open) => open.doc,
            None => self.doc(Vec::new()),
        };
        self.sink.warn(doc, kind)
    }

    /// Il documento di un evento: il file più le voci `inner`.
    fn doc(&self, inner: Vec<String>) -> Arc<DocPath> {
        Arc::new(DocPath {
            path: self.path.to_owned(),
            inner,
        })
    }

    fn handle(&mut self, event: ExtractEvent) -> Result<(), Stop> {
        let first = matches!(self.pass, Pass::First);
        match event {
            ExtractEvent::DocumentStart { inner, .. } => {
                self.open_document(inner);
                Ok(())
            }
            ExtractEvent::Unit(unit) => self.unit(&unit),
            ExtractEvent::DocumentEnd => self.close_document(),
            ExtractEvent::Warning { inner, kind } if first => {
                let doc = self.doc(inner);
                self.sink.warn(doc, WarningKind::from_extract(kind))
            }
            ExtractEvent::Skipped { reason, .. } if first => {
                let counter = match reason {
                    SkipReason::Binary => Counter::BinarySkipped,
                    SkipReason::TooLarge | SkipReason::SpecialEntry => Counter::FilesSkipped,
                };
                self.progress.add(counter, 1);
                Ok(())
            }
            ExtractEvent::Warning { .. } | ExtractEvent::Skipped { .. } => Ok(()),
        }
    }

    /// Apre il documento `inner`, se questa lettura lo riguarda.
    fn open_document(&mut self, inner: Vec<String>) {
        let query = &self.request.query;
        let batcher = match self.pass {
            Pass::First => HitBatcher::new(query, self.held),
            Pass::Reread(wanted) if wanted.contains(&inner) => {
                HitBatcher::satisfied(query, self.held)
            }
            Pass::Reread(_) => {
                self.open = None;
                return;
            }
        };
        let doc = self.doc(inner);
        if matches!(self.pass, Pass::First) {
            self.progress.set_current(Some(Arc::clone(&doc)));
        }
        self.open = Some(OpenDocument { doc, batcher });
    }

    /// Chiude il documento aperto: consegna ciò che resta, o lo segna da rileggere.
    fn close_document(&mut self) -> Result<(), Stop> {
        let Some(open) = self.open.take() else {
            return Ok(());
        };
        match open.batcher.finish() {
            Verdict::Hits(hits) if hits.is_empty() => Ok(()),
            Verdict::Hits(hits) => self.sink.deliver(hits),
            Verdict::Reread => {
                self.rereads.push(open.doc.inner.clone());
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
        let label = HitLabel {
            query: &self.request.query,
            limits: &self.request.limits,
            root: self.root,
            doc: Arc::clone(&open.doc),
        };
        let room = self.sink.remaining();
        for found in find_words(label.query, &unit.text) {
            let make = || {
                label.hit(
                    &unit.text,
                    &found,
                    line_location(&unit.location, found.line),
                )
            };
            if let Some(batch) = open.batcher.push(found.terms, room, make) {
                self.sink.deliver(batch)?;
            }
        }
        Ok(())
    }
}

impl<S: HitSink> EventSink for DocSearch<'_, S> {
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

#[cfg(test)]
mod tests {
    use lynkeus_core::{Combine, Location, MatchOptions, Query, Term, WildcardMode};
    use lynkeus_extract::Format;

    use super::*;
    use crate::event::Hit;
    use crate::request::{Filters, Limits};

    /// Raccoglie risultati e avvisi, senza limite.
    #[derive(Default)]
    struct Collect {
        hits: Vec<Hit>,
        warnings: Vec<WarningKind>,
    }

    impl HitSink for &mut Collect {
        fn check(&self) -> Result<(), Stop> {
            Ok(())
        }
        fn remaining(&self) -> usize {
            usize::MAX
        }
        fn deliver(&mut self, hits: Vec<Hit>) -> Result<(), Stop> {
            self.hits.extend(hits);
            Ok(())
        }
        fn warn(&mut self, _doc: Arc<DocPath>, kind: WarningKind) -> Result<(), Stop> {
            self.warnings.push(kind);
            Ok(())
        }
    }

    fn request(terms: &[&str]) -> SearchRequest {
        let terms = terms
            .iter()
            .map(|t| Term::parse(t, WildcardMode::Enabled).expect("termine valido"))
            .collect();
        SearchRequest {
            roots: Vec::new(),
            query: Query::new(terms, MatchOptions::default(), Combine::All)
                .and_then(|q| q.compile())
                .expect("query valida"),
            filters: Filters::default(),
            limits: Limits::default(),
        }
    }

    /// Un archivio con due voci: `a.txt` ha 20 righe "carta" e poi "zeta", `b.txt` solo
    /// "carta"; più un avviso.
    fn archive() -> Vec<ExtractEvent> {
        let line = |n: u32, text: &str| {
            ExtractEvent::Unit(TextUnit {
                location: Location::Line(n),
                text: text.to_owned(),
            })
        };
        let start = |name: &str| ExtractEvent::DocumentStart {
            inner: vec![name.to_owned()],
            format: Format::Text,
        };
        let mut events = vec![start("a.txt")];
        events.extend((1..=20).map(|n| line(n, "carta")));
        events.push(line(21, "zeta"));
        events.push(ExtractEvent::DocumentEnd);
        events.push(start("b.txt"));
        events.push(line(1, "carta"));
        events.push(ExtractEvent::DocumentEnd);
        events.push(ExtractEvent::Warning {
            inner: vec!["c.pdf".to_owned()],
            kind: lynkeus_extract::WarningKind::Corrupt,
        });
        events
    }

    /// Senza posti per trattenere (altri documenti li occupano) la voce con tutti i termini
    /// si segna da rileggere e non consegna nulla; la seconda lettura consegna solo lei, per
    /// intero, senza ripetere gli avvisi.
    #[test]
    fn documents_without_places_are_reread() {
        let request = request(&["ar", "zeta"]);
        let progress = Progress::default();
        let held = HeldHits::new(10);
        let path = Path::new("archivio.zip");
        let mut first = Collect::default();
        let rereads = {
            let mut search =
                DocSearch::new(&request, &progress, &held, &mut first, path, 0, Pass::First);
            for event in archive() {
                assert!(search.emit(event).is_ok());
            }
            search.take_rereads()
        };
        assert_eq!(rereads, [vec!["a.txt".to_owned()]]);
        assert!(first.hits.is_empty());
        assert_eq!(first.warnings, [WarningKind::Corrupt]);

        let mut second = Collect::default();
        let mut search = DocSearch::new(
            &request,
            &progress,
            &held,
            &mut second,
            path,
            0,
            Pass::Reread(&rereads),
        );
        for event in archive() {
            assert!(search.emit(event).is_ok());
        }
        assert!(search.take_rereads().is_empty());
        drop(search);
        assert_eq!(second.hits.len(), 21);
        assert!(second.hits.iter().all(|h| h.doc.inner == ["a.txt"]));
        assert!(second.warnings.is_empty());
    }
}
