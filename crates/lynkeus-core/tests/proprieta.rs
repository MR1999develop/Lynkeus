// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Test di proprietà di `find_words`, `word_text` e `snippet` sull'API pubblica.
//!
//! Invarianti: la parola contiene le corrispondenze e sta nella sua riga; nessuna
//! corrispondenza attraversa un a capo; tutti gli offset cadono su confini UTF-8; cercare
//! a blocchi di righe dà gli stessi risultati che sull'intero testo; i jolly spenti danno
//! una ricerca letterale; testo NFC e NFD danno le stesse parole.

use std::ops::Range;

use lynkeus_core::{
    AccentMode, CaseMode, Combine, CompiledQuery, MatchOptions, Query, Term, WildcardMode,
    WordMatch, find_words, snippet, word_text,
};
use proptest::prelude::*;
use unicode_normalization::UnicodeNormalization;

/// Pezzi di testo: lettere con e senza accenti (anche decomposti), spazi Unicode, a capo
/// LF e CRLF, punteggiatura e i caratteri dei jolly.
const TEXT_PIECES: &[&str] = &[
    "a", "b", "c", "à", "é", "e\u{301}", "A", "É", "ü", "ar", "carta", " ", "  ", "\t", "\u{a0}",
    "\n", "\r\n", ",", ".", "«", "»", "'", "*", "?", "$",
];

/// Pezzi dei termini.
const TERM_PIECES: &[&str] = &["a", "b", "c", "à", "e", "é", "ar", "*", "?", " ", ",", "."];

fn text_strategy(pieces: &'static [&'static str]) -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(pieces), 0..60).prop_map(|v| v.concat())
}

fn options_strategy() -> impl Strategy<Value = MatchOptions> {
    (any::<bool>(), any::<bool>(), any::<bool>()).prop_map(|(case, accents, wildcards)| {
        MatchOptions {
            case: if case {
                CaseMode::Insensitive
            } else {
                CaseMode::Sensitive
            },
            accents: if accents {
                AccentMode::Distinct
            } else {
                AccentMode::Ignored
            },
            wildcards: if wildcards {
                WildcardMode::Enabled
            } else {
                WildcardMode::Literal
            },
        }
    })
}

/// Da 1 a 3 termini di 1–4 pezzi; `None` se uno non è valido con queste opzioni.
fn terms_strategy() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(
        prop::collection::vec(prop::sample::select(TERM_PIECES), 1..5).prop_map(|v| v.concat()),
        1..4,
    )
}

fn compile(terms: &[String], options: MatchOptions) -> Option<CompiledQuery> {
    let terms: Option<Vec<Term>> = terms
        .iter()
        .map(|t| Term::parse(t, options.wildcards).ok())
        .collect();
    Query::new(terms?, options, Combine::Any)
        .and_then(|q| q.compile())
        .ok()
}

fn on_boundaries(text: &str, range: &Range<usize>) -> bool {
    text.is_char_boundary(range.start) && text.is_char_boundary(range.end)
}

/// Sposta un risultato trovato in un blocco che inizia al byte `offset` e alla riga `line`.
fn shifted(mut found: WordMatch, offset: usize, line: u32) -> WordMatch {
    let shift = |r: &Range<usize>| r.start + offset..r.end + offset;
    found.line += line;
    found.line_range = shift(&found.line_range);
    found.word = shift(&found.word);
    found.highlights = found.highlights.iter().map(shift).collect();
    found
}

