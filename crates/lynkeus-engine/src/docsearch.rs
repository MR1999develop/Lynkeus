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
//! chi chiama rilegge il file con [`Pass::Reread`], che guarda solo quei documenti e chiede
//! di fermare l'estrazione appena li ha consegnati tutti (i documenti seguono lo stesso
//! ordine, quindi un errore più avanti nel file, come una scadenza, non si ripete). Se la
//! seconda lettura si interrompe prima, ognuno dei documenti mancanti ha un avviso.

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
use crate::step::steps;

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
    /// Seconda lettura: le voci dei documenti non ancora consegnati.
    pending: Vec<Vec<String>>,
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
            pending: match pass {
                Pass::First => Vec::new(),
                Pass::Reread(wanted) => wanted.to_vec(),
            },
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

    /// Avvisa che l'estrazione si è interrotta. Prima lettura: l'avviso va al documento
    /// aperto (i suoi risultati si scartano), altrimenti al file. Seconda lettura: a ogni
    /// documento da rileggere non ancora consegnato.
    pub(crate) fn warn_interrupted(&mut self, kind: WarningKind) -> Result<(), Stop> {
        let open = self.open.take();
        if let Pass::Reread(_) = self.pass {
            for inner in std::mem::take(&mut self.pending) {
                let doc = self.doc(inner);
                self.sink.warn(doc, kind)?;
            }
            return Ok(());
        }
        let doc = match open {
            Some(open) => open.doc,
            None => self.doc(Vec::new()),
        };
        self.sink.warn(doc, kind)
    }

    /// Vero se la seconda lettura ha consegnato tutti i suoi documenti: il resto del file non
    /// serve.
    fn rereads_done(&self) -> bool {
        matches!(self.pass, Pass::Reread(_)) && self.pending.is_empty()
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
        self.pending.retain(|inner| *inner != open.doc.inner);
        match open.batcher.finish() {
            Verdict::Hits(hits) if hits.is_empty() => Ok(()),
            Verdict::Hits(hits) => self.sink.deliver(hits),
            Verdict::Reread => {
                self.rereads.push(open.doc.inner.clone());
                Ok(())
            }
        }
    }

    /// Cerca in un'unità del documento aperto, a passi come i file di testo ([`steps`]).
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
        for step in steps(&unit.text) {
            self.sink.check()?;
            // Un'unità ha al più MAX_UNIT_TEXT_BYTES byte: le righe stanno in un u32.
            let lines_before = u32::try_from(step.lines_before).unwrap_or(u32::MAX);
            let room = self.sink.remaining();
            for found in find_words(label.query, step.text) {
                let make = || {
                    let line = lines_before.saturating_add(found.line);
                    label.hit(step.text, &found, line_location(&unit.location, line))
                };
                if let Some(batch) = open.batcher.push(found.terms, room, make) {
                    self.sink.deliver(batch)?;
                }
            }
        }
        Ok(())
    }
}

impl<S: HitSink> EventSink for DocSearch<'_, S> {
    /// Ferma l'estrazione se la ricerca si ferma, o se la seconda lettura ha consegnato tutti
    /// i suoi documenti (allora [`DocSearch::stopped`] resta `None`).
    fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped> {
        if self.stopped.is_some() || self.rereads_done() {
            return Err(Stopped);
        }
        self.handle(event).map_err(|stop| {
            self.stopped = Some(stop);
            Stopped
        })?;
        if self.rereads_done() {
            return Err(Stopped);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use lynkeus_core::{Combine, Location, MatchOptions, Query, Term, WildcardMode};
    use lynkeus_extract::{Format, MAX_UNIT_TEXT_BYTES};

    use super::*;
    use crate::event::Hit;
    use crate::request::{Filters, Limits};
    use crate::step::{STEP_MAX_BYTES, STEP_MAX_LINES};

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

    /// Un'unità più lunga di un passo si cerca a passi (la memoria dei risultati di un passo
    /// è limitata) con le stesse righe dell'unità intera, anche oltre i confini dei passi.
    #[test]
    fn a_unit_longer_than_a_step_keeps_its_line_numbers() {
        let lines = 3 * STEP_MAX_LINES;
        let mut text = String::new();
        for n in 0..lines {
            let word = if n % 997 == 0 { "carta" } else { "riga" };
            text.push_str(&format!("{word} {n}\n"));
        }
        assert!(text.len() > 2 * STEP_MAX_BYTES && text.len() <= MAX_UNIT_TEXT_BYTES);
        let mut request = request(&["ar"]);
        request.limits.max_results = usize::MAX;
        let progress = Progress::default();
        let held = HeldHits::new(usize::MAX);
        let mut found = Collect::default();
        {
            let path = Path::new("documento.pdf");
            let mut search =
                DocSearch::new(&request, &progress, &held, &mut found, path, 0, Pass::First);
            let events = [
                ExtractEvent::DocumentStart {
                    inner: Vec::new(),
                    format: Format::Pdf,
                },
                ExtractEvent::Unit(TextUnit {
                    location: Location::PageLine { page: 3, line: 5 },
                    text,
                }),
                ExtractEvent::DocumentEnd,
            ];
            for event in events {
                assert!(search.emit(event).is_ok());
            }
        }
        let found: Vec<Location> = found.hits.into_iter().map(|h| h.location).collect();
        let expected: Vec<Location> = (0..lines)
            .filter(|n| n % 997 == 0)
            .map(|n| Location::PageLine {
                page: 3,
                line: 5 + u32::try_from(n).unwrap_or(u32::MAX),
            })
            .collect();
        assert_eq!(found, expected);
    }

    /// Senza posti per trattenere (altri documenti li occupano) la voce con tutti i termini
    /// si segna da rileggere e non consegna nulla; la seconda lettura consegna solo lei, per
    /// intero, senza ripetere gli avvisi, e ferma l'estrazione appena l'ha consegnata.
    #[test]
    fn documents_without_places_are_reread() {
        let request = request(&["ar", "zeta"]);
        let progress = Progress::default();
        // Posto per al più 10 risultati: `a.txt` ne ha 20 prima di "zeta".
        let held = HeldHits::new(10 * size_of::<Hit>());
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
        let stopped_at = archive()
            .into_iter()
            .position(|event| search.emit(event).is_err());
        // Inizio, 21 righe, fine di `a.txt`: poi l'estrazione non serve più.
        assert_eq!(stopped_at, Some(22));
        assert_eq!(search.stopped(), None, "non è un arresto della ricerca");
        assert!(search.take_rereads().is_empty());
        drop(search);
        assert_eq!(second.hits.len(), 21);
        assert!(second.hits.iter().all(|h| h.doc.inner == ["a.txt"]));
        assert!(second.warnings.is_empty());
    }
}
