// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! I risultati di un documento (un file di testo o un documento estratto) dalla ricerca alla
//! consegna: OR/AND, lotti e limite, in un solo posto ([`HitBatcher`]).
//!
//! Regole:
//!
//! - con OR, e con AND da quando ogni termine è comparso nel documento, i risultati sono
//!   sicuri: partono a lotti di [`BATCH_HITS`], o subito se bastano a riempire il limite;
//! - prima (AND) si trattengono; oltre i posti che il limite lascia ancora si tengono solo
//!   quelli che portano un termine nuovo (servono a sapere se il documento li ha tutti);
//! - i risultati trattenuti da tutti i documenti in lettura, in tutti i thread, non superano
//!   [`MAX_HELD_BYTES`] ([`HeldHits`], byte stimati con [`held_bytes`]). Un documento che non
//!   trova posto smette di trattenere e guarda solo quali termini compaiono ("sondaggio"):
//!   se alla fine li ha visti tutti, chi lo legge lo rilegge con un batcher già sicuro
//!   ([`Verdict::Reread`]). Così nessun risultato si perde e la memoria resta limitata,
//!   qualunque sia il limite dei risultati.

use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use lynkeus_core::{
    Combine, CompiledQuery, FileAccumulator, Location, TermSet, WordMatch, snippet, word_text,
};

use crate::doc::DocPath;
use crate::event::{Hit, WarningKind};
use crate::halt::Stop;
use crate::request::Limits;

/// Risultati già sicuri trattenuti al massimo prima di consegnarne un lotto.
pub(crate) const BATCH_HITS: usize = 1024;

/// Byte stimati ([`held_bytes`]) dei risultati non ancora sicuri trattenuti al massimo da
/// tutti i documenti in lettura insieme: con parole e righe brevi (200-300 byte stimati
/// l'uno) da 100.000 a 150.000 risultati.
/// Il picco reale, con l'allocatore e la crescita dei vettori, è circa una volta e mezza la
/// stima (P-03 del piano di test).
pub(crate) const MAX_HELD_BYTES: usize = 32 * 1024 * 1024;

/// Dove vanno i risultati e gli avvisi di un documento.
pub(crate) trait HitSink {
    /// Errore se la ricerca deve fermarsi (annullata, limite raggiunto, chiamante sparito).
    fn check(&self) -> Result<(), Stop>;
    /// Risultati che il limite permette ancora di consegnare.
    fn remaining(&self) -> usize;
    /// Consegna un lotto (mai vuoto) di risultati già filtrati da OR/AND.
    fn deliver(&mut self, hits: Vec<Hit>) -> Result<(), Stop>;
    /// Un documento che non si è potuto leggere.
    fn warn(&mut self, doc: Arc<DocPath>, kind: WarningKind) -> Result<(), Stop>;
}

/// I risultati trattenuti (non ancora sicuri) da tutti i documenti in lettura, con un tetto
/// comune in byte stimati.
#[derive(Debug)]
pub(crate) struct HeldHits {
    held: AtomicUsize,
    max_bytes: usize,
}

impl HeldHits {
    /// Al più `max_bytes` byte stimati ([`held_bytes`]) trattenuti in tutto.
    pub(crate) fn new(max_bytes: usize) -> HeldHits {
        HeldHits {
            held: AtomicUsize::new(0),
            max_bytes,
        }
    }

    /// Prenota il posto di `bytes` byte; falso se non c'è.
    fn reserve(&self, bytes: usize) -> bool {
        self.held
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
                held.checked_add(bytes)
                    .filter(|&total| total <= self.max_bytes)
            })
            .is_ok()
    }

    /// Libera `bytes` byte prenotati.
    fn release(&self, bytes: usize) {
        if bytes > 0 {
            self.held.fetch_sub(bytes, Ordering::Relaxed);
        }
    }
}

