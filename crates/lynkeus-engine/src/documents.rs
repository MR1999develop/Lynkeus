// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Il testo dei documenti (PDF, Office, archivi): dalla cache se c'è, altrimenti dal pool
//! dei processi estrattori, salvandolo poi nella cache.
//!
//! [`DocumentSource`] è l'unico punto che lega cache e pool: la ricerca e l'anteprima
//! dell'app lo usano allo stesso modo. Una voce si salva solo se l'estrazione è finita
//! normalmente e non contiene avvisi passeggeri (file in uso, accesso negato, errori di
//! lettura), che alla prossima lettura potrebbero non esserci.
//!
//! I percorsi si rendono assoluti prima della chiave della cache e della richiesta al
//! processo estrattore: lo stesso file ha una sola chiave, e l'estrattore (con un'altra
//! cartella corrente) legge proprio quel file.
//!
//! Una scrittura della cache che fallisce non ferma la lettura: la cache la conta
//! ([`TextCache::write_failures`]) e l'app la registra.
//!
//! L'anteprima ([`DocumentSource::units_around`]) ferma l'estrazione appena ha l'unità e
//! le sue vicine, se il documento non va comunque salvato nella cache (cache spenta, voce
//! troppo grande): il processo non resta occupato fino alla scadenza.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use lynkeus_core::{Location, TextUnit};
use lynkeus_extract::{EntrySink, EventSink, ExtractEvent, ExtractOptions, Stopped, line_location};

use crate::cache::{CacheKey, CacheRead, MAX_ENTRY_BYTES, TextCache};
use crate::doc::DocPath;
use crate::event::WarningKind;
use crate::pool::{EntryEnd, ExtractBudget, ExtractEnd, ExtractorPool, absolute_path};

/// Da dove vengono gli eventi di un file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Dalla cache: nessun processo usato.
    Cache,
    /// Da un processo estrattore.
    Extractor,
}

/// Il testo dei documenti: pool di processi estrattori più cache facoltativa.
#[derive(Debug)]
pub struct DocumentSource {
    pool: ExtractorPool,
    cache: Option<TextCache>,
    cache_enabled: AtomicBool,
    cache_hits: AtomicU64,
}

impl DocumentSource {
    /// Una fonte con `pool` e, se c'è, `cache` (accesa).
    pub fn new(pool: ExtractorPool, cache: Option<TextCache>) -> DocumentSource {
        DocumentSource {
            pool,
            cache,
            cache_enabled: AtomicBool::new(true),
            cache_hits: AtomicU64::new(0),
        }
    }

    /// Il pool.
    pub fn pool(&self) -> &ExtractorPool {
        &self.pool
    }

    /// La cache, se configurata (accesa o spenta).
    pub fn cache(&self) -> Option<&TextCache> {
        self.cache.as_ref()
    }

    /// Accende o spegne la cache (le voci restano sul disco: si svuota a parte).
    pub fn set_cache_enabled(&self, enabled: bool) {
        self.cache_enabled.store(enabled, Ordering::Relaxed);
    }

    /// Vero se la cache c'è ed è accesa.
    pub fn cache_enabled(&self) -> bool {
        self.cache.is_some() && self.cache_enabled.load(Ordering::Relaxed)
    }

    /// File letti dalla cache finora (diagnostica e test).
    pub fn cache_hits(&self) -> u64 {
        self.cache_hits.load(Ordering::Relaxed)
    }

    fn active_cache(&self) -> Option<&TextCache> {
        self.cache.as_ref().filter(|_| self.cache_enabled())
    }

    /// Gli eventi d'estrazione di `path`, mandati a `sink` man mano; come
    /// [`ExtractorPool::extract`], più la cache.
    pub fn extract(
        &self,
        path: &Path,
        options: &ExtractOptions,
        timeout: Duration,
        stop: &dyn Fn() -> bool,
        sink: &mut dyn EventSink,
    ) -> (ExtractEnd, Origin) {
        let budget = ExtractBudget::new(timeout);
        self.extract_with(path, options, &budget, stop, sink, SinkStop::Stops)
    }

