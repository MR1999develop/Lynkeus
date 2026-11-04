// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Ricerca in un file di testo, blocco per blocco.
//!
//! I blocchi di [`TextReader`] (circa 8 MB, tagliati a fine riga) si dividono in passi di
//! al massimo [`STEP_MAX_LINES`] righe e [`STEP_MAX_BYTES`] byte: a ogni passo si controlla
//! l'annullamento e si cercano le parole con [`find_words`]. I numeri di riga si
//! ricostruiscono contando gli a capo con `memchr`.
//!
//! OR e AND con [`FileAccumulator`]: i risultati partono a lotti durante la lettura appena
//! sono sicuri, cioè sempre con OR e, con AND, da quando ogni termine è comparso nel file
//! (prima si trattengono; se un termine non compare mai, il file non dà risultati). Non si
//! trattengono mai più risultati di quelli che il limite permette ancora di consegnare.

use std::io::Read;
use std::sync::Arc;

use lynkeus_core::{
    CompiledQuery, FileAccumulator, Location, WordMatch, find_words, snippet, word_text,
};
use lynkeus_extract::text::soft_cut;
use lynkeus_extract::{ExtractError, TextReader};

use crate::doc::DocPath;
use crate::event::Hit;
use crate::request::Limits;
use crate::worker::Stop;

/// Righe massime di un passo, tra due controlli dell'annullamento.
pub(crate) const STEP_MAX_LINES: usize = 10_000;

/// Byte massimi di un passo: limita anche la memoria dei risultati di [`find_words`].
pub(crate) const STEP_MAX_BYTES: usize = 1024 * 1024;

/// Risultati già sicuri trattenuti al massimo prima di consegnarne un lotto.
const BATCH_HITS: usize = 1024;

/// Dove vanno i risultati di un file.
pub(crate) trait HitSink {
    /// Errore se la ricerca deve fermarsi (annullata, limite raggiunto, chiamante sparito).
    fn check(&self) -> Result<(), Stop>;
    /// Risultati che il limite permette ancora di consegnare.
    fn remaining(&self) -> usize;
    /// Consegna un lotto (mai vuoto) di risultati già filtrati da OR/AND.
    fn deliver(&mut self, hits: Vec<Hit>) -> Result<(), Stop>;
}

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

/// Cosa cercare e come etichettare i risultati di un file.
pub(crate) struct FileSearch<'a> {
    pub query: &'a CompiledQuery,
    pub limits: &'a Limits,
    pub root: usize,
    pub doc: Arc<DocPath>,
}

/// Cerca in tutto il testo di `reader` e consegna i risultati a `sink`.
pub(crate) fn search_text<R: Read>(
    reader: &mut TextReader<R>,
    file: &FileSearch<'_>,
    sink: &mut impl HitSink,
) -> Result<(), FileError> {
    let mut accumulator = FileAccumulator::new(file.query.term_count(), file.query.combine());
    while let Some(chunk) = reader.next_chunk().map_err(FileError::Read)? {
        let mut offset = 0;
        let mut line = chunk.first_line;
        while offset < chunk.text.len() {
            sink.check()?;
            let rest = &chunk.text[offset..];
            let step = &rest[..step_len(rest)];
            let room = sink.remaining();
            for found in find_words(file.query, step) {
                // Oltre il limite servono solo i risultati che portano un termine nuovo
                // (perché AND sappia se il file li contiene tutti).
                if accumulator.len() >= room && accumulator.seen().contains_all(found.terms) {
                    continue;
                }
                accumulator.push(found.terms, make_hit(file, step, &found, line));
            }
            line = line.saturating_add(count_newlines(step));
            offset += step.len();

            // Risultati già sicuri (sempre con OR, con AND da quando ogni termine è
            // comparso): partono a lotti, o subito se bastano a riempire il limite (allora
            // `deliver` ferma la ricerca).
            let held = accumulator.len();
            if accumulator.is_satisfied() && (held >= room || held >= BATCH_HITS) {
                deliver_ready(sink, &mut accumulator)?;
            }
        }
    }
    deliver(sink, accumulator).map_err(FileError::Stop)
}

/// Consegna i risultati già sicuri dell'accumulatore, se ce ne sono.
fn deliver_ready(
    sink: &mut impl HitSink,
    accumulator: &mut FileAccumulator<Hit>,
) -> Result<(), Stop> {
    let hits = accumulator.take_ready();
    if hits.is_empty() {
        Ok(())
    } else {
        sink.deliver(hits)
    }
}

