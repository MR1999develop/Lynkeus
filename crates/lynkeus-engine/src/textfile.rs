// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Ricerca in un file di testo, blocco per blocco.
//!
//! I blocchi di [`TextReader`] (circa 8 MB, tagliati a fine riga) si dividono in passi
//! ([`steps`]): a ogni passo si controlla l'annullamento e si cercano le parole con
//! [`find_words`]. I numeri di riga si ricostruiscono contando gli a capo.
//!
//! OR, AND, lotti e limite li decide [`HitBatcher`]: se dice che il file va riletto
//! ([`Verdict::Reread`]), chi chiama lo riapre e cerca di nuovo con un batcher già sicuro.

use std::io::Read;

use lynkeus_core::{Location, find_words};
use lynkeus_extract::{ExtractError, TextReader};

use crate::batch::{HitBatcher, HitLabel, HitSink, Verdict};
use crate::halt::Stop;
use crate::step::steps;

/// Perché la ricerca in un file si è interrotta.
#[derive(Debug)]
pub(crate) enum FileError {
    /// Errore di lettura dopo l'apertura.
    Read(ExtractError),
    /// La ricerca intera si ferma.
    Stop(Stop),
}

impl From<Stop> for FileError {
    fn from(stop: Stop) -> Self {
        FileError::Stop(stop)
    }
}

/// Com'è finita la ricerca in un file letto per intero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextEnd {
    /// Risultati consegnati (forse nessuno).
    Searched,
    /// Il file va riletto con un batcher già sicuro (vedi [`HitBatcher`]): nulla è stato
    /// consegnato.
    Reread,
}