/// I byte che un risultato trattenuto occupa, stimati: la struttura più la capacità dei suoi
/// testi e delle evidenziazioni (il documento è condiviso tra le righe e non conta).
pub(crate) fn held_bytes(hit: &Hit) -> usize {
    let sheet = match &hit.location {
        Location::Cell { sheet, .. } => sheet.capacity(),
        _ => 0,
    };
    let highlights = hit
        .snippet
        .highlights
        .capacity()
        .saturating_mul(size_of::<Range<usize>>());
    size_of::<Hit>()
        .saturating_add(hit.word.capacity())
        .saturating_add(hit.snippet.text.capacity())
        .saturating_add(highlights)
        .saturating_add(sheet)
}

/// Cosa cercare e come etichettare i risultati di un documento.
pub(crate) struct HitLabel<'a> {
    pub query: &'a CompiledQuery,
    pub limits: &'a Limits,
    pub root: usize,
    pub doc: Arc<DocPath>,
}

impl HitLabel<'_> {
    /// La riga del risultato `found` nel testo `text`, in `location`.
    pub(crate) fn hit(&self, text: &str, found: &WordMatch, location: Location) -> Hit {
        let word = word_text(text, found, self.limits.word_max_chars);
        Hit {
            doc: Arc::clone(&self.doc),
            root: self.root,
            location,
            word: word.text,
            word_truncated_start: word.truncated_start,
            word_truncated_end: word.truncated_end,
            snippet: snippet(text, found, self.limits.context_chars),
            terms: found.terms,
        }
    }
}

/// Com'è finito un documento.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// I risultati rimasti (forse nessuno), da consegnare.
    Hits(Vec<Hit>),
    /// Il documento ha tutti i termini ma i suoi risultati non sono stati trattenuti: va
    /// riletto con [`HitBatcher::satisfied`].
    Reread,
}

/// Lo stato dei risultati di un documento.
enum State {
    /// Si raccolgono i risultati; `reserved` byte presi da [`HeldHits`].
    Collecting {
        accumulator: FileAccumulator<Hit>,
        reserved: usize,
    },
    /// Nessun posto: si guardano solo i termini comparsi.
    Probing { seen: TermSet },
}

/// I risultati di un documento verso la consegna (vedi il modulo).
pub(crate) struct HitBatcher<'h> {
    held: &'h HeldHits,
    required: TermSet,
    combine: Combine,
    state: State,
}

