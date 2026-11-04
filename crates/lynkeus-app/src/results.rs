// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Modello di presentazione della tabella dei risultati, senza egui: ordine delle righe,
//! ordinamento per colonna e selezione multipla (clic, Ctrl+clic, Maiusc+clic, frecce).
//!
//! Le righe restano nel `Vec<Hit>` nell'ordine d'arrivo; [`ResultsView`] tiene solo la
//! permutazione mostrata e la selezione per indice di risultato, così un riordino non
//! perde la selezione.

use std::cmp::{Ordering, Reverse};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use lynkeus_engine::{DocPath, Hit};
use serde::{Deserialize, Serialize};

/// Colonna su cui ordinare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SortColumn {
    /// La parola trovata.
    Word,
    /// Il primo termine che corrisponde.
    Term,
    /// Il file, poi la posizione.
    File,
    /// La posizione nel file.
    Position,
    /// Il contesto.
    Context,
}

/// Verso dell'ordinamento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SortOrder {
    /// Crescente (A → Z).
    Ascending,
    /// Decrescente (Z → A).
    Descending,
}

/// Ordinamento scelto dall'utente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SortSpec {
    /// La colonna.
    pub column: SortColumn,
    /// Il verso.
    pub order: SortOrder,
}

impl SortSpec {
    /// Ordinamento crescente su `column`.
    pub fn ascending(column: SortColumn) -> SortSpec {
        SortSpec {
            column,
            order: SortOrder::Ascending,
        }
    }

    /// L'ordinamento dopo un clic sull'intestazione `column`: la stessa colonna inverte il
    /// verso, un'altra colonna parte crescente.
    pub fn after_click(current: Option<SortSpec>, column: SortColumn) -> SortSpec {
        match current {
            Some(spec) if spec.column == column => SortSpec {
                column,
                order: match spec.order {
                    SortOrder::Ascending => SortOrder::Descending,
                    SortOrder::Descending => SortOrder::Ascending,
                },
            },
            _ => SortSpec::ascending(column),
        }
    }
}

/// Come un clic su una riga cambia la selezione.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickKind {
    /// Clic semplice: solo questa riga.
    Replace,
    /// Ctrl+clic: aggiunge o toglie la riga.
    Toggle,
    /// Maiusc+clic: l'intervallo dall'ancora a questa riga.
    Range,
}

/// Ordine mostrato e selezione delle righe.
#[derive(Debug, Clone, Default)]
pub struct ResultsView {
    /// Posizione mostrata → indice del risultato.
    order: Vec<usize>,
    /// Ordinamento scelto dall'utente; `None` = ordine predefinito.
    sort: Option<SortSpec>,
    /// Selezione per indice del risultato.
    selected: Vec<bool>,
    selected_count: usize,
    /// Riga da cui parte una selezione con Maiusc (indice del risultato).
    anchor: Option<usize>,
    /// Riga con il cursore della tastiera (indice del risultato).
    cursor: Option<usize>,
    /// Vero se il cursore è appena cambiato: la tabella lo porta in vista una volta.
    scroll_pending: bool,
}

impl ResultsView {
    /// Vista vuota con l'ordinamento dell'utente `sort`.
    pub fn new(sort: Option<SortSpec>) -> ResultsView {
        ResultsView {
            sort,
            ..ResultsView::default()
        }
    }

    /// Svuota righe e selezione per una nuova ricerca; l'ordinamento scelto resta.
    pub fn reset(&mut self) {
        *self = ResultsView::new(self.sort);
    }

    /// Aggiunge in fondo i risultati arrivati, fino a `hit_count` in tutto.
    pub fn extend_to(&mut self, hit_count: usize) {
        let start = self.order.len();
        if hit_count > start {
            self.order.extend(start..hit_count);
            self.selected.resize(hit_count, false);
        }
    }