    /// Come [`DocumentSource::extract`], con il tempo di `budget`.
    pub(crate) fn extract_within(
        &self,
        path: &Path,
        options: &ExtractOptions,
        budget: &ExtractBudget<'_>,
        stop: &dyn Fn() -> bool,
        sink: &mut dyn EventSink,
    ) -> (ExtractEnd, Origin) {
        self.extract_with(path, options, budget, stop, sink, SinkStop::Stops)
    }

    /// L'estrazione con la cache; `when_sink_stops` dice se un destinatario che si ferma
    /// ferma anche il salvataggio nella cache.
    fn extract_with(
        &self,
        path: &Path,
        options: &ExtractOptions,
        budget: &ExtractBudget<'_>,
        stop: &dyn Fn() -> bool,
        sink: &mut dyn EventSink,
        when_sink_stops: SinkStop,
    ) -> (ExtractEnd, Origin) {
        let path = absolute_path(path);
        let path = path.as_path();
        let cache = self.active_cache();
        let key = cache.and_then(|_| CacheKey::for_file(path, options));
        if let (Some(cache), Some(key)) = (cache, &key) {
            let mut checked = StopChecked {
                stop,
                sink: &mut *sink,
            };
            match cache.read(key, &mut checked) {
                CacheRead::Delivered => {
                    self.cache_hits.fetch_add(1, Ordering::Relaxed);
                    return (ExtractEnd::Completed, Origin::Cache);
                }
                CacheRead::Stopped => {
                    self.cache_hits.fetch_add(1, Ordering::Relaxed);
                    return (ExtractEnd::Stopped, Origin::Cache);
                }
                CacheRead::Missing => {}
            }
        }
        let mut tee = Tee {
            sink,
            saved: key.as_ref().map(|_| Vec::new()),
            saved_bytes: 0,
            when_sink_stops,
            sink_done: false,
        };
        let end = self
            .pool
            .extract_within(path, options, budget, stop, &mut tee);
        if end == ExtractEnd::Completed
            && let (Some(cache), Some(key), Some(events)) = (cache, &key, tee.saved)
        {
            // La cache è un derivato: se non si scrive, si rileggerà il file. L'errore lo
            // conta la cache (`write_failures`) e lo registra l'app.
            let _counted = cache.put(key, &events);
        }
        (end, Origin::Extractor)
    }

    /// I byte originali della voce d'archivio `doc` (al più `max_bytes`), mandati a `sink`
    /// man mano: sempre dal processo estrattore, mai dalla cache, che ha solo il testo. Come
    /// [`ExtractorPool::extract_entry`].
    pub fn extract_entry(
        &self,
        doc: &DocPath,
        max_bytes: u64,
        options: &ExtractOptions,
        timeout: Duration,
        stop: &dyn Fn() -> bool,
        sink: &mut dyn EntrySink,
    ) -> EntryEnd {
        self.pool.extract_entry(
            &doc.path, &doc.inner, max_bytes, options, timeout, stop, sink,
        )
    }

    /// Le unità del documento `doc` vicine a `location`: al più `radius` prima e `radius`
    /// dopo, più quella in `location`, nell'ordine del documento. Le unità di più righe con
    /// posizione a righe ([`Location::Line`], [`Location::PageLine`]) si dividono in una per
    /// riga, così ogni risultato ha la sua unità.
    ///
    /// `Ok(None)` se il documento o la posizione non ci sono (il file è cambiato) o se `stop`
    /// è diventato vero; `Err` con il motivo se il documento non si legge.
    pub fn units_around(
        &self,
        doc: &DocPath,
        location: &Location,
        radius: usize,
        options: &ExtractOptions,
        timeout: Duration,
        stop: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<TextUnit>>, WarningKind> {
        let mut window = UnitWindow::new(&doc.inner, location, radius);
        let budget = ExtractBudget::new(timeout);
        let (end, _) = self.extract_with(
            &doc.path,
            options,
            &budget,
            stop,
            &mut window,
            SinkStop::FinishesForTheCache,
        );
        if stop() {
            return Ok(None);
        }
        match end {
            ExtractEnd::Completed => {}
            // Finestra completa: si è fermata da sola, o le unità che servono sono arrivate
            // prima del guasto.
            ExtractEnd::Stopped | ExtractEnd::Failed(_) if window.is_complete() => {}
            ExtractEnd::Stopped => return Ok(None),
            ExtractEnd::Failed(failure) => return Err(failure.warning_kind()),
        }
        if let Some(kind) = window.warning
            && !window.found()
        {
            return Err(WarningKind::from_extract(kind));
        }
        Ok(window.finish())
    }
}

/// Avvisi che dipendono dal momento e non dal contenuto del file: non si mettono in cache.
fn is_transient(kind: lynkeus_extract::WarningKind) -> bool {
    use lynkeus_extract::WarningKind as K;
    matches!(
        kind,
        K::NotFound | K::AccessDenied | K::InUse | K::Io | K::Timeout
    )
}

/// Che cosa vuol dire un destinatario che si ferma.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SinkStop {
    /// Si ferma anche l'estrazione (ricerca annullata o al limite).
    Stops,
    /// Il destinatario ha finito; l'estrazione continua solo se il documento va ancora
    /// salvato nella cache (anteprima).
    FinishesForTheCache,
}

