// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Modello di presentazione della tabella dei risultati, senza egui: ordine delle righe,
//! ordinamento per colonna e selezione multipla (clic, Ctrl+clic, Maiusc+clic, frecce).
//!
//! Le righe restano nel `Vec<Hit>` nell'ordine d'arrivo; [`ResultsView`] tiene solo la
//! permutazione mostrata e la selezione per indice di risultato, così un riordino non
//! perde la selezione.
//!
//! Senza un ordinamento scelto le righe arrivano in coda e a fine ricerca si ordinano per
//! File → Posizione; con un ordinamento scelto ogni riga entra subito al suo posto (a
//! parità resta l'ordine d'arrivo, come nell'ordinamento completo).
//!
//! Inerzia del display: i clic arrivano per risultato, non per posizione
//! ([`ResultsView::click_hit`]), e le righe che entrano sopra la riga da tenere ferma (il
//! cursore se è in vista, altrimenti la prima riga visibile) diventano uno scorrimento
//! della tabella ([`ResultsView::take_scroll_shift`]): nulla si sposta sotto il puntatore.
//!
//! Gli indici per documento e per gruppo di righe consecutive dello stesso documento si
//! aggiornano a ogni cambio dell'ordine: posizione del cursore e passi tra i risultati
//! ([`ResultsView::step_target`]) costano O(1), anche con 200 000 righe.

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
    pub const fn ascending(column: SortColumn) -> SortSpec {
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

/// Un passo tra i risultati, dall'anteprima o dal menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Risultato successivo nello stesso file.
    NextInFile,
    /// Risultato precedente nello stesso file.
    PreviousInFile,
    /// Primo risultato del file successivo.
    NextFile,
    /// Primo risultato del file precedente.
    PreviousFile,
}

/// Ordine mostrato e selezione delle righe.
#[derive(Debug, Clone, Default)]
pub struct ResultsView {
    /// Posizione mostrata → indice del risultato.
    order: Vec<usize>,
    /// Indice del risultato → posizione mostrata (l'inversa di `order`).
    position_of: Vec<usize>,
    /// Ordinamento scelto dall'utente; `None` = ordine predefinito.
    sort: Option<SortSpec>,
    /// I documenti dei risultati.
    docs: DocIndex,
    /// I gruppi di righe dello stesso documento nell'ordine mostrato.
    groups: GroupIndex,
    /// Selezione per indice del risultato.
    selected: Vec<bool>,
    selected_count: usize,
    /// Riga da cui parte una selezione con Maiusc (indice del risultato).
    anchor: Option<usize>,
    /// Riga con il cursore della tastiera (indice del risultato).
    cursor: Option<usize>,
    /// Vero se il cursore è appena cambiato: la tabella lo porta in vista una volta.
    scroll_pending: bool,
    /// Vero se la riga del cursore deve prendere il focus della tastiera (dopo un clic o
    /// una freccia), finché la tabella non la disegna.
    focus_pending: bool,
    /// Le posizioni della prima e dell'ultima riga dell'ultimo disegno della tabella.
    visible: Option<(usize, usize)>,
    /// La riga da tenere ferma sullo schermo quando entrano righe sopra di lei.
    pinned: Option<Pinned>,
}