    /// A fine ricerca: ordine dell'utente se scelto, altrimenti File → Posizione.
    pub fn finish(&mut self, hits: &[Hit], roots: &[PathBuf]) {
        self.extend_to(hits.len());
        let spec = self
            .sort
            .unwrap_or_else(|| SortSpec::ascending(SortColumn::File));
        sort_order(&mut self.order, spec, hits, roots);
    }

    /// Ordina per scelta dell'utente (clic su un'intestazione).
    pub fn set_sort(&mut self, spec: SortSpec, hits: &[Hit], roots: &[PathBuf]) {
        self.sort = Some(spec);
        self.extend_to(hits.len());
        sort_order(&mut self.order, spec, hits, roots);
        self.scroll_pending = self.cursor.is_some();
    }

    /// L'ordinamento scelto dall'utente.
    pub fn sort(&self) -> Option<SortSpec> {
        self.sort
    }

    /// Righe mostrate.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Vero se non ci sono righe.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// L'indice del risultato mostrato in posizione `position`.
    pub fn hit_at(&self, position: usize) -> Option<usize> {
        self.order.get(position).copied()
    }

    /// Vero se il risultato `hit` è selezionato.
    pub fn is_selected(&self, hit: usize) -> bool {
        self.selected.get(hit).copied().unwrap_or(false)
    }

    /// Righe selezionate.
    pub fn selected_count(&self) -> usize {
        self.selected_count
    }

    /// Indici dei risultati selezionati, nell'ordine mostrato.
    pub fn selected_hits(&self) -> impl Iterator<Item = usize> + '_ {
        self.order
            .iter()
            .copied()
            .filter(|&hit| self.is_selected(hit))
    }

    /// Clic sulla riga in posizione `position`.
    pub fn click(&mut self, position: usize, kind: ClickKind) {
        let Some(hit) = self.hit_at(position) else {
            return;
        };
        match kind {
            ClickKind::Replace => {
                self.clear_selection();
                self.set_selected(hit, true);
                self.anchor = Some(hit);
            }
            ClickKind::Toggle => {
                let now = !self.is_selected(hit);
                self.set_selected(hit, now);
                self.anchor = Some(hit);
            }
            ClickKind::Range => self.select_range_to(position),
        }
        self.cursor = Some(hit);
    }

    /// Sposta il cursore di `delta` righe (frecce); con `extend` allarga la selezione
    /// dall'ancora come Maiusc+clic. Senza righe non fa nulla.
    pub fn move_cursor(&mut self, delta: isize, extend: bool) {
        if self.order.is_empty() {
            return;
        }
        let last = self.order.len() - 1;
        let target = match self.cursor_position() {
            Some(current) => current.saturating_add_signed(delta).min(last),
            None if delta < 0 => last,
            None => 0,
        };
        let kind = if extend {
            ClickKind::Range
        } else {
            ClickKind::Replace
        };
        self.click(target, kind);
        self.scroll_pending = true;
    }

    /// Posizione mostrata del cursore.
    pub fn cursor_position(&self) -> Option<usize> {
        let cursor = self.cursor?;
        self.order.iter().position(|&hit| hit == cursor)
    }

    /// La posizione da portare in vista, una sola volta dopo un movimento del cursore.
    pub fn take_scroll_target(&mut self) -> Option<usize> {
        if std::mem::take(&mut self.scroll_pending) {
            self.cursor_position()
        } else {
            None
        }
    }

    /// Seleziona tutte le righe.
    pub fn select_all(&mut self) {
        self.selected.fill(true);
        self.selected_count = self.selected.len();
    }

    /// Toglie la selezione.
    pub fn clear_selection(&mut self) {
        self.selected.fill(false);
        self.selected_count = 0;
    }

    fn select_range_to(&mut self, position: usize) {
        let anchor_position = self
            .anchor
            .and_then(|anchor| self.order.iter().position(|&hit| hit == anchor))
            .unwrap_or(position);
        let (from, to) = if anchor_position <= position {
            (anchor_position, position)
        } else {
            (position, anchor_position)
        };
        self.clear_selection();
        let range: Vec<usize> = self.order.get(from..=to).unwrap_or_default().to_vec();
        for hit in range {
            self.set_selected(hit, true);
        }
        if self.anchor.is_none() {
            self.anchor = self.hit_at(position);
        }
    }

    fn set_selected(&mut self, hit: usize, value: bool) {
        if let Some(slot) = self.selected.get_mut(hit)
            && *slot != value
        {
            *slot = value;
            if value {
                self.selected_count += 1;
            } else {
                self.selected_count -= 1;
            }
        }
    }
}