impl<'h> HitBatcher<'h> {
    /// Un batcher per un documento nuovo della query `query`.
    pub(crate) fn new(query: &CompiledQuery, held: &'h HeldHits) -> HitBatcher<'h> {
        HitBatcher::with(query.term_count(), query.combine(), held)
    }

    /// Un batcher per la seconda lettura di un documento che ha già tutti i termini: ogni
    /// risultato è sicuro.
    pub(crate) fn satisfied(query: &CompiledQuery, held: &'h HeldHits) -> HitBatcher<'h> {
        HitBatcher::with(query.term_count(), Combine::Any, held)
    }

    fn with(term_count: usize, combine: Combine, held: &'h HeldHits) -> HitBatcher<'h> {
        HitBatcher {
            held,
            required: TermSet::all(term_count),
            combine,
            state: State::Collecting {
                accumulator: FileAccumulator::new(term_count, combine),
                reserved: 0,
            },
        }
    }

    /// Aggiunge un risultato con i termini `terms`, costruito con `make` solo se serve;
    /// `room` = posti che il limite lascia ancora. Restituisce un lotto da consegnare subito,
    /// se è pronto.
    pub(crate) fn push(
        &mut self,
        terms: TermSet,
        room: usize,
        make: impl FnOnce() -> Hit,
    ) -> Option<Vec<Hit>> {
        let State::Collecting {
            accumulator,
            reserved,
        } = &mut self.state
        else {
            if let State::Probing { seen } = &mut self.state {
                *seen = seen.union(terms);
            }
            return None;
        };
        // Oltre il limite servono solo i risultati che portano un termine nuovo.
        if accumulator.len() >= room && accumulator.seen().contains_all(terms) {
            return None;
        }
        let hit = make();
        if !accumulator.is_satisfied() {
            let bytes = held_bytes(&hit);
            if !self.held.reserve(bytes) {
                let seen = accumulator.seen().union(terms);
                self.start_probing(seen);
                return None;
            }
            *reserved = reserved.saturating_add(bytes);
        }
        accumulator.push(terms, hit);
        let len = accumulator.len();
        (accumulator.is_satisfied() && (len >= BATCH_HITS || len >= room))
            .then(|| self.take_ready())
    }

    /// Vero se il documento sta solo sondando i termini e li ha già visti tutti: il resto
    /// del documento non cambia nulla, si può passare alla seconda lettura.
    pub(crate) fn is_settled(&self) -> bool {
        matches!(self.state, State::Probing { seen } if self.is_complete(seen))
    }

    /// Chiude il documento.
    pub(crate) fn finish(mut self) -> Verdict {
        match std::mem::replace(
            &mut self.state,
            State::Probing {
                seen: TermSet::EMPTY,
            },
        ) {
            State::Collecting {
                accumulator,
                reserved,
            } => {
                self.held.release(reserved);
                Verdict::Hits(accumulator.finish())
            }
            State::Probing { seen } if self.is_complete(seen) => Verdict::Reread,
            State::Probing { .. } => Verdict::Hits(Vec::new()),
        }
    }

    fn is_complete(&self, seen: TermSet) -> bool {
        match self.combine {
            Combine::Any => true,
            Combine::All => seen.contains_all(self.required),
        }
    }

    /// I risultati già sicuri, liberando i posti che occupavano.
    fn take_ready(&mut self) -> Vec<Hit> {
        match &mut self.state {
            State::Collecting {
                accumulator,
                reserved,
            } => {
                let hits = accumulator.take_ready();
                self.held.release(std::mem::take(reserved));
                hits
            }
            State::Probing { .. } => Vec::new(),
        }
    }

    /// Smette di trattenere: i posti tornano liberi, i risultati si scartano.
    fn start_probing(&mut self, seen: TermSet) {
        if let State::Collecting { reserved, .. } = &mut self.state {
            self.held.release(std::mem::take(reserved));
        }
        self.state = State::Probing { seen };
    }
}

impl Drop for HitBatcher<'_> {
    /// Un documento interrotto (arresto, errore) libera i posti che occupava.
    fn drop(&mut self) {
        if let State::Collecting { reserved, .. } = &mut self.state {
            self.held.release(std::mem::take(reserved));
        }
    }
}

#[cfg(test)]
mod tests {
    use lynkeus_core::{MatchOptions, Query, Term, WildcardMode};

    use super::*;

    fn query(terms: &[&str], combine: Combine) -> CompiledQuery {
        let terms = terms
            .iter()
            .map(|t| Term::parse(t, WildcardMode::Enabled).expect("termine valido"))
            .collect();
        Query::new(terms, MatchOptions::default(), combine)
            .and_then(|q| q.compile())
            .expect("query valida")
    }

    fn hit(n: usize) -> Hit {
        let limits = Limits::default();
        let compiled = query(&["ar"], Combine::Any);
        let label = HitLabel {
            query: &compiled,
            limits: &limits,
            root: 0,
            doc: Arc::new(DocPath::file(format!("f{n}.txt"))),
        };
        let found = lynkeus_core::find_words(&compiled, "carta")
            .pop()
            .expect("carta");
        label.hit("carta", &found, Location::Line(1))
    }

    /// Byte trattenuti ora.
    fn current(held: &HeldHits) -> usize {
        held.held.load(Ordering::Relaxed)
    }

    /// I byte che trattiene un risultato di [`hit`] (tutti uguali: il documento è condiviso
    /// e non conta).
    fn one() -> usize {
        held_bytes(&hit(0))
    }

    /// Posto per `count` risultati di [`hit`].
    fn places(count: usize) -> HeldHits {
        HeldHits::new(count * one())
    }

    /// Quanti risultati di [`hit`] sono trattenuti ora.
    fn held_count(held: &HeldHits) -> usize {
        current(held) / one()
    }

    /// Un risultato con parola (150 caratteri, 30 corrispondenze) e contesto lunghi.
    fn long_hit() -> Hit {
        let limits = Limits::default();
        let compiled = query(&["ar"], Combine::Any);
        let label = HitLabel {
            query: &compiled,
            limits: &limits,
            root: 0,
            doc: Arc::new(DocPath::file("lungo.txt")),
        };
        let text = format!("{} {}", "x".repeat(300), "carta".repeat(30));
        let found = lynkeus_core::find_words(&compiled, &text)
            .pop()
            .expect("carta");
        label.hit(&text, &found, Location::Line(1))
    }

    const AR: usize = 0;
    const ZETA: usize = 1;

    /// Due documenti AND che non hanno ancora tutti i termini, in due thread: insieme non
    /// trattengono più del tetto comune. Quello che non trova posto sonda i termini e, se
    /// alla fine li ha tutti, chiede la seconda lettura; l'altro consegna normalmente.
    #[test]
    fn held_hits_are_bounded_across_documents() {
        let compiled = query(&["ar", "zeta"], Combine::All);
        let held = places(100);
        let mut first = HitBatcher::new(&compiled, &held);
        let mut second = HitBatcher::new(&compiled, &held);
        for n in 0..80 {
            assert_eq!(first.push(TermSet::single(AR), usize::MAX, || hit(n)), None);
        }
        for n in 0..80 {
            assert_eq!(
                second.push(TermSet::single(AR), usize::MAX, || hit(n)),
                None
            );
            assert!(held_count(&held) <= 100, "{}", current(&held));
        }
        assert!(!second.is_settled());
        assert_eq!(
            second.push(TermSet::single(ZETA), usize::MAX, || hit(0)),
            None
        );
        assert!(second.is_settled());
        assert_eq!(second.finish(), Verdict::Reread);
        // Il primo aveva trovato posto: con l'ultimo termine consegna tutto.
        let batch = first.push(TermSet::single(ZETA), 1000, || hit(0));
        assert_eq!(
            batch.map(|b| b.len()),
            None,
            "lotto sotto BATCH_HITS: alla fine"
        );
        assert_eq!(first.finish().hits_len(), 81);
        assert_eq!(current(&held), 0, "posti non liberati");
    }

    /// Il tetto vale tra i thread: otto documenti AND letti insieme, con 40 risultati non
    /// sicuri ciascuno, non trattengono mai più di 100 risultati in tutto. Dopo l'ultimo
    /// termine ognuno consegna tutti i suoi 41 risultati o chiede la seconda lettura; al più
    /// due hanno trovato posto per tutti; alla fine i posti sono liberi.
    #[test]
    fn held_hits_are_bounded_across_threads() {
        const THREADS: usize = 8;
        const PLACES: usize = 100;
        let compiled = query(&["ar", "zeta"], Combine::All);
        let held = places(PLACES);
        // Tutti trattengono prima che qualcuno finisca: la concorrenza non dipende dal caso.
        let all_held = std::sync::Barrier::new(THREADS);
        let verdicts: Vec<Verdict> = std::thread::scope(|scope| {
            let readers: Vec<_> = (0..THREADS)
                .map(|_| {
                    scope.spawn(|| {
                        let mut batcher = HitBatcher::new(&compiled, &held);
                        for n in 0..40 {
                            let batch = batcher.push(TermSet::single(AR), usize::MAX, || hit(n));
                            assert_eq!(batch, None);
                            assert!(held_count(&held) <= PLACES, "{}", current(&held));
                        }
                        all_held.wait();
                        let batch = batcher.push(TermSet::single(ZETA), usize::MAX, || hit(0));
                        assert_eq!(batch, None, "lotto sotto BATCH_HITS: alla fine");
                        batcher.finish()
                    })
                })
                .collect();
            readers
                .into_iter()
                .map(|reader| reader.join().expect("thread del lettore"))
                .collect()
        });
        let delivered: Vec<usize> = verdicts
            .iter()
            .filter(|v| **v != Verdict::Reread)
            .map(Verdict::hits_len)
            .collect();
        assert!(delivered.len() <= 2, "{delivered:?}");
        assert!(delivered.iter().all(|&len| len == 41), "{delivered:?}");
        assert_eq!(current(&held), 0, "posti non liberati");
    }

    /// Il tetto è in byte: un risultato con parola e contesto lunghi occupa più posto di uno
    /// breve. Nel posto di due lunghi il terzo non sta (il documento smette di trattenere e
    /// poi chiede la seconda lettura), ma stanno sei brevi.
    #[test]
    fn held_places_are_counted_in_bytes() {
        let long_bytes = held_bytes(&long_hit());
        assert!(long_bytes > 3 * one(), "{long_bytes} contro {}", one());
        let compiled = query(&["ar", "zeta"], Combine::All);
        let held = HeldHits::new(2 * long_bytes);
        let mut batcher = HitBatcher::new(&compiled, &held);
        for _ in 0..2 {
            let batch = batcher.push(TermSet::single(AR), usize::MAX, long_hit);
            assert_eq!(batch, None);
        }
        assert_eq!(current(&held), 2 * long_bytes);
        let batch = batcher.push(TermSet::single(AR), usize::MAX, long_hit);
        assert_eq!(batch, None);
        assert_eq!(
            current(&held),
            0,
            "il documento che non trova posto libera i suoi"
        );
        let batch = batcher.push(TermSet::single(ZETA), usize::MAX, || hit(0));
        assert_eq!(batch, None);
        assert_eq!(batcher.finish(), Verdict::Reread);
        let mut short = HitBatcher::new(&compiled, &held);
        for n in 0..6 {
            assert_eq!(short.push(TermSet::single(AR), usize::MAX, || hit(n)), None);
        }
        assert_eq!(held_count(&held), 6);
    }

    /// Un documento senza tutti i termini non consegna nulla e libera i posti, anche se
    /// interrotto.
    #[test]
    fn unsatisfied_documents_release_their_places() {
        let compiled = query(&["ar", "zeta"], Combine::All);
        let held = places(10);
        let mut probing = HitBatcher::new(&compiled, &held);
        for n in 0..20 {
            assert_eq!(
                probing.push(TermSet::single(AR), usize::MAX, || hit(n)),
                None
            );
        }
        assert_eq!(probing.finish(), Verdict::Hits(Vec::new()));
        let mut interrupted = HitBatcher::new(&compiled, &held);
        for n in 0..5 {
            assert_eq!(
                interrupted.push(TermSet::single(AR), usize::MAX, || hit(n)),
                None
            );
        }
        assert_eq!(held_count(&held), 5);
        drop(interrupted);
        assert_eq!(current(&held), 0);
    }

    /// Con OR (o AND già soddisfatto) i risultati partono a lotti e non occupano posti.
    #[test]
    fn satisfied_documents_stream_in_batches() {
        let compiled = query(&["ar"], Combine::Any);
        let held = HeldHits::new(0);
        let mut batcher = HitBatcher::new(&compiled, &held);
        let mut batches = Vec::new();
        for n in 0..(2 * BATCH_HITS + 3) {
            if let Some(batch) = batcher.push(TermSet::single(AR), usize::MAX, || hit(n)) {
                batches.push(batch.len());
            }
        }
        assert_eq!(batches, [BATCH_HITS, BATCH_HITS]);
        assert_eq!(batcher.finish().hits_len(), 3);
        // Un lotto parte subito se basta a riempire il limite.
        let mut batcher = HitBatcher::satisfied(&query(&["ar", "zeta"], Combine::All), &held);
        assert_eq!(batcher.push(TermSet::single(AR), 2, || hit(0)), None);
        assert_eq!(
            batcher
                .push(TermSet::single(AR), 2, || hit(1))
                .map(|b| b.len()),
            Some(2)
        );
    }

    /// Oltre i posti del limite si tengono solo i risultati con un termine nuovo.
    #[test]
    fn beyond_the_limit_only_new_terms_are_kept() {
        let compiled = query(&["ar", "zeta"], Combine::All);
        let held = places(1000);
        let mut batcher = HitBatcher::new(&compiled, &held);
        for n in 0..10 {
            assert_eq!(batcher.push(TermSet::single(AR), 3, || hit(n)), None);
        }
        assert_eq!(held_count(&held), 3);
        let batch = batcher.push(TermSet::single(ZETA), 3, || hit(99));
        assert_eq!(batch.map(|b| b.len()), Some(4));
        assert_eq!(current(&held), 0);
    }

    impl Verdict {
        fn hits_len(&self) -> usize {
            match self {
                Verdict::Hits(hits) => hits.len(),
                Verdict::Reread => usize::MAX,
            }
        }
    }
}