/// Testo della parola e dei suoi evidenziati, in NFC.
fn nfc_words(text: &str, found: &[WordMatch]) -> Vec<(String, Vec<String>)> {
    found
        .iter()
        .map(|m| {
            let word = text[m.word.clone()].nfc().collect();
            let highlights = m
                .highlights
                .iter()
                .map(|h| text[h.clone()].nfc().collect())
                .collect();
            (word, highlights)
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// La parola contiene le corrispondenze e sta nella riga; niente a capo; confini UTF-8;
    /// parole ordinate e disgiunte; troncamenti su confini di carattere.
    #[test]
    fn word_contains_matches_and_offsets_are_valid(
        text in text_strategy(TEXT_PIECES),
        terms in terms_strategy(),
        options in options_strategy(),
        max_chars in 0usize..12,
    ) {
        let Some(query) = compile(&terms, options) else { return Ok(()); };
        let found = find_words(&query, &text);
        let mut previous_end = 0;
        for m in &found {
            prop_assert!(on_boundaries(&text, &m.word));
            prop_assert!(on_boundaries(&text, &m.line_range));
            prop_assert!(m.word.start < m.word.end);
            prop_assert!(m.line_range.start <= m.word.start && m.word.end <= m.line_range.end);
            prop_assert!(previous_end <= m.word.start, "parole sovrapposte");
            previous_end = m.word.end;
            prop_assert!(!text[m.line_range.clone()].contains('\n'));
            let phrase = terms.iter().any(|t| t.trim().contains(' '));
            prop_assert!(phrase || !text[m.word.clone()].contains(char::is_whitespace));
            let newlines = text[..m.line_range.start].matches('\n').count();
            prop_assert_eq!(m.line as usize, newlines);
            prop_assert!(!m.terms.is_empty());
            prop_assert!(!m.highlights.is_empty());
            let mut last_end = m.word.start;
            for (i, h) in m.highlights.iter().enumerate() {
                prop_assert!(on_boundaries(&text, h));
                prop_assert!(h.start < h.end);
                // Ordinati, disgiunti e non adiacenti.
                let after_previous = h.start > last_end || (i == 0 && h.start == last_end);
                prop_assert!(after_previous);
                prop_assert!(h.end <= m.word.end);
                prop_assert!(!text[h.clone()].contains(['\n', '\r']));
                last_end = h.end;
            }
            let word = word_text(&text, m, max_chars);
            prop_assert_eq!(word.truncated, word.truncated_start || word.truncated_end);
            if word.truncated {
                prop_assert_eq!(word.text.chars().count(), max_chars);
            } else {
                prop_assert_eq!(word.text.as_str(), &text[m.word.clone()]);
            }
            let context = snippet(&text, m, max_chars);
            prop_assert!(context.text.chars().count() <= max_chars);
            for h in &context.highlights {
                prop_assert!(context.text.get(h.clone()).is_some());
            }
        }
    }

    /// Cercare a blocchi di righe dà gli stessi risultati che sull'intero testo.
    #[test]
    fn blocks_of_lines_give_the_same_results(
        text in text_strategy(TEXT_PIECES),
        terms in terms_strategy(),
        options in options_strategy(),
        cuts in prop::collection::vec(any::<prop::sample::Index>(), 0..4),
    ) {
        let Some(query) = compile(&terms, options) else { return Ok(()); };
        let line_ends: Vec<usize> = text.match_indices('\n').map(|(i, _)| i + 1).collect();
        let mut boundaries: Vec<usize> = if line_ends.is_empty() {
            Vec::new()
        } else {
            cuts.iter().map(|c| line_ends[c.index(line_ends.len())]).collect()
        };
        boundaries.push(0);
        boundaries.push(text.len());
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut by_blocks = Vec::new();
        for pair in boundaries.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            let line = u32::try_from(text[..start].matches('\n').count()).unwrap_or(u32::MAX);
            by_blocks.extend(
                find_words(&query, &text[start..end])
                    .into_iter()
                    .map(|m| shifted(m, start, line)),
            );
        }
        prop_assert_eq!(by_blocks, find_words(&query, &text));
    }

    /// Con i jolly spenti, maiuscole e accenti distinti, gli evidenziati sono le
    /// occorrenze letterali del termine (non sovrapposte, da sinistra; quelle adiacenti unite).
    #[test]
    fn literal_wildcards_are_a_literal_search(
        text in text_strategy(&["a", "b", "à", "*", "?", " ", "\n", ",", "a*", "?b"]),
        term in prop::collection::vec(prop::sample::select(&["a", "b", "à", "*", "?", ","][..]), 1..4)
            .prop_map(|v| v.concat()),
    ) {
        let options = MatchOptions {
            case: CaseMode::Sensitive,
            accents: AccentMode::Distinct,
            wildcards: WildcardMode::Literal,
        };
        let Some(query) = compile(std::slice::from_ref(&term), options) else { return Ok(()); };
        let mut expected: Vec<Range<usize>> = Vec::new();
        for (i, _) in text.match_indices(term.as_str()) {
            let range = i..i + term.len();
            match expected.last_mut() {
                Some(last) if last.end == range.start => last.end = range.end,
                _ => expected.push(range),
            }
        }
        let actual: Vec<Range<usize>> = find_words(&query, &text)
            .into_iter()
            .flat_map(|m| m.highlights)
            .collect();
        prop_assert_eq!(actual, expected);
    }

    /// Lo stesso testo in NFC e in NFD dà le stesse parole e gli stessi evidenziati.
    #[test]
    fn nfc_and_nfd_text_give_the_same_words(
        text in text_strategy(TEXT_PIECES),
        terms in terms_strategy(),
        options in options_strategy(),
    ) {
        let Some(query) = compile(&terms, options) else { return Ok(()); };
        let nfc: String = text.nfc().collect();
        let nfd: String = text.nfd().collect();
        prop_assert_eq!(
            nfc_words(&nfc, &find_words(&query, &nfc)),
            nfc_words(&nfd, &find_words(&query, &nfd))
        );
    }
}