/// Ordina la permutazione `order` (stabile: a parità resta l'ordine precedente).
///
/// Per File e Posizione la chiave del file (percorso relativo in minuscolo) si calcola una
/// volta per documento, non per riga: con 200 000 righe l'ordinamento resta sotto i 50 ms
/// (misura in `sort_200k_timing`).
fn sort_order(order: &mut [usize], spec: SortSpec, hits: &[Hit], roots: &[PathBuf]) {
    let descending = spec.order == SortOrder::Descending;
    let directed = |ordering: Ordering| {
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    };
    macro_rules! by_key {
        ($key:expr) => {{
            let key = $key;
            if descending {
                order.sort_by_cached_key(|&i| hits.get(i).map(|hit| Reverse(key(hit))));
            } else {
                order.sort_by_cached_key(|&i| hits.get(i).map(key));
            }
        }};
    }
    match spec.column {
        SortColumn::Word => by_key!(|hit: &Hit| hit.word.to_lowercase()),
        SortColumn::Term => by_key!(|hit: &Hit| hit.terms.iter().next().unwrap_or(usize::MAX)),
        SortColumn::Context => by_key!(|hit: &Hit| hit.snippet.text.to_lowercase()),
        SortColumn::File | SortColumn::Position => {
            let ranks = file_ranks(hits, roots);
            let file = |i: usize| ranks.get(i).copied();
            let location = |i: usize| hits.get(i).map(|hit| &hit.location);
            let file_first = spec.column == SortColumn::File;
            order.sort_by(|&a, &b| {
                let by_file = file(a).cmp(&file(b));
                let by_location = location(a).cmp(&location(b));
                directed(if file_first {
                    by_file.then(by_location)
                } else {
                    by_location.then(by_file)
                })
            });
        }
    }
}

/// Per ogni riga, il posto del suo file nell'ordine dei file: radice, poi percorso
/// relativo in minuscolo. Righe dello stesso file hanno lo stesso posto.
///
/// La chiave testuale si calcola una volta per documento (le righe di un file condividono
/// lo stesso `Arc<DocPath>`), poi si ordinano solo i documenti distinti.
fn file_ranks(hits: &[Hit], roots: &[PathBuf]) -> Vec<usize> {
    let mut slot_of: HashMap<(*const DocPath, usize), usize> = HashMap::new();
    let mut keys: Vec<(usize, String)> = Vec::new();
    let slots: Vec<usize> = hits
        .iter()
        .map(|hit| {
            *slot_of
                .entry((Arc::as_ptr(&hit.doc), hit.root))
                .or_insert_with(|| {
                    keys.push(file_key(hit, roots));
                    keys.len() - 1
                })
        })
        .collect();
    let mut sorted: Vec<usize> = (0..keys.len()).collect();
    sorted.sort_by(|&a, &b| keys[a].cmp(&keys[b]));
    let mut rank_of_slot = vec![0; keys.len()];
    let mut rank = 0;
    for (position, &slot) in sorted.iter().enumerate() {
        if position > 0 && keys[sorted[position - 1]] != keys[slot] {
            rank += 1;
        }
        rank_of_slot[slot] = rank;
    }
    slots.into_iter().map(|slot| rank_of_slot[slot]).collect()
}