/// Passa gli eventi a chi chiama e ne tiene una copia per la cache, finché è piccola.
struct Tee<'a> {
    sink: &'a mut dyn EventSink,
    /// `None` = non si salva (cache spenta, voce troppo grande, avviso passeggero).
    saved: Option<Vec<ExtractEvent>>,
    saved_bytes: usize,
    when_sink_stops: SinkStop,
    /// Il destinatario ha finito: gli eventi vanno solo alla copia.
    sink_done: bool,
}

impl EventSink for Tee<'_> {
    fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped> {
        if let Some(saved) = &mut self.saved {
            let bytes = approximate_bytes(&event);
            self.saved_bytes = self.saved_bytes.saturating_add(bytes);
            let transient = matches!(
                &event,
                ExtractEvent::Warning { kind, .. } if is_transient(*kind)
            );
            if transient || self.saved_bytes > MAX_ENTRY_BYTES {
                self.saved = None;
            } else {
                saved.push(event.clone());
            }
        }
        if self.sink_done {
            // Senza più nulla da salvare, inutile continuare.
            return if self.saved.is_some() {
                Ok(())
            } else {
                Err(Stopped)
            };
        }
        match self.sink.emit(event) {
            Err(Stopped)
                if self.when_sink_stops == SinkStop::FinishesForTheCache
                    && self.saved.is_some() =>
            {
                self.sink_done = true;
                Ok(())
            }
            other => other,
        }
    }
}

/// Passa gli eventi a `sink` finché `stop` resta falso (lettura dalla cache).
struct StopChecked<'a> {
    stop: &'a dyn Fn() -> bool,
    sink: &'a mut dyn EventSink,
}

impl EventSink for StopChecked<'_> {
    fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped> {
        if (self.stop)() {
            return Err(Stopped);
        }
        self.sink.emit(event)
    }
}

/// Byte approssimati di un evento codificato: testo e nomi più un margine fisso.
fn approximate_bytes(event: &ExtractEvent) -> usize {
    const OVERHEAD: usize = 32;
    let names = |inner: &[String]| inner.iter().map(String::len).sum::<usize>();
    OVERHEAD
        + match event {
            ExtractEvent::DocumentStart { inner, .. }
            | ExtractEvent::Warning { inner, .. }
            | ExtractEvent::Skipped { inner, .. } => names(inner),
            ExtractEvent::Unit(unit) => unit.text.len(),
            ExtractEvent::DocumentEnd => 0,
        }
}

/// Dove si trova la finestra di [`UnitWindow`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowState {
    /// L'unità cercata non è ancora arrivata.
    Before,
    /// Trovata: mancano ancora `left` unità dopo.
    After { left: usize },
    /// Completa: il resto si ignora.
    Done,
    /// Il documento è finito senza l'unità cercata.
    Missing,
}

/// Raccoglie le unità intorno a una posizione di un documento, senza tenere tutto il
/// documento in memoria.
struct UnitWindow<'a> {
    inner: &'a [String],
    location: &'a Location,
    radius: usize,
    in_target: bool,
    state: WindowState,
    units: VecDeque<TextUnit>,
    warning: Option<lynkeus_extract::WarningKind>,
}