/// Cerca in tutto il testo di `reader` con `batcher` e consegna i risultati a `sink`.
pub(crate) fn search_text<R: Read>(
    reader: &mut TextReader<R>,
    label: &HitLabel<'_>,
    mut batcher: HitBatcher<'_>,
    sink: &mut impl HitSink,
) -> Result<TextEnd, FileError> {
    while let Some(chunk) = reader.next_chunk().map_err(FileError::Read)? {
        for step in steps(chunk.text) {
            sink.check()?;
            let line = chunk.first_line.saturating_add(step.lines_before);
            let room = sink.remaining();
            for found in find_words(label.query, step.text) {
                let make = || {
                    let number = line.saturating_add(u64::from(found.line)).saturating_add(1);
                    let location = Location::Line(u32::try_from(number).unwrap_or(u32::MAX));
                    label.hit(step.text, &found, location)
                };
                if let Some(batch) = batcher.push(found.terms, room, make) {
                    sink.deliver(batch)?;
                }
            }
            if batcher.is_settled() {
                return Ok(TextEnd::Reread);
            }
        }
    }
    match batcher.finish() {
        Verdict::Hits(hits) => {
            if !hits.is_empty() {
                sink.deliver(hits)?;
            }
            Ok(TextEnd::Searched)
        }
        Verdict::Reread => Ok(TextEnd::Reread),
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::sync::Arc;

    use super::*;
    use crate::batch::{BATCH_HITS, HeldHits};
    use crate::doc::DocPath;
    use crate::event::{Hit, WarningKind};
    use crate::request::Limits;
    use lynkeus_core::{ByteSize, Combine, CompiledQuery, MatchOptions, Query, Term, WildcardMode};
    use proptest::prelude::*;

    fn query(terms: &[&str], combine: Combine) -> CompiledQuery {
        let terms = terms
            .iter()
            .map(|t| Term::parse(t, WildcardMode::Enabled).expect("termine valido"))
            .collect();
        Query::new(terms, MatchOptions::default(), combine)
            .and_then(|q| q.compile())
            .expect("query valida")
    }

    /// Raccoglie i lotti; si ferma (come la ricerca vera) quando il limite è pieno.
    struct Collect {
        remaining: usize,
        batches: Vec<Vec<Hit>>,
    }

    impl HitSink for Collect {
        fn check(&self) -> Result<(), Stop> {
            Ok(())
        }
        fn remaining(&self) -> usize {
            self.remaining
        }
        fn deliver(&mut self, mut hits: Vec<Hit>) -> Result<(), Stop> {
            let reached = hits.len() >= self.remaining;
            hits.truncate(self.remaining);
            self.remaining -= hits.len();
            self.batches.push(hits);
            if reached {
                Err(Stop::LimitReached)
            } else {
                Ok(())
            }
        }
        fn warn(&mut self, _doc: Arc<DocPath>, _kind: WarningKind) -> Result<(), Stop> {
            Ok(())
        }
    }

    /// Cerca `text` con al più `held` risultati trattenuti; l'esito e i lotti consegnati.
    fn search(
        text: &str,
        query: &CompiledQuery,
        limits: &Limits,
        sink: &mut Collect,
        held: usize,
        block: usize,
    ) -> Result<TextEnd, FileError> {
        let label = HitLabel {
            query,
            limits,
            root: 0,
            doc: Arc::new(DocPath::file("prova.txt")),
        };
        let block = NonZeroUsize::new(block).unwrap_or(NonZeroUsize::MIN);
        let mut reader =
            TextReader::with_block_bytes(text.as_bytes(), ByteSize::from_mib(64), block)
                .expect("testo");
        let held = HeldHits::new(held);
        search_text(&mut reader, &label, HitBatcher::new(query, &held), sink)
    }

    fn run(
        text: &str,
        terms: &[&str],
        combine: Combine,
        max_results: usize,
        block: usize,
    ) -> (Vec<(String, Location)>, usize, bool) {
        let query = query(terms, combine);
        let mut sink = Collect {
            remaining: max_results,
            batches: Vec::new(),
        };
        let stopped = matches!(
            search(
                text,
                &query,
                &Limits::default(),
                &mut sink,
                usize::MAX,
                block
            ),
            Err(FileError::Stop(Stop::LimitReached))
        );
        let batches = sink.batches.len();
        let hits = sink
            .batches
            .into_iter()
            .flatten()
            .map(|h| (h.word, h.location))
            .collect();
        (hits, batches, stopped)
    }

    #[test]
    fn line_numbers_across_blocks() {
        let mut text = "riga\n".repeat(5000);
        text.push_str("l'aragosta\n");
        text.push_str(&"riga\n".repeat(5000));
        text.push_str("carta");
        let (hits, _, stopped) = run(&text, &["ar"], Combine::Any, 100, 4096);
        assert!(!stopped);
        assert_eq!(
            hits,
            [
                ("l'aragosta".to_owned(), Location::Line(5001)),
                ("carta".to_owned(), Location::Line(10_002)),
            ]
        );
    }

    #[test]
    fn and_needs_every_term_in_the_file() {
        let text = "carta\nsasso\n";
        let (hits, _, _) = run(text, &["ar", "zz"], Combine::All, 100, 4096);
        assert!(hits.is_empty());
        let (hits, batches, _) = run(text, &["ar", "ss"], Combine::All, 100, 4096);
        assert_eq!(hits.len(), 2);
        assert_eq!(batches, 1);
    }

    #[test]
    fn a_long_word_says_which_side_was_cut() {
        let query = query(&["zeta"], Combine::Any);
        let limits = Limits {
            word_max_chars: 10,
            ..Limits::default()
        };
        let text = format!(
            "{}zeta{}
zetabc
",
            "a".repeat(30),
            "b".repeat(30)
        );
        let mut sink = Collect {
            remaining: 10,
            batches: Vec::new(),
        };
        search(&text, &query, &limits, &mut sink, 10, 4096).expect("ricerca");
        let sides: Vec<(bool, bool)> = sink
            .batches
            .iter()
            .flatten()
            .map(|h| (h.word_truncated_start, h.word_truncated_end))
            .collect();
        assert_eq!(sides, [(true, true), (false, false)]);
    }

    #[test]
    fn and_streams_in_batches_once_every_term_has_appeared() {
        // Tutti i termini nella prima riga: da lì AND consegna a lotti come OR, invece di
        // trattenere ogni risultato fino alla fine del file.
        let mut text = "zeta ar\n".to_owned();
        text.push_str(&"ar\n".repeat(3 * BATCH_HITS));
        let (hits, batches, stopped) = run(&text, &["ar", "zeta"], Combine::All, usize::MAX, 4096);
        assert!(!stopped);
        assert_eq!(hits.len(), 3 * BATCH_HITS + 2);
        assert!(batches > 1, "lotti: {batches}");
        assert_eq!(hits[0], ("zeta".to_owned(), Location::Line(1)));
        assert_eq!(
            hits.last().map(|h| h.1.clone()),
            Some(Location::Line(
                u32::try_from(3 * BATCH_HITS + 1).unwrap_or(0)
            ))
        );
        // Con un termine che manca il file resta escluso per intero.
        let (hits, batches, _) = run(&text, &["ar", "omega"], Combine::All, usize::MAX, 4096);
        assert!(hits.is_empty());
        assert_eq!(batches, 0);
    }

    #[test]
    fn the_limit_stops_the_file_and_bounds_what_is_kept() {
        let text = "ar ".repeat(10_000);
        let (hits, _, stopped) = run(&text, &["ar"], Combine::Any, 7, 4096);
        assert!(stopped);
        assert_eq!(hits.len(), 7);
        // AND: il secondo termine compare solo alla fine, oltre il limite: il file vale.
        let mut text = "ar\n".repeat(10_000);
        text.push_str("zeta\n");
        let (hits, _, stopped) = run(&text, &["ar", "zeta"], Combine::All, 5, 4096);
        assert!(stopped);
        assert_eq!(hits.len(), 5);
    }

    /// Senza posti per trattenere i risultati (altri file li occupano), un file AND che ha
    /// tutti i termini chiede la seconda lettura e non consegna nulla; con i posti, consegna.
    #[test]
    fn without_places_to_hold_hits_the_file_is_reread() {
        let mut text = "ar
"
        .repeat(50);
        text.push_str(
            "zeta
",
        );
        text.push_str(
            &"ar
"
            .repeat(3 * BATCH_HITS),
        );
        let query = query(&["ar", "zeta"], Combine::All);
        let limits = Limits::default();
        let mut sink = Collect {
            remaining: usize::MAX,
            batches: Vec::new(),
        };
        let end = search(&text, &query, &limits, &mut sink, 10, 4096).expect("ricerca");
        assert_eq!(end, TextEnd::Reread);
        assert!(sink.batches.is_empty());
        let end = search(&text, &query, &limits, &mut sink, 100, 4096).expect("ricerca");
        assert_eq!(end, TextEnd::Searched);
        let delivered: usize = sink.batches.iter().map(Vec::len).sum();
        assert_eq!(delivered, 3 * BATCH_HITS + 51);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// Blocchi = intero: la ricerca a blocchi piccoli trova le stesse righe della
        /// ricerca sul testo intero (con righe più corte di un blocco).
        #[test]
        fn blocks_equal_whole(
            lines in proptest::collection::vec("[a-c ]{0,30}", 0..2000),
            block in 32usize..200,
        ) {
            let text = lines.join("\n");
            let (chunked, _, _) = run(&text, &["ab"], Combine::Any, usize::MAX, block);
            let (whole, _, _) = run(&text, &["ab"], Combine::Any, usize::MAX, 1 << 20);
            prop_assert_eq!(chunked, whole);
        }
    }
}
