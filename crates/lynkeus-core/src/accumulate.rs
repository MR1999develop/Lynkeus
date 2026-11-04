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

    /// Chiude il file: i risultati nell'ordine di arrivo, o nessuno se con AND manca un termine.
    pub fn finish(self) -> Vec<T> {
        match self.combine {
            Combine::Any => self.items,
            Combine::All if self.seen.contains_all(self.required) => self.items,
            Combine::All => Vec::new(),
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
    fn empty_file_gives_nothing() {
        let acc: FileAccumulator<&str> = FileAccumulator::new(1, Combine::All);
        assert!(acc.finish().is_empty());
    }
}