/// Chiude l'accumulatore e consegna ciò che resta, se qualcosa resta.
fn deliver(sink: &mut impl HitSink, accumulator: FileAccumulator<Hit>) -> Result<(), Stop> {
    let hits = accumulator.finish();
    if hits.is_empty() {
        Ok(())
    } else {
        sink.deliver(hits)
    }
}

/// Una riga dei risultati; `first_line` è l'indice nel file della prima riga di `text`.
fn make_hit(file: &FileSearch<'_>, text: &str, found: &WordMatch, first_line: u64) -> Hit {
    let word = word_text(text, found, file.limits.word_max_chars);
    let line = first_line
        .saturating_add(u64::from(found.line))
        .saturating_add(1);
    Hit {
        doc: Arc::clone(&file.doc),
        root: file.root,
        location: Location::Line(u32::try_from(line).unwrap_or(u32::MAX)),
        word: word.text,
        word_truncated_start: word.truncated_start,
        word_truncated_end: word.truncated_end,
        snippet: snippet(text, found, file.limits.context_chars),
        terms: found.terms,
    }
}

/// Lunghezza del prossimo passo di `rest`: fino alla fine della riga numero
/// [`STEP_MAX_LINES`], entro [`STEP_MAX_BYTES`]; una riga più lunga si taglia dopo uno
/// spazio. Sempre almeno un carattere se `rest` non è vuoto.
pub(crate) fn step_len(rest: &str) -> usize {
    let bytes = rest.as_bytes();
    let window = &bytes[..bytes.len().min(STEP_MAX_BYTES)];
    if let Some(newline) = memchr::memchr_iter(b'\n', window).nth(STEP_MAX_LINES - 1) {
        return newline + 1;
    }
    if window.len() == bytes.len() {
        return bytes.len();
    }
    match memchr::memrchr(b'\n', window) {
        Some(newline) => newline + 1,
        None => soft_cut(rest, STEP_MAX_BYTES),
    }
}

fn count_newlines(text: &str) -> u64 {
    let count = memchr::memchr_iter(b'\n', text.as_bytes()).count();
    u64::try_from(count).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::*;
    use lynkeus_core::{ByteSize, Combine, MatchOptions, Query, Term, WildcardMode};
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
    }

    fn run(
        text: &str,
        terms: &[&str],
        combine: Combine,
        max_results: usize,
        block: usize,
    ) -> (Vec<(String, Location)>, usize, bool) {
        let query = query(terms, combine);
        let limits = Limits::default();
        let file = FileSearch {
            query: &query,
            limits: &limits,
            root: 0,
            doc: Arc::new(DocPath::file("prova.txt")),
        };
        let block = NonZeroUsize::new(block).unwrap_or(NonZeroUsize::MIN);
        let mut reader =
            TextReader::with_block_bytes(text.as_bytes(), ByteSize::from_mib(64), block)
                .expect("testo");
        let mut sink = Collect {
            remaining: max_results,
            batches: Vec::new(),
        };
        let stopped = matches!(
            search_text(&mut reader, &file, &mut sink),
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
        let file = FileSearch {
            query: &query,
            limits: &limits,
            root: 0,
            doc: Arc::new(DocPath::file("prova.txt")),
        };
        let text = format!("{}zeta{}\nzetabc\n", "a".repeat(30), "b".repeat(30));
        let mut reader = TextReader::with_block_bytes(
            text.as_bytes(),
            ByteSize::from_mib(64),
            NonZeroUsize::new(4096).unwrap_or(NonZeroUsize::MIN),
        )
        .expect("testo");
        let mut sink = Collect {
            remaining: 10,
            batches: Vec::new(),
        };
        search_text(&mut reader, &file, &mut sink).expect("ricerca");
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

    #[test]
    fn step_len_partitions() {
        assert_eq!(step_len(""), 0);
        assert_eq!(step_len("abc"), 3);
        let lines = "x\n".repeat(STEP_MAX_LINES + 5);
        assert_eq!(step_len(&lines), STEP_MAX_LINES * 2);
        let long = format!("{} fine\naltro", "a".repeat(STEP_MAX_BYTES));
        assert_eq!(step_len(&long), STEP_MAX_BYTES);
        let spaced = format!("{} {}", "a".repeat(10), "b".repeat(STEP_MAX_BYTES));
        assert_eq!(step_len(&spaced), 11);
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
