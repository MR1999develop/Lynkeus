// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! OR e AND sui risultati di un file.

use crate::matcher::TermSet;
use crate::query::Combine;

/// Raccoglie i risultati di un file e decide alla fine quali tenere.
///
/// Con [`Combine::Any`] tiene tutto; con [`Combine::All`] tiene tutto solo se, nell'insieme
/// dei risultati del file, compaiono tutti i termini, altrimenti nulla. Un file dentro un
/// archivio è un file a sé: serve un accumulatore per ciascuno.
///
/// Uso con file letti a blocchi: dopo ogni blocco [`FileAccumulator::take_ready`] consegna
/// i risultati già sicuri (sempre con OR; con AND dal momento in cui tutti i termini sono
/// comparsi), così la memoria trattenuta resta limitata; [`FileAccumulator::finish`] chiude
/// il file con quelli rimasti. L'ordine complessivo resta quello di arrivo.
#[derive(Debug, Clone)]
pub struct FileAccumulator<T> {
    required: TermSet,
    combine: Combine,
    seen: TermSet,
    items: Vec<T>,
}

impl<T> FileAccumulator<T> {
    /// Accumulatore per una query di `term_count` termini.
    pub fn new(term_count: usize, combine: Combine) -> Self {
        FileAccumulator {
            required: TermSet::all(term_count),
            combine,
            seen: TermSet::EMPTY,
            items: Vec::new(),
        }
    }

    /// Aggiunge un risultato con i termini che vi corrispondono.
    pub fn push(&mut self, terms: TermSet, item: T) {
        self.seen = self.seen.union(terms);
        self.items.push(item);
    }

    /// Termini visti finora.
    pub fn seen(&self) -> TermSet {
        self.seen
    }

    /// Risultati trattenuti finora.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Vero se non ha ancora risultati.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Vero se i risultati del file sono già sicuri: sempre con OR, con AND quando tutti i
    /// termini sono comparsi (da lì in poi ogni risultato del file si tiene).
    pub fn is_satisfied(&self) -> bool {
        match self.combine {
            Combine::Any => true,
            Combine::All => self.seen.contains_all(self.required),
        }
    }

    /// Consegna i risultati trattenuti se sono già sicuri ([`FileAccumulator::is_satisfied`]),
    /// nell'ordine di arrivo; altrimenti nessuno, e restano trattenuti.
    pub fn take_ready(&mut self) -> Vec<T> {
        if self.is_satisfied() {
            std::mem::take(&mut self.items)
        } else {
            Vec::new()
        }
    }

    /// Chiude il file: i risultati ancora trattenuti nell'ordine di arrivo, o nessuno se con
    /// AND manca un termine.
    pub fn finish(self) -> Vec<T> {
        if self.is_satisfied() {
            self.items
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_keeps_everything() {
        let mut acc = FileAccumulator::new(2, Combine::Any);
        acc.push(TermSet::single(0), "a");
        assert_eq!(acc.finish(), ["a"]);
    }

    #[test]
    fn all_drops_the_file_when_a_term_is_missing() {
        let mut acc = FileAccumulator::new(2, Combine::All);
        acc.push(TermSet::single(0), "a");
        acc.push(TermSet::single(0), "b");
        assert!(acc.finish().is_empty());
    }

    #[test]
    fn all_keeps_the_file_when_every_term_appears() {
        let mut acc = FileAccumulator::new(2, Combine::All);
        acc.push(TermSet::single(1), "a");
        acc.push(TermSet::single(0), "b");
        assert_eq!(acc.finish(), ["a", "b"]);
    }

    #[test]
    fn single_term_all_behaves_like_any() {
        let mut acc = FileAccumulator::new(1, Combine::All);
        acc.push(TermSet::single(0), "a");
        assert_eq!(acc.finish(), ["a"]);
    }

    #[test]
    fn a_row_with_several_terms_satisfies_all_at_once() {
        let mut acc = FileAccumulator::new(2, Combine::All);
        acc.push(TermSet::all(2), "a");
        assert!(acc.is_satisfied());
        assert_eq!(acc.finish(), ["a"]);
    }

    #[test]
    fn any_streams_every_block() {
        let mut acc = FileAccumulator::new(2, Combine::Any);
        acc.push(TermSet::single(1), "a");
        assert_eq!(acc.take_ready(), ["a"]);
        acc.push(TermSet::single(1), "b");
        assert_eq!(acc.finish(), ["b"]);
    }

    #[test]
    fn all_holds_until_satisfied_then_streams() {
        let mut acc = FileAccumulator::new(2, Combine::All);
        acc.push(TermSet::single(0), "a");
        assert!(acc.take_ready().is_empty());
        assert_eq!(acc.len(), 1);
        acc.push(TermSet::single(1), "b");
        assert_eq!(acc.take_ready(), ["a", "b"]);
        assert!(acc.is_empty());
        acc.push(TermSet::single(0), "c");
        assert_eq!(acc.finish(), ["c"]);
    }

    #[test]
    fn empty_file_gives_nothing() {
        let acc: FileAccumulator<&str> = FileAccumulator::new(1, Combine::All);
        assert!(acc.finish().is_empty());
    }
}