impl<'a> UnitWindow<'a> {
    fn new(inner: &'a [String], location: &'a Location, radius: usize) -> UnitWindow<'a> {
        UnitWindow {
            inner,
            location,
            radius,
            in_target: false,
            state: WindowState::Before,
            units: VecDeque::new(),
            warning: None,
        }
    }

    fn found(&self) -> bool {
        matches!(self.state, WindowState::After { .. } | WindowState::Done)
    }

    /// Vero se non arriva più nulla che serva: unità e vicine raccolte, o documento finito.
    fn is_complete(&self) -> bool {
        matches!(self.state, WindowState::Done | WindowState::Missing)
    }

    fn finish(self) -> Option<Vec<TextUnit>> {
        self.found().then(|| self.units.into())
    }

    fn push(&mut self, unit: TextUnit) {
        match self.state {
            WindowState::Before => {
                let is_target = unit.location == *self.location;
                self.units.push_back(unit);
                if is_target {
                    self.state = if self.radius == 0 {
                        WindowState::Done
                    } else {
                        WindowState::After { left: self.radius }
                    };
                } else if self.units.len() > self.radius {
                    self.units.pop_front();
                }
            }
            WindowState::After { left } => {
                self.units.push_back(unit);
                self.state = if left <= 1 {
                    WindowState::Done
                } else {
                    WindowState::After { left: left - 1 }
                };
            }
            WindowState::Done | WindowState::Missing => {}
        }
    }

    /// Fine del documento cercato: dopo non arrivano altre sue unità (le voci sono uniche nel
    /// loro archivio).
    fn end_of_target(&mut self) {
        self.state = match self.state {
            WindowState::Before | WindowState::Missing => WindowState::Missing,
            WindowState::After { .. } | WindowState::Done => WindowState::Done,
        };
    }
}

impl EventSink for UnitWindow<'_> {
    /// Raccoglie le unità; quando la finestra è completa chiede di fermarsi (chi la usa
    /// decide se l'estrazione continua per la cache).
    fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped> {
        match event {
            ExtractEvent::DocumentStart { inner, .. } => {
                self.in_target = inner == self.inner;
            }
            ExtractEvent::DocumentEnd => {
                if self.in_target {
                    self.end_of_target();
                }
                self.in_target = false;
            }
            ExtractEvent::Unit(unit) if self.in_target && !self.is_complete() => {
                for piece in split_lines(unit) {
                    self.push(piece);
                }
            }
            ExtractEvent::Warning { inner, kind } if inner == self.inner => {
                self.warning.get_or_insert(kind);
            }
            ExtractEvent::Unit(_) | ExtractEvent::Warning { .. } | ExtractEvent::Skipped { .. } => {
            }
        }
        if self.is_complete() {
            Err(Stopped)
        } else {
            Ok(())
        }
    }
}