/// La riga da tenere ferma sullo schermo e dov'era quando la tabella l'ha disegnata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pinned {
    /// Il risultato.
    hit: usize,
    /// La sua posizione in quel disegno.
    position: usize,
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

    /// Mostra i risultati arrivati dopo quelli già mostrati (`hits` sono tutti i risultati
    /// della ricerca, nell'ordine d'arrivo): in coda senza un ordinamento scelto, al loro
    /// posto con un ordinamento scelto.
    pub fn extend_to(&mut self, hits: &[Hit], roots: &[PathBuf]) {
        let known = self.position_of.len();
        if hits.len() <= known {
            return;
        }
        self.docs.extend(hits, roots);
        self.selected.resize(hits.len(), false);
        self.position_of.resize(hits.len(), 0);
        match self.sort {
            None => {
                let first_new = self.order.len();
                for hit in known..hits.len() {
                    self.position_of[hit] = self.order.len();
                    self.order.push(hit);
                }
                self.groups.append(&self.order, first_new, &self.docs);
            }
            Some(spec) => {
                insert_sorted(&mut self.order, known..hits.len(), spec, hits, &self.docs);
                self.reindex();
            }
        }
    }

    /// A fine ricerca: ordine dell'utente se scelto (già applicato), altrimenti File →
    /// Posizione; se le righe si spostano, la riga del cursore torna in vista.
    pub fn finish(&mut self, hits: &[Hit], roots: &[PathBuf]) {
        self.extend_to(hits, roots);
        if self.sort.is_none() {
            let spec = SortSpec::ascending(SortColumn::File);
            sort_order(&mut self.order, spec, hits, &self.docs);
            self.reindex();
            self.forget_visible_rows();
            self.scroll_pending = self.cursor.is_some();
        }
    }

    /// Ordina per scelta dell'utente (clic su un'intestazione).
    pub fn set_sort(&mut self, spec: SortSpec, hits: &[Hit], roots: &[PathBuf]) {
        self.extend_to(hits, roots);
        self.sort = Some(spec);
        sort_order(&mut self.order, spec, hits, &self.docs);
        self.reindex();
        self.forget_visible_rows();
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

    /// Clic sulla riga del risultato `hit`: cambia la selezione, porta lì il cursore e
    /// chiede il focus della tastiera per quella riga. Il risultato, non la posizione: con
    /// un ordinamento scelto le righe nuove entrano sopra durante la ricerca e la posizione
    /// cliccata può mostrare ormai un altro risultato. Un risultato non mostrato non cambia
    /// nulla.
    pub fn click_hit(&mut self, hit: usize, kind: ClickKind) {
        if let Some(position) = self.position(hit) {
            self.click_at(position, kind);
        }
    }

    /// Doppio clic sulla riga del risultato `hit`: vero se va aperto, cioè se il primo clic
    /// era sullo stesso risultato (il cursore è lì). Se tra i due clic le righe si sono
    /// spostate e il secondo è caduto su un altro risultato, vale come clic semplice su
    /// quello e non apre nulla.
    pub fn double_click_hit(&mut self, hit: usize) -> bool {
        if self.cursor == Some(hit) && self.position(hit).is_some() {
            return true;
        }
        self.click_hit(hit, ClickKind::Replace);
        false
    }

    /// Il focus della tastiera è arrivato con Tab o F6 sulla riga del risultato `hit`: il
    /// cursore va lì e la selezione resta com'è, come nella ListView di Windows (Ctrl+C
    /// copia ancora le righe scelte, Invio apre la riga con il focus). Un risultato non
    /// mostrato non cambia nulla.
    pub fn focus_hit(&mut self, hit: usize) {
        if self.position(hit).is_some() {
            self.cursor = Some(hit);
            self.pin();
        }
    }

    /// La tabella ha appena disegnato le righe dalla posizione `first` a `last` comprese.
    /// Se poi entrano righe ordinate sopra, resta ferma sullo schermo la riga del cursore,
    /// se è tra queste, altrimenti la prima: ciò che l'utente guarda o sta per cliccare non
    /// si sposta, e la riga selezionata resta in vista ([`ResultsView::take_scroll_shift`]).
    pub fn set_visible_rows(&mut self, first: usize, last: usize) {
        self.visible = Some((first, last));
        self.pin();
    }

    /// Di quante righe è scesa la riga ferma ([`ResultsView::set_visible_rows`]) per le
    /// righe entrate sopra di lei: la tabella scorre di altrettante righe, una volta sola.
    /// Zero se non si è spostata; zero anche dopo un riordino (ordinamento scelto, fine
    /// della ricerca), che porta in vista il cursore ([`ResultsView::take_scroll_target`]).
    pub fn take_scroll_shift(&mut self) -> usize {
        let Some(pinned) = self.pinned else {
            return 0;
        };
        let Some(now) = self.position(pinned.hit) else {
            self.pinned = None;
            return 0;
        };
        self.pinned = Some(Pinned {
            position: now,
            ..pinned
        });
        now.saturating_sub(pinned.position)
    }

    /// Sposta il cursore di `delta` righe (frecce); con `extend` allarga la selezione
    /// dall'ancora come Maiusc+clic. La riga raggiunta va in vista e prende il focus. Senza
    /// righe non fa nulla.
    pub fn move_cursor(&mut self, delta: isize, extend: bool) {
        let Some(last) = self.order.len().checked_sub(1) else {
            return;
        };
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
        self.click_at(target, kind);
        self.scroll_pending = true;
    }

    /// Il risultato con il cursore della tastiera (l'ultimo cliccato o raggiunto con le
    /// frecce): è quello che "Apri" apre e l'anteprima mostra.
    pub fn cursor_hit(&self) -> Option<usize> {
        self.cursor
    }

    /// Porta cursore e selezione sulla riga in posizione `position` e la tabella in vista
    /// (precedente/successiva dell'anteprima), senza togliere il focus a chi ce l'ha. Fuori
    /// dai limiti non fa nulla.
    pub fn jump_to(&mut self, position: usize) {
        if self.place_cursor(position, ClickKind::Replace) {
            self.scroll_pending = true;
        }
    }

    /// La posizione a cui porta `step` partendo dal cursore, nell'ordine mostrato; `None`
    /// se non c'è un cursore o se non c'è una riga in quella direzione.
    ///
    /// - [`Step::NextInFile`]/[`Step::PreviousInFile`]: la riga dopo/prima dello stesso
    ///   documento;
    /// - [`Step::NextFile`]: la prima riga dopo il cursore di un altro documento;
    /// - [`Step::PreviousFile`]: l'inizio del gruppo di righe consecutive del documento che
    ///   precede quello del cursore.
    ///
    /// Tempo costante.
    pub fn step_target(&self, step: Step) -> Option<usize> {
        let current = self.cursor_position()?;
        match step {
            Step::NextInFile => self.groups.next_same.get(current).copied().flatten(),
            Step::PreviousInFile => self.groups.previous_same.get(current).copied().flatten(),
            Step::NextFile => {
                let end = self.groups.group_end(current, self.order.len())?;
                (end < self.order.len()).then_some(end)
            }
            Step::PreviousFile => {
                let group = *self.groups.group_of.get(current)?;
                let previous = group.checked_sub(1)?;
                self.groups.group_start.get(previous).copied()
            }
        }
    }

    /// Posizione mostrata del cursore.
    pub fn cursor_position(&self) -> Option<usize> {
        self.position(self.cursor?)
    }

    /// La posizione da portare in vista, una sola volta dopo un movimento del cursore.
    pub fn take_scroll_target(&mut self) -> Option<usize> {
        if std::mem::take(&mut self.scroll_pending) {
            self.cursor_position()
        } else {
            None
        }
    }

    /// Il risultato la cui riga deve prendere il focus della tastiera, se c'è una richiesta
    /// in sospeso (resta finché la tabella non disegna la riga: [`ResultsView::focus_done`]).
    pub fn focus_request(&self) -> Option<usize> {
        self.cursor.filter(|_| self.focus_pending)
    }

    /// La riga ha preso il focus: la richiesta è soddisfatta.
    pub fn focus_done(&mut self) {
        self.focus_pending = false;
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

    /// Come un clic sulla riga in posizione `position`, più la richiesta del focus.
    fn click_at(&mut self, position: usize, kind: ClickKind) {
        if self.place_cursor(position, kind) {
            self.focus_pending = true;
        }
    }

    /// Ferma sullo schermo la riga del cursore se era tra quelle disegnate, altrimenti la
    /// prima disegnata; nulla prima del primo disegno o dopo un riordino.
    fn pin(&mut self) {
        let Some((first, last)) = self.visible else {
            self.pinned = None;
            return;
        };
        let position = self
            .cursor_position()
            .filter(|position| (first..=last).contains(position))
            .unwrap_or(first);
        self.pinned = self.hit_at(position).map(|hit| Pinned { hit, position });
    }

    /// Dopo un riordino le posizioni disegnate non valgono più: niente riga ferma finché la
    /// tabella non ridisegna.
    fn forget_visible_rows(&mut self) {
        self.visible = None;
        self.pinned = None;
    }

    /// Cambia selezione e cursore come un clic su `position`; falso fuori dai limiti.
    fn place_cursor(&mut self, position: usize, kind: ClickKind) -> bool {
        let Some(hit) = self.hit_at(position) else {
            return false;
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
        self.pin();
        true
    }

    /// La posizione mostrata del risultato `hit`.
    fn position(&self, hit: usize) -> Option<usize> {
        self.position_of
            .get(hit)
            .copied()
            .filter(|&position| self.order.get(position) == Some(&hit))
    }

    /// Ricalcola posizioni e gruppi dopo un cambio dell'ordine (tempo lineare).
    fn reindex(&mut self) {
        for (position, &hit) in self.order.iter().enumerate() {
            if let Some(slot) = self.position_of.get_mut(hit) {
                *slot = position;
            }
        }
        self.groups.rebuild(&self.order, &self.docs);
    }

    fn select_range_to(&mut self, position: usize) {
        let anchor_position = self
            .anchor
            .and_then(|anchor| self.position(anchor))
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

/// I documenti distinti dei risultati, nell'ordine in cui compaiono.
///
/// Due risultati sono dello stesso documento se i loro [`DocPath`] sono uguali (di solito
/// condividono anche lo stesso `Arc`). Ogni file si legge da una sola radice: la chiave di
/// ordinamento del documento usa la radice del suo primo risultato.
#[derive(Debug, Clone, Default)]
struct DocIndex {
    /// Indice del risultato → indice del documento.
    doc_of_hit: Vec<usize>,
    /// Documento (per valore) → indice.
    index_of: HashMap<Arc<DocPath>, usize>,
    /// Indirizzo dell'`Arc` → indice: evita l'hash del percorso per le righe che
    /// condividono l'`Arc` del loro file. L'`Arc` conservato tiene valido l'indirizzo.
    by_pointer: HashMap<usize, (Arc<DocPath>, usize)>,
    /// Per documento: radice, poi percorso relativo in minuscolo (chiave di File e
    /// Posizione), calcolata una volta per documento.
    keys: Vec<(usize, String)>,
}

impl DocIndex {
    /// Registra i documenti dei risultati non ancora visti.
    fn extend(&mut self, hits: &[Hit], roots: &[PathBuf]) {
        let first = self.doc_of_hit.len();
        for (index, hit) in hits.iter().enumerate().skip(first) {
            // Le righe di un file arrivano di solito insieme e con lo stesso `Arc`: il
            // confronto per puntatore evita di calcolare l'hash del percorso.
            let same_as_previous = index
                .checked_sub(1)
                .and_then(|previous| Some((hits.get(previous)?, self.of(previous)?)))
                .filter(|(previous, _)| Arc::ptr_eq(&previous.doc, &hit.doc))
                .map(|(_, doc)| doc);
            let address = Arc::as_ptr(&hit.doc).addr();
            let known = same_as_previous
                .or_else(|| self.by_pointer.get(&address).map(|(_, doc)| *doc))
                .or_else(|| self.index_of.get(&hit.doc).copied());
            let doc = match known {
                Some(doc) => doc,
                None => {
                    let doc = self.keys.len();
                    self.index_of.insert(Arc::clone(&hit.doc), doc);
                    self.keys.push(file_key(hit, roots));
                    doc
                }
            };
            self.by_pointer
                .entry(address)
                .or_insert_with(|| (Arc::clone(&hit.doc), doc));
            self.doc_of_hit.push(doc);
        }
    }

    /// Il documento del risultato `hit`.
    fn of(&self, hit: usize) -> Option<usize> {
        self.doc_of_hit.get(hit).copied()
    }

    /// La chiave di ordinamento del file del risultato `hit`.
    fn key(&self, hit: usize) -> Option<&(usize, String)> {
        self.keys.get(self.of(hit)?)
    }

    /// Per ogni documento, il suo posto nell'ordine dei file (uguale per chiavi uguali).
    fn ranks(&self) -> Vec<usize> {
        let mut sorted: Vec<usize> = (0..self.keys.len()).collect();
        sorted.sort_by(|&a, &b| self.keys[a].cmp(&self.keys[b]));
        let mut rank_of = vec![0; self.keys.len()];
        let mut rank = 0;
        for (index, &doc) in sorted.iter().enumerate() {
            if index > 0 && self.keys[sorted[index - 1]] != self.keys[doc] {
                rank += 1;
            }
            rank_of[doc] = rank;
        }
        rank_of
    }
}

/// Per ogni posizione mostrata: il gruppo di righe consecutive dello stesso documento e le
/// righe precedente e successiva dello stesso documento (anche non consecutive).
#[derive(Debug, Clone, Default)]
struct GroupIndex {
    /// Posizione → gruppo.
    group_of: Vec<usize>,
    /// Gruppo → prima posizione.
    group_start: Vec<usize>,
    /// Posizione → posizione precedente dello stesso documento.
    previous_same: Vec<Option<usize>>,
    /// Posizione → posizione successiva dello stesso documento.
    next_same: Vec<Option<usize>>,
    /// Documento → ultima posizione indicizzata.
    last_of_doc: Vec<Option<usize>>,
}

impl GroupIndex {
    /// Indicizza da capo l'ordine `order`.
    fn rebuild(&mut self, order: &[usize], docs: &DocIndex) {
        self.group_of.clear();
        self.group_start.clear();
        self.previous_same.clear();
        self.next_same.clear();
        self.last_of_doc.clear();
        self.append(order, 0, docs);
    }

    /// Indicizza le posizioni da `first` in poi, aggiunte in coda a `order`.
    fn append(&mut self, order: &[usize], first: usize, docs: &DocIndex) {
        self.last_of_doc.resize(docs.keys.len(), None);
        for (position, &hit) in order.iter().enumerate().skip(first) {
            let doc = docs.of(hit);
            let previous_doc = position
                .checked_sub(1)
                .and_then(|p| order.get(p))
                .and_then(|&h| docs.of(h));
            let group = match self.group_of.last() {
                Some(&group) if previous_doc == doc => group,
                _ => {
                    self.group_start.push(position);
                    self.group_start.len() - 1
                }
            };
            self.group_of.push(group);
            let previous_same = doc.and_then(|d| self.last_of_doc.get(d).copied().flatten());
            if let Some(slot) = previous_same.and_then(|p| self.next_same.get_mut(p)) {
                *slot = Some(position);
            }
            self.previous_same.push(previous_same);
            self.next_same.push(None);
            if let Some(slot) = doc.and_then(|d| self.last_of_doc.get_mut(d)) {
                *slot = Some(position);
            }
        }
    }

    /// La posizione dopo l'ultima riga del gruppo di `position`.
    fn group_end(&self, position: usize, len: usize) -> Option<usize> {
        let group = *self.group_of.get(position)?;
        Some(self.group_start.get(group + 1).copied().unwrap_or(len))
    }
}

/// Il testo in minuscolo, carattere per carattere: la stessa trasformazione nelle chiavi
/// dell'ordinamento completo e nei confronti dell'inserimento ordinato.
fn folded(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars().flat_map(char::to_lowercase)
}

/// Il primo termine che corrisponde (per la colonna Termine).
fn first_term(hit: &Hit) -> usize {
    hit.terms.iter().next().unwrap_or(usize::MAX)
}

/// Confronta i risultati `a` e `b` secondo `spec`: la stessa relazione di
/// [`sort_order`], senza allocare.
fn compare(spec: SortSpec, hits: &[Hit], docs: &DocIndex, a: usize, b: usize) -> Ordering {
    let (Some(first), Some(second)) = (hits.get(a), hits.get(b)) else {
        return hits.get(a).is_some().cmp(&hits.get(b).is_some());
    };
    let by_file = || docs.key(a).cmp(&docs.key(b));
    let ordering = match spec.column {
        SortColumn::Word => folded(&first.word).cmp(folded(&second.word)),
        SortColumn::Term => first_term(first).cmp(&first_term(second)),
        SortColumn::Context => folded(&first.snippet.text).cmp(folded(&second.snippet.text)),
        SortColumn::File => by_file().then_with(|| first.location.cmp(&second.location)),
        SortColumn::Position => first.location.cmp(&second.location).then_with(by_file),
    };
    match spec.order {
        SortOrder::Ascending => ordering,
        SortOrder::Descending => ordering.reverse(),
    }
}

/// Inserisce i risultati `incoming` in `order`, già ordinato secondo `spec`: ognuno dopo
/// le righe uguali già presenti, così a parità resta l'ordine d'arrivo.
///
/// Costo: ordinamento dei nuovi (con le chiavi in cache se sono le prime righe, quindi
/// tante), una ricerca binaria per ognuno, una copia di `order`.
fn insert_sorted(
    order: &mut Vec<usize>,
    incoming: std::ops::Range<usize>,
    spec: SortSpec,
    hits: &[Hit],
    docs: &DocIndex,
) {
    let mut incoming: Vec<usize> = incoming.collect();
    if order.is_empty() {
        sort_order(&mut incoming, spec, hits, docs);
        *order = incoming;
        return;
    }
    let compare = |a: usize, b: usize| compare(spec, hits, docs, a, b);
    incoming.sort_by(|&a, &b| compare(a, b));
    let mut merged = Vec::with_capacity(order.len() + incoming.len());
    let mut rest: &[usize] = order;
    for hit in incoming {
        let at = rest.partition_point(|&existing| compare(existing, hit) != Ordering::Greater);
        let (before, after) = rest.split_at(at);
        merged.extend_from_slice(before);
        merged.push(hit);
        rest = after;
    }
    merged.extend_from_slice(rest);
    *order = merged;
}

/// Ordina la permutazione `order` (stabile: a parità resta l'ordine precedente). È la
/// stessa relazione di [`compare`], con le chiavi calcolate una volta per riga.
///
/// Per File e Posizione la chiave del file si calcola una volta per documento, non per
/// riga: con 200 000 righe l'ordinamento resta sotto i 50 ms (misura in
/// `sort_200k_timing`).
fn sort_order(order: &mut [usize], spec: SortSpec, hits: &[Hit], docs: &DocIndex) {
    let descending = spec.order == SortOrder::Descending;
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
        SortColumn::Word => by_key!(|hit: &Hit| folded(&hit.word).collect::<String>()),
        SortColumn::Term => by_key!(first_term),
        SortColumn::Context => by_key!(|hit: &Hit| folded(&hit.snippet.text).collect::<String>()),
        SortColumn::File | SortColumn::Position => {
            let ranks = docs.ranks();
            let file = |i: usize| docs.of(i).and_then(|doc| ranks.get(doc).copied());
            let location = |i: usize| hits.get(i).map(|hit| &hit.location);
            let file_first = spec.column == SortColumn::File;
            order.sort_by(|&a, &b| {
                let by_file = file(a).cmp(&file(b));
                let by_location = location(a).cmp(&location(b));
                let ordering = if file_first {
                    by_file.then(by_location)
                } else {
                    by_location.then(by_file)
                };
                if descending {
                    ordering.reverse()
                } else {
                    ordering
                }
            });
        }
    }
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
        view.extend_to(&hits, &roots());
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

    /// Clic sulla riga in posizione `position`, come la tabella: sul risultato che mostra
    /// (nessuno fuori dai limiti).
    fn click_row(view: &mut ResultsView, position: usize, kind: ClickKind) {
        let hit = view.hit_at(position).unwrap_or(usize::MAX);
        view.click_hit(hit, kind);
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
        click_row(&mut view, 1, ClickKind::Replace);
        assert_eq!(view.selected_count(), 1);
        // Ctrl+clic: aggiunge, poi toglie.
        click_row(&mut view, 3, ClickKind::Toggle);
        assert_eq!(view.selected_count(), 2);
        click_row(&mut view, 3, ClickKind::Toggle);
        assert_eq!(view.selected_count(), 1);
        // Maiusc+clic: dall'ancora (posizione 3, l'ultimo Ctrl+clic) alla 0.
        click_row(&mut view, 0, ClickKind::Range);
        assert_eq!(view.selected_count(), 4);
        // Fuori dai limiti: nulla cambia.
        click_row(&mut view, 99, ClickKind::Replace);
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
        click_row(&mut view, 0, ClickKind::Replace);
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

    #[test]
    fn steps_between_results_and_files() {
        let hits = sample();
        let mut view = ResultsView::new(None);
        view.finish(&hits, &roots());
        // Ordine mostrato: a.txt, a.txt, b.txt, b.txt (ogni riga con un suo `Arc`).
        assert_eq!(view.step_target(Step::NextInFile), None, "senza cursore");

        view.jump_to(0);
        assert_eq!(view.cursor_hit(), view.hit_at(0));
        assert_eq!(view.selected_count(), 1);
        assert_eq!(view.take_scroll_target(), Some(0));
        assert_eq!(view.step_target(Step::NextInFile), Some(1));
        assert_eq!(view.step_target(Step::PreviousInFile), None);
        assert_eq!(view.step_target(Step::NextFile), Some(2));
        assert_eq!(view.step_target(Step::PreviousFile), None);

        view.jump_to(3);
        assert_eq!(view.step_target(Step::PreviousInFile), Some(2));
        assert_eq!(view.step_target(Step::NextInFile), None);
        assert_eq!(view.step_target(Step::NextFile), None);
        assert_eq!(view.step_target(Step::PreviousFile), Some(0));

        // Fuori dai limiti: nulla cambia.
        view.jump_to(9);
        assert_eq!(view.cursor_position(), Some(3));
    }

    #[test]
    fn steps_follow_the_displayed_order_when_files_are_mixed() {
        let hits = sample();
        let mut view = ResultsView::new(None);
        // Per parola: Aragosta (a), carta (b), chitarra (b), mare (a).
        view.set_sort(SortSpec::ascending(SortColumn::Word), &hits, &roots());
        view.jump_to(0);
        assert_eq!(view.step_target(Step::NextInFile), Some(3));
        assert_eq!(view.step_target(Step::NextFile), Some(1));
        view.jump_to(3);
        assert_eq!(view.step_target(Step::PreviousFile), Some(1));
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

    /// Il clic e le frecce portano il focus sulla riga del cursore; i passi
    /// dell'anteprima no (il focus resta al bottone premuto).
    #[test]
    fn focus_follows_clicks_and_arrows_not_steps() {
        let hits = sample();
        let mut view = ResultsView::new(None);
        view.finish(&hits, &roots());
        view.jump_to(1);
        assert_eq!(view.focus_request(), None);
        click_row(&mut view, 2, ClickKind::Replace);
        assert_eq!(view.focus_request(), view.hit_at(2));
        view.focus_done();
        assert_eq!(view.focus_request(), None);
        view.move_cursor(1, false);
        assert_eq!(view.focus_request(), view.hit_at(3));
    }

    /// Il passo calcolato scorrendo le righe, come faceva la prima versione: l'oracolo per
    /// gli indici.
    fn naive_step(view: &ResultsView, hits: &[Hit], step: Step) -> Option<usize> {
        let current = view.cursor_position()?;
        let doc_at =
            |position: usize| -> Option<&DocPath> { Some(&hits.get(view.hit_at(position)?)?.doc) };
        let here = doc_at(current)?;
        let same = |position: usize| doc_at(position) == Some(here);
        let mut after = current + 1..view.len();
        match step {
            Step::NextInFile => after.find(|&p| same(p)),
            Step::PreviousInFile => (0..current).rev().find(|&p| same(p)),
            Step::NextFile => after.find(|&p| !same(p)),
            Step::PreviousFile => {
                let end = (0..current).rev().find(|&p| !same(p))?;
                let target = doc_at(end)?;
                let mut start = end;
                while start > 0 && doc_at(start - 1) == Some(target) {
                    start -= 1;
                }
                Some(start)
            }
        }
    }

    /// Righe generate: poche parole e pochi file, così ci sono molte parità. Ogni riga ha
    /// il suo `Arc`, come quando lo stesso file arriva da due lotti diversi.
    fn generated(rows: &[(u8, u8, u8)]) -> Vec<Hit> {
        rows.iter()
            .map(|&(file, line, word)| {
                let words = ["ar", "Ar", "carta", "Città", "città", "mare"];
                let word = words[usize::from(word) % words.len()];
                hit(&format!("f{}.txt", file % 5), u32::from(line % 7), word)
            })
            .collect()
    }

    const ALL_SPECS: [SortSpec; 10] = {
        let columns = [
            SortColumn::Word,
            SortColumn::Term,
            SortColumn::File,
            SortColumn::Position,
            SortColumn::Context,
        ];
        let mut specs = [SortSpec::ascending(SortColumn::Word); 10];
        let mut i = 0;
        while i < 5 {
            specs[2 * i] = SortSpec {
                column: columns[i],
                order: SortOrder::Ascending,
            };
            specs[2 * i + 1] = SortSpec {
                column: columns[i],
                order: SortOrder::Descending,
            };
            i += 1;
        }
        specs
    };

    proptest::proptest! {
        /// Inserire le righe al loro posto man mano che arrivano dà lo stesso ordine
        /// dell'ordinamento completo a fine ricerca, parità comprese.
        #[test]
        fn incremental_insertion_equals_a_full_sort(
            rows in proptest::collection::vec((0u8..8, 0u8..10, 0u8..6), 0..40),
            cuts in proptest::collection::vec(0usize..40, 0..6),
            spec_index in 0usize..10,
        ) {
            let hits = generated(&rows);
            let spec = ALL_SPECS[spec_index];
            let mut incremental = ResultsView::new(Some(spec));
            let mut cuts: Vec<usize> = cuts.into_iter().map(|c| c.min(hits.len())).collect();
            cuts.sort_unstable();
            for cut in cuts {
                incremental.extend_to(&hits[..cut], &roots());
            }
            incremental.finish(&hits, &roots());
            let mut full = ResultsView::new(None);
            full.extend_to(&hits, &roots());
            full.set_sort(spec, &hits, &roots());
            proptest::prop_assert_eq!(&incremental.order, &full.order);
        }

        /// I passi con gli indici coincidono con quelli calcolati scorrendo le righe, in
        /// ogni ordinamento e da ogni riga.
        #[test]
        fn indexed_steps_equal_the_naive_ones(
            rows in proptest::collection::vec((0u8..8, 0u8..10, 0u8..6), 1..30),
            spec_index in 0usize..11,
            arrival_only in proptest::bool::ANY,
        ) {
            let hits = generated(&rows);
            let mut view = ResultsView::new(None);
            if arrival_only {
                view.extend_to(&hits, &roots());
            } else if let Some(spec) = ALL_SPECS.get(spec_index) {
                view.set_sort(*spec, &hits, &roots());
            } else {
                view.finish(&hits, &roots());
            }
            for position in 0..view.len() {
                view.jump_to(position);
                for step in [Step::NextInFile, Step::PreviousInFile, Step::NextFile, Step::PreviousFile] {
                    proptest::prop_assert_eq!(
                        view.step_target(step),
                        naive_step(&view, &hits, step),
                        "{:?} da {}", step, position
                    );
                }
            }
        }
    }

    /// C-17: con un ordinamento scelto, le righe che arrivano durante la ricerca entrano al
    /// loro posto, non in coda.
    #[test]
    fn rows_arriving_during_a_search_take_their_sorted_place() {
        let hits = vec![
            hit("b.txt", 3, "carta"),
            hit("a.txt", 2, "mare"),
            hit("a.txt", 9, "Aragosta"),
            hit("b.txt", 1, "chitarra"),
        ];
        let mut view = ResultsView::new(None);
        view.extend_to(&hits[..2], &roots());
        view.set_sort(SortSpec::ascending(SortColumn::Word), &hits[..2], &roots());
        assert_eq!(shown(&view, &hits), ["carta", "mare"]);
        view.extend_to(&hits, &roots());
        assert_eq!(
            shown(&view, &hits),
            ["Aragosta", "carta", "chitarra", "mare"]
        );
    }

    /// Righe in arrivo durante una ricerca ordinata per parola: `early` arrivano prima,
    /// `late` dopo.
    fn sorted_by_word(early: &[&str], late: &[&str]) -> (ResultsView, Vec<Hit>) {
        let hits: Vec<Hit> = early
            .iter()
            .chain(late)
            .enumerate()
            .map(|(line, word)| hit("a.txt", u32::try_from(line).unwrap_or(0), word))
            .collect();
        let mut view = ResultsView::new(Some(SortSpec::ascending(SortColumn::Word)));
        view.extend_to(&hits[..early.len()], &roots());
        (view, hits)
    }

    /// R-01: righe inserite sopra la prima riga visibile (nessun cursore in vista): la
    /// tabella scorre di altrettante righe e la prima riga visibile resta la stessa.
    #[test]
    fn rows_inserted_above_the_first_visible_row_keep_it_in_place() {
        let (mut view, hits) = sorted_by_word(&["d", "e", "f", "g", "h"], &["a", "b", "z"]);
        view.set_visible_rows(2, 4);
        let first_visible = view.hit_at(2);
        assert_eq!(view.take_scroll_shift(), 0, "nulla di inserito");
        view.extend_to(&hits, &roots());
        // "a" e "b" entrano sopra, "z" sotto.
        assert_eq!(view.take_scroll_shift(), 2);
        assert_eq!(view.hit_at(4), first_visible);
        assert_eq!(view.take_scroll_shift(), 0, "una volta sola");
    }

    /// R-01: righe inserite sotto la riga ferma non fanno scorrere.
    #[test]
    fn rows_inserted_below_do_not_scroll() {
        let (mut view, hits) = sorted_by_word(&["a", "b", "c"], &["x", "y"]);
        view.set_visible_rows(0, 2);
        view.extend_to(&hits, &roots());
        assert_eq!(view.take_scroll_shift(), 0);
    }

    /// R-01: con la riga del cursore in vista resta ferma lei, anche per le righe inserite
    /// tra la prima visibile e il cursore: la riga selezionata resta in vista e sotto il
    /// puntatore.
    #[test]
    fn the_visible_cursor_row_stays_in_place() {
        let (mut view, hits) = sorted_by_word(&["a", "c", "e", "g"], &["b", "d", "f0"]);
        let g = view.hit_at(3).expect("g");
        view.click_hit(g, ClickKind::Replace);
        view.take_scroll_target();
        view.set_visible_rows(1, 3);
        view.extend_to(&hits, &roots());
        // "b", "d" e "f0" entrano tutte sopra "g".
        assert_eq!(view.take_scroll_shift(), 3);
        assert_eq!(view.cursor_position(), Some(6));
        assert_eq!(view.cursor_hit(), Some(g));

        // Cursore fuori dalle righe visibili: ferma la prima visibile.
        let (mut view, hits) = sorted_by_word(&["a", "c", "e", "g"], &["b", "d"]);
        view.click_hit(view.hit_at(3).expect("g"), ClickKind::Replace);
        view.set_visible_rows(0, 1);
        view.extend_to(&hits, &roots());
        assert_eq!(view.take_scroll_shift(), 0, "\"a\" resta la prima");
    }

    /// R-01: un riordino (ordinamento scelto, fine della ricerca) non fa scorrere della
    /// differenza: porta in vista il cursore.
    #[test]
    fn a_resort_brings_the_cursor_into_view_instead_of_scrolling() {
        let (mut view, hits) = sorted_by_word(&["d", "e", "f"], &["a", "b"]);
        view.click_hit(view.hit_at(2).expect("f"), ClickKind::Replace);
        view.take_scroll_target();
        view.set_visible_rows(0, 2);
        view.extend_to(&hits, &roots());
        view.set_sort(SortSpec::ascending(SortColumn::Position), &hits, &roots());
        assert_eq!(view.take_scroll_shift(), 0);
        assert_eq!(view.take_scroll_target(), view.cursor_position());
    }

    /// R-01: il clic agisce sul risultato identificato, non sulla posizione: dopo un
    /// inserimento sopra, la stessa posizione mostra un altro risultato.
    #[test]
    fn a_click_acts_on_the_identified_result() {
        let (mut view, hits) = sorted_by_word(&["c", "d"], &["a"]);
        let d = view.hit_at(1).expect("d");
        view.extend_to(&hits, &roots());
        assert_ne!(view.hit_at(1), Some(d), "\"d\" si è spostata");
        view.click_hit(d, ClickKind::Replace);
        assert_eq!(view.cursor_hit(), Some(d));
        assert_eq!(view.selected_hits().collect::<Vec<_>>(), [d]);
        assert_eq!(view.focus_request(), Some(d));
        // Un risultato che non c'è non cambia nulla.
        view.click_hit(99, ClickKind::Replace);
        assert_eq!(view.cursor_hit(), Some(d));
    }

    /// R-01: doppio clic dopo un inserimento: si apre il risultato del primo clic; se il
    /// secondo clic cade su un altro risultato, lo seleziona e non apre nulla.
    #[test]
    fn a_double_click_after_an_insertion_opens_the_clicked_result() {
        let (mut view, hits) = sorted_by_word(&["c", "d", "e"], &["a", "b"]);
        let d = view.hit_at(1).expect("d");
        // Come nella finestra: la tabella disegna, poi arriva il primo clic, poi le righe.
        view.set_visible_rows(0, 2);
        view.click_hit(d, ClickKind::Replace);
        view.extend_to(&hits, &roots());
        assert_eq!(
            view.take_scroll_shift(),
            2,
            "\"d\" resta sotto il puntatore"
        );
        assert!(view.double_click_hit(d), "si apre \"d\"");
        assert_eq!(view.cursor_hit(), Some(d));

        let other = view.hit_at(0).expect("a");
        assert!(
            !view.double_click_hit(other),
            "secondo clic su un'altra riga"
        );
        assert_eq!(view.cursor_hit(), Some(other));
        assert_eq!(view.selected_hits().collect::<Vec<_>>(), [other]);
    }

    /// R-02: il focus arrivato con Tab o F6 porta lì il cursore e lascia la selezione.
    #[test]
    fn focus_moves_the_cursor_and_keeps_the_selection() {
        let hits = sample();
        let mut view = ResultsView::new(None);
        view.finish(&hits, &roots());
        view.select_all();
        let second = view.hit_at(1).expect("riga");
        view.focus_hit(second);
        assert_eq!(view.cursor_hit(), Some(second));
        assert_eq!(view.selected_count(), 4);
        assert_eq!(view.focus_request(), None, "il focus ce l'ha già");
        view.focus_hit(99);
        assert_eq!(view.cursor_hit(), Some(second));
    }

    /// C-17: il riordino di fine ricerca riporta in vista la riga del cursore.
    #[test]
    fn finish_brings_the_cursor_row_back_into_view() {
        let hits = sample();
        let mut view = ResultsView::new(None);
        view.extend_to(&hits, &roots());
        click_row(&mut view, 0, ClickKind::Replace);
        view.take_scroll_target();
        view.finish(&hits, &roots());
        assert_eq!(
            view.take_scroll_target(),
            Some(3),
            "\"carta\" è ora l'ultima"
        );
    }

    /// C-08, misura P-07: `cargo test --release -p lynkeus-app --lib step_target_200k --
    /// --ignored --nocapture`. Obiettivo: meno di 1 ms per chiamata.
    #[test]
    #[ignore = "misura di prestazioni, da eseguire in release"]
    fn step_target_200k() {
        let hits = many_hits();
        let steps = [
            Step::NextInFile,
            Step::PreviousInFile,
            Step::NextFile,
            Step::PreviousFile,
        ];
        // Righe per file (ordine predefinito) e sparse (per parola), con il cursore
        // all'inizio, a metà e in fondo: i casi peggiori di una ricerca per scorrimento.
        for sort in [None, Some(SortSpec::ascending(SortColumn::Word))] {
            let mut view = ResultsView::new(sort);
            view.finish(&hits, &roots());
            for position in [0, hits.len() / 2, hits.len() - 1] {
                view.jump_to(position);
                const CALLS: u32 = 50;
                let started = std::time::Instant::now();
                for _ in 0..CALLS {
                    for step in steps {
                        std::hint::black_box(view.step_target(std::hint::black_box(step)));
                    }
                }
                let per_call = started.elapsed() / (CALLS * 4);
                println!("step_target {sort:?} da {position}: {per_call:?} per chiamata");
                assert!(
                    per_call < std::time::Duration::from_millis(1),
                    "{per_call:?}"
                );
            }
        }
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

            // Durante la ricerca, con l'ordinamento scelto: righe inserite al loro posto a
            // lotti come quelli di un frame (`MAX_HITS_PER_FRAME`).
            let mut streaming = ResultsView::new(Some(SortSpec::ascending(column)));
            let mut slowest = std::time::Duration::ZERO;
            for end in (5_000..=hits.len()).step_by(5_000) {
                let batch = std::time::Instant::now();
                streaming.extend_to(&hits[..end], &roots());
                slowest = slowest.max(batch.elapsed());
            }
            println!("{column:?}, a lotti di 5000: al massimo {slowest:?} per lotto");
            assert!(
                slowest < std::time::Duration::from_millis(50),
                "{column:?}: {slowest:?}"
            );
        }
    }
}