/// Chiave del file di una riga: la radice, poi il percorso relativo in minuscolo.
fn file_key(hit: &Hit, roots: &[PathBuf]) -> (usize, String) {
    let relative = roots.get(hit.root).map_or_else(
        || hit.doc.to_string(),
        |root| hit.doc.display_relative(root),
    );
    (hit.root, relative.to_lowercase())
}

#[cfg(test)]
mod tests {
    use lynkeus_core::{Location, Snippet, TermSet};

    use super::*;

    fn hit(file: &str, line: u32, word: &str) -> Hit {
        Hit {
            doc: Arc::new(DocPath::file(PathBuf::from("radice").join(file))),
            root: 0,
            location: Location::Line(line),
            word: word.to_owned(),
            word_truncated_start: false,
            word_truncated_end: false,
            snippet: Snippet {
                text: word.to_owned(),
                highlights: Vec::new(),
                truncated_start: false,
                truncated_end: false,
            },
            terms: TermSet::single(0),
        }
    }

    fn roots() -> Vec<PathBuf> {
        vec![PathBuf::from("radice")]
    }

    fn sample() -> Vec<Hit> {
        vec![
            hit("b.txt", 3, "carta"),
            hit("a.txt", 9, "Aragosta"),
            hit("b.txt", 1, "chitarra"),
            hit("a.txt", 2, "mare"),
        ]
    }

    fn shown(view: &ResultsView, hits: &[Hit]) -> Vec<String> {
        (0..view.len())
            .filter_map(|p| view.hit_at(p))
            .map(|i| hits[i].word.clone())
            .collect()
    }

    #[test]
    fn arrival_order_until_finished_then_file_and_position() {
        let hits = sample();
        let mut view = ResultsView::new(None);
        view.extend_to(hits.len());
        assert_eq!(
            shown(&view, &hits),
            ["carta", "Aragosta", "chitarra", "mare"]
        );
        view.finish(&hits, &roots());
        assert_eq!(
            shown(&view, &hits),
            ["mare", "Aragosta", "chitarra", "carta"]
        );
    }

    #[test]
    fn user_sort_survives_finish_and_reset() {
        let hits = sample();
        let mut view = ResultsView::new(None);
        view.set_sort(SortSpec::ascending(SortColumn::Word), &hits, &roots());
        view.finish(&hits, &roots());
        assert_eq!(
            shown(&view, &hits),
            ["Aragosta", "carta", "chitarra", "mare"]
        );
        view.reset();
        assert!(view.is_empty());
        assert_eq!(view.sort(), Some(SortSpec::ascending(SortColumn::Word)));
    }

    #[test]
    fn clicking_the_same_header_reverses() {
        let first = SortSpec::after_click(None, SortColumn::Word);
        assert_eq!(first, SortSpec::ascending(SortColumn::Word));
        let second = SortSpec::after_click(Some(first), SortColumn::Word);
        assert_eq!(second.order, SortOrder::Descending);
        let other = SortSpec::after_click(Some(second), SortColumn::File);
        assert_eq!(other, SortSpec::ascending(SortColumn::File));

        let hits = sample();
        let mut view = ResultsView::new(None);
        view.set_sort(second, &hits, &roots());
        assert_eq!(
            shown(&view, &hits),
            ["mare", "chitarra", "carta", "Aragosta"]
        );
        // File decrescente: file e posizione entrambi invertiti.
        let file_down = SortSpec::after_click(Some(other), SortColumn::File);
        view.set_sort(file_down, &hits, &roots());
        assert_eq!(
            shown(&view, &hits),
            ["carta", "chitarra", "Aragosta", "mare"]
        );
        // Posizione: prima la riga, a parità il file.
        view.set_sort(SortSpec::ascending(SortColumn::Position), &hits, &roots());
        assert_eq!(
            shown(&view, &hits),
            ["chitarra", "mare", "carta", "Aragosta"]
        );
    }