/// Divide un'unità a righe in un'unità per riga (posizioni con [`line_location`]); le altre
/// restano intere.
fn split_lines(unit: TextUnit) -> Vec<TextUnit> {
    let by_line = matches!(unit.location, Location::Line(_) | Location::PageLine { .. });
    if !by_line || !unit.text.contains('\n') {
        return vec![unit];
    }
    let text = unit.text.strip_suffix('\n').unwrap_or(&unit.text);
    text.split('\n')
        .enumerate()
        .map(|(i, line)| TextUnit {
            location: line_location(&unit.location, u32::try_from(i).unwrap_or(u32::MAX)),
            text: line.strip_suffix('\r').unwrap_or(line).to_owned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use lynkeus_extract::Format;

    use super::*;

    fn start(inner: &[&str]) -> ExtractEvent {
        ExtractEvent::DocumentStart {
            inner: inner.iter().map(|s| (*s).to_owned()).collect(),
            format: Format::Text,
        }
    }

    fn unit(location: Location, text: &str) -> ExtractEvent {
        ExtractEvent::Unit(TextUnit {
            location,
            text: text.to_owned(),
        })
    }

    fn window(
        events: Vec<ExtractEvent>,
        inner: &[&str],
        at: &Location,
        radius: usize,
    ) -> Option<Vec<String>> {
        let inner: Vec<String> = inner.iter().map(|s| (*s).to_owned()).collect();
        let mut window = UnitWindow::new(&inner, at, radius);
        for event in events {
            if window.emit(event).is_err() {
                assert!(window.is_complete(), "fermata prima di essere completa");
                break;
            }
        }
        window
            .finish()
            .map(|units| units.into_iter().map(|u| u.text).collect())
    }

    #[test]
    fn window_partitions() {
        let lines = || {
            vec![
                start(&["a.zip", "b.txt"]),
                unit(Location::Line(1), "uno\ndue\ntre\n"),
                unit(Location::Line(4), "quattro\r\ncinque"),
                ExtractEvent::DocumentEnd,
            ]
        };
        // In mezzo, all'inizio, alla fine; raggio zero; posizione assente; altro documento.
        assert_eq!(
            window(lines(), &["a.zip", "b.txt"], &Location::Line(3), 1),
            Some(vec!["due".into(), "tre".into(), "quattro".into()])
        );
        assert_eq!(
            window(lines(), &["a.zip", "b.txt"], &Location::Line(1), 2),
            Some(vec!["uno".into(), "due".into(), "tre".into()])
        );
        assert_eq!(
            window(lines(), &["a.zip", "b.txt"], &Location::Line(5), 1),
            Some(vec!["quattro".into(), "cinque".into()])
        );
        assert_eq!(
            window(lines(), &["a.zip", "b.txt"], &Location::Line(2), 0),
            Some(vec!["due".into()])
        );
        assert_eq!(
            window(lines(), &["a.zip", "b.txt"], &Location::Line(9), 1),
            None
        );
        assert_eq!(window(lines(), &["a.zip"], &Location::Line(2), 1), None);
    }

    #[test]
    fn paragraphs_are_not_split() {
        let events = vec![
            start(&[]),
            unit(Location::Paragraph(1), "prima riga\nseconda"),
            unit(Location::Paragraph(2), "carta"),
            ExtractEvent::DocumentEnd,
        ];
        assert_eq!(
            window(events, &[], &Location::Paragraph(2), 3),
            Some(vec!["prima riga\nseconda".into(), "carta".into()])
        );
    }

    /// Con la cache che salva, la finestra completa non ferma l'estrazione finché il
    /// documento è da salvare; senza salvataggio, o oltre il massimo di una voce, si ferma.
    #[test]
    fn a_complete_window_stops_the_extraction_unless_the_cache_needs_it() {
        let location = Location::Line(1);
        let inner: Vec<String> = Vec::new();
        let big = "x".repeat(1024 * 1024);
        let events = || {
            let mut events = vec![start(&[]), unit(Location::Line(1), "carta")];
            events.extend((2..=70).map(|n| unit(Location::Line(n), &big)));
            events.push(ExtractEvent::DocumentEnd);
            events
        };
        let feed = |saving: bool| -> (usize, bool) {
            let mut window = UnitWindow::new(&inner, &location, 0);
            let mut tee = Tee {
                sink: &mut window,
                saved: saving.then(Vec::new),
                saved_bytes: 0,
                when_sink_stops: SinkStop::FinishesForTheCache,
                sink_done: false,
            };
            let mut accepted = 0;
            for event in events() {
                if tee.emit(event).is_err() {
                    break;
                }
                accepted += 1;
            }
            (accepted, tee.saved.is_some())
        };
        // Senza cache si ferma alla seconda unità (quella cercata).
        assert_eq!(feed(false), (1, false));
        // Con la cache continua finché la voce non supera il massimo, poi si ferma.
        let (accepted, still_saving) = feed(true);
        assert!(!still_saving);
        assert!((60..70).contains(&accepted), "{accepted}");
    }

    #[test]
    fn transient_warnings_are_not_cached() {
        use lynkeus_extract::WarningKind as K;
        assert!(is_transient(K::InUse));
        assert!(is_transient(K::AccessDenied));
        assert!(!is_transient(K::Encrypted));
        assert!(!is_transient(K::Corrupt));
    }
}
