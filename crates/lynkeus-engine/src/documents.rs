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

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use lynkeus_core::{Location, TextUnit};
use lynkeus_extract::{EntrySink, EventSink, ExtractEvent, ExtractOptions, Stopped, line_location};

use crate::cache::{CacheKey, MAX_ENTRY_BYTES, TextCache};
use crate::doc::DocPath;
use crate::event::WarningKind;
use crate::pool::{EntryEnd, ExtractEnd, ExtractorPool, absolute_path};

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
        let path = absolute_path(path);
        let path = path.as_path();
        let cache = self.active_cache();
        let key = cache.and_then(|_| CacheKey::for_file(path, options));
        if let (Some(cache), Some(key)) = (cache, &key)
            && let Some(events) = cache.get(key)
        {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
            for event in events {
                if stop() || sink.emit(event).is_err() {
                    return (ExtractEnd::Stopped, Origin::Cache);
                }
            }
            return (ExtractEnd::Completed, Origin::Cache);
        }
        let mut tee = Tee {
            sink,
            saved: key.as_ref().map(|_| Vec::new()),
            saved_bytes: 0,
        };
        let end = self.pool.extract(path, options, timeout, stop, &mut tee);
        if end == ExtractEnd::Completed
            && let (Some(cache), Some(key), Some(events)) = (cache, &key, tee.saved)
        {
            // La cache è un derivato: se non si scrive, si rileggerà il file.
            let _saved = cache.put(key, &events);
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
        let (end, _) = self.extract(&doc.path, options, timeout, stop, &mut window);
        match end {
            ExtractEnd::Completed => {}
            ExtractEnd::Stopped => return Ok(None),
            // Se l'unità e le sue vicine sono già arrivate, bastano.
            ExtractEnd::Failed(_) if window.is_complete() => {}
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

/// Passa gli eventi a chi chiama e ne tiene una copia per la cache, finché è piccola.
struct Tee<'a> {
    sink: &'a mut dyn EventSink,
    /// `None` = non si salva (cache spenta, voce troppo grande, avviso passeggero).
    saved: Option<Vec<ExtractEvent>>,
    saved_bytes: usize,
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
        self.state != WindowState::Before
    }

    fn is_complete(&self) -> bool {
        self.state == WindowState::Done
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
            WindowState::Done => {}
        }
    }
}

impl EventSink for UnitWindow<'_> {
    fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped> {
        match event {
            ExtractEvent::DocumentStart { inner, .. } => {
                self.in_target = inner == self.inner;
            }
            ExtractEvent::DocumentEnd => self.in_target = false,
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
        // Si lascia finire l'estrazione: il documento completo finisce nella cache.
        Ok(())
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
            assert!(window.emit(event).is_ok());
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

    #[test]
    fn transient_warnings_are_not_cached() {
        use lynkeus_extract::WarningKind as K;
        assert!(is_transient(K::InUse));
        assert!(is_transient(K::AccessDenied));
        assert!(!is_transient(K::Encrypted));
        assert!(!is_transient(K::Corrupt));
    }
}