    #[test]
    fn selection_partitions() {
        let hits = sample();
        let mut view = ResultsView::new(None);
        view.finish(&hits, &roots());
        // Clic semplice: una riga.
        view.click(1, ClickKind::Replace);
        assert_eq!(view.selected_count(), 1);
        // Ctrl+clic: aggiunge, poi toglie.
        view.click(3, ClickKind::Toggle);
        assert_eq!(view.selected_count(), 2);
        view.click(3, ClickKind::Toggle);
        assert_eq!(view.selected_count(), 1);
        // Maiusc+clic: dall'ancora (posizione 3, l'ultimo Ctrl+clic) alla 0.
        view.click(0, ClickKind::Range);
        assert_eq!(view.selected_count(), 4);
        // Fuori dai limiti: nulla cambia.
        view.click(99, ClickKind::Replace);
        assert_eq!(view.selected_count(), 4);
        view.clear_selection();
        assert_eq!(view.selected_count(), 0);
        view.select_all();
        assert_eq!(view.selected_hits().count(), 4);
    }

    #[test]
    fn selection_follows_rows_when_resorted() {
        let hits = sample();
        let mut view = ResultsView::new(None);
        view.finish(&hits, &roots());
        view.click(0, ClickKind::Replace);
        let chosen: Vec<usize> = view.selected_hits().collect();
        view.set_sort(SortSpec::ascending(SortColumn::Word), &hits, &roots());
        assert_eq!(view.selected_hits().collect::<Vec<_>>(), chosen);
    }

    #[test]
    fn arrows_move_and_extend_within_bounds() {
        let hits = sample();
        let mut view = ResultsView::new(None);
        view.finish(&hits, &roots());
        view.move_cursor(1, false);
        assert_eq!(view.cursor_position(), Some(0));
        view.move_cursor(-5, false);
        assert_eq!(view.cursor_position(), Some(0));
        view.move_cursor(2, true);
        assert_eq!(view.cursor_position(), Some(2));
        assert_eq!(view.selected_count(), 3);
        assert_eq!(view.take_scroll_target(), Some(2));
        assert_eq!(view.take_scroll_target(), None);
        view.move_cursor(10, false);
        assert_eq!(view.cursor_position(), Some(3));

        let mut empty = ResultsView::new(None);
        empty.move_cursor(1, false);
        assert_eq!(empty.cursor_position(), None);
    }

    /// 200 000 righe da 2 000 file (100 righe per file, file condivisi come nel motore),
    /// in ordine d'arrivo mescolato.
    fn many_hits() -> Vec<Hit> {
        let docs: Vec<Arc<DocPath>> = (0..2_000)
            .map(|f| {
                let path = PathBuf::from("radice")
                    .join(format!("cartella {}", f % 37))
                    .join("sotto cartella con un nome lungo")
                    .join(format!("Documento numero {f}.txt"));
                Arc::new(DocPath::file(path))
            })
            .collect();
        (0..200_000u32)
            .map(|i| {
                let doc = &docs[(i as usize * 7919) % docs.len()];
                let mut row = hit("x", i % 5000, &format!("parola{}", i % 977));
                row.doc = Arc::clone(doc);
                row
            })
            .collect()
    }

    /// Misura dell'ordinamento finale: `cargo test --release -p lynkeus-app --lib
    /// sort_200k -- --ignored --nocapture`. Obiettivo: meno di ~50 ms per colonna.
    #[test]
    #[ignore = "misura di prestazioni, da eseguire in release"]
    fn sort_200k_timing() {
        let hits = many_hits();
        for column in [
            SortColumn::File,
            SortColumn::Position,
            SortColumn::Word,
            SortColumn::Term,
            SortColumn::Context,
        ] {
            let mut view = ResultsView::new(Some(SortSpec::ascending(column)));
            let started = std::time::Instant::now();
            view.finish(&hits, &roots());
            println!("{column:?}: {:?}", started.elapsed());
        }
    }
}
