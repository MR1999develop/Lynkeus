// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Test di proprietà di `find_words`, `word_text` e `snippet` sull'API pubblica.
//!
//! Invarianti: la parola contiene le corrispondenze e sta nella sua riga; nessuna
//! corrispondenza attraversa un a capo; tutti gli offset cadono su confini UTF-8; cercare
//! a blocchi di righe dà gli stessi risultati che sull'intero testo; i jolly spenti danno
//! una ricerca letterale; testo NFC e NFD danno le stesse parole (anche hangul e vietnamita);
//! ogni termine trova sé stesso con tutte le opzioni; su testo ASCII i risultati coincidono
//! con quelli di un matcher ingenuo scritto qui (oracolo differenziale).

use std::ops::Range;
use std::sync::LazyLock;

use lynkeus_core::{
    AccentMode, CaseMode, Combine, CompiledQuery, MatchOptions, Query, Term, WildcardMode,
    WordMatch, find_words, snippet, word_text,
};
use proptest::prelude::*;
use regex::Regex;
use unicode_normalization::UnicodeNormalization;

/// Pezzi di testo: lettere con e senza accenti (anche decomposti), sillabe hangul (con e
/// senza consonante finale), vietnamita con due segni, spazi Unicode, a capo LF e CRLF,
/// punteggiatura e i caratteri dei jolly.
const TEXT_PIECES: &[&str] = &[
    "a", "b", "c", "à", "é", "e\u{301}", "A", "É", "ü", "ar", "carta", " ", "  ", "\t", "\u{a0}",
    "\n", "\r\n", ",", ".", "«", "»", "'", "*", "?", "$", "한국", "국", "구", "사람", "Việt", "ệ",
    "ế",
];

/// Pezzi dei termini.
const TERM_PIECES: &[&str] = &[
    "a", "b", "c", "à", "e", "é", "ar", "*", "?", " ", ",", ".", "한", "국", "구", "ệ",
];

/// Le 8 combinazioni di opzioni.
fn every_option() -> Vec<MatchOptions> {
    let mut all = Vec::new();
    for case in [CaseMode::Insensitive, CaseMode::Sensitive] {
        for accents in [AccentMode::Distinct, AccentMode::Ignored] {
            for wildcards in [WildcardMode::Enabled, WildcardMode::Literal] {
                all.push(MatchOptions {
                    case,
                    accents,
                    wildcards,
                });
            }
        }
    }
    all
}

/// Caratteri dei termini che devono trovare sé stessi: lettere latine accentate (anche
/// vietnamite), greco (anche politonico e i suoi accenti spaziati), sillabe e jamo hangul,
/// ideogrammi CJK di compatibilità, segni combinanti, cifre, punteggiatura, simboli e jolly.
fn self_char() -> impl Strategy<Value = char> {
    prop_oneof![
        prop::char::range('a', 'z'),
        prop::char::range('A', 'Z'),
        prop::char::range('\u{C0}', '\u{24F}'),
        prop::char::range('\u{1EA0}', '\u{1EF9}'),
        prop::char::range('\u{300}', '\u{36F}'),
        prop::char::range('\u{370}', '\u{3FF}'),
        prop::char::range('\u{1F00}', '\u{1FFF}'),
        prop::char::range('\u{AC00}', '\u{D7A3}'),
        prop::char::range('\u{1100}', '\u{11FF}'),
        prop::char::range('\u{F900}', '\u{FAD9}'),
        prop::sample::select(vec!['0', '7', '.', ',', '«', '»', '*', '?', '$', '≠']),
    ]
}

/// Un segno combinante non spaziato (categoria `Mn`).
fn is_mark(c: char) -> bool {
    static MARK: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\A\p{Mn}\z").ok());
    let mut buf = [0_u8; 4];
    MARK.as_ref()
        .is_some_and(|mark| mark.is_match(c.encode_utf8(&mut buf)))
}

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

/// Pezzi del testo dell'oracolo: lettere ASCII, punteggiatura (anche non ASCII), un simbolo
/// e spazi Unicode, a capo compresi.
const ORACLE_TEXT: &[&str] = &[
    "a", "b", "A", "ab", "aab", ".", ",", "*", "?", "«", "$", " ", "\t", "\n", "\u{a0}",
];

/// Pezzi dei termini dell'oracolo: senza spazi, ogni corrispondenza sta in una parola.
const ORACLE_TERM: &[&str] = &["a", "b", "A", "*", "?", ".", ","];

/// Punteggiatura (categoria `P`) tra i caratteri dell'oracolo; `$` è un simbolo.
fn oracle_punctuation(c: char) -> bool {
    matches!(c, '.' | ',' | '*' | '?' | '«')
}

/// Un pezzo di termine per l'oracolo.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Piece {
    /// `*`: zero o più caratteri non spazio, il più lungo possibile.
    Star,
    /// `?`: un carattere non spazio.
    One,
    /// Un carattere letterale.
    Literal(char),
}

fn oracle_pieces(term: &str, wildcards: bool) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = Vec::new();
    for c in term.chars() {
        let piece = match c {
            '*' if wildcards => Piece::Star,
            '?' if wildcards => Piece::One,
            c => Piece::Literal(c),
        };
        if !(piece == Piece::Star && pieces.last() == Some(&Piece::Star)) {
            pieces.push(piece);
        }
    }
    pieces
}

/// La corrispondenza di `pieces` che inizia al carattere `at`, con backtracking (ogni `*`
/// prova prima la parte più lunga): l'inizio di ogni pezzo e, in fondo, la fine.
fn match_at(pieces: &[Piece], text: &[char], at: usize, fold: bool) -> Option<Vec<usize>> {
    let Some((first, rest)) = pieces.split_first() else {
        return Some(vec![at]);
    };
    let then = |next: usize| {
        match_at(rest, text, next, fold).map(|mut starts| {
            starts.insert(0, at);
            starts
        })
    };
    match *first {
        Piece::Literal(c) => text
            .get(at)
            .filter(|&&t| t == c || (fold && t.eq_ignore_ascii_case(&c)))
            .and_then(|_| then(at + 1)),
        Piece::One => text
            .get(at)
            .filter(|t| !t.is_whitespace())
            .and_then(|_| then(at + 1)),
        Piece::Star => {
            let run = text
                .get(at..)
                .unwrap_or_default()
                .iter()
                .take_while(|c| !c.is_whitespace())
                .count();
            (0..=run).rev().find_map(|len| then(at + len))
        }
    }
}

/// Le corrispondenze di un termine (indici di carattere), da sinistra e non sovrapposte,
/// senza la punteggiatura di bordo presa dai `*` iniziale e finale.
fn oracle_hits(pieces: &[Piece], text: &[char], fold: bool) -> Vec<Range<usize>> {
    let lead = pieces.len() > 1 && pieces.first() == Some(&Piece::Star);
    let trail = pieces.len() > 1 && pieces.last() == Some(&Piece::Star);
    let mut hits = Vec::new();
    let mut from = 0;
    while let Some((start, starts)) =
        (from..=text.len()).find_map(|s| match_at(pieces, text, s, fold).map(|p| (s, p)))
    {
        let end = starts.last().copied().unwrap_or(start);
        from = end.max(start + 1);
        let mut hit = start..end;
        if lead {
            let lead_end = starts.get(1).copied().unwrap_or(end);
            while hit.start < lead_end && oracle_punctuation(text[hit.start]) {
                hit.start += 1;
            }
        }
        if trail {
            let trail_start = starts.get(pieces.len() - 1).copied().unwrap_or(start);
            while hit.end > trail_start.max(hit.start) && oracle_punctuation(text[hit.end - 1]) {
                hit.end -= 1;
            }
        }
        if hit.start < hit.end {
            hits.push(hit);
        }
    }
    hits
}

/// Una parola trovata, nella forma confrontata: parola, evidenziati e termini.
type Found = (Range<usize>, Vec<Range<usize>>, Vec<usize>);

/// Il risultato atteso secondo un matcher ingenuo: per ogni parola (sequenza di non spazi)
/// con corrispondenze, gli evidenziati uniti se si toccano e la parola senza la
/// punteggiatura di bordo fuori dagli evidenziati. Offset in byte.
fn oracle_words(terms: &[String], options: MatchOptions, text: &str) -> Vec<Found> {
    let chars: Vec<char> = text.chars().collect();
    let mut offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    offsets.push(text.len());
    let fold = options.case == CaseMode::Insensitive;
    let wildcards = options.wildcards == WildcardMode::Enabled;
    let mut hits: Vec<(Range<usize>, usize)> = Vec::new();
    for (index, term) in terms.iter().enumerate() {
        let pieces = oracle_pieces(term.trim(), wildcards);
        for hit in oracle_hits(&pieces, &chars, fold) {
            hits.push((hit, index));
        }
    }
    let mut words = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        if chars[start].is_whitespace() {
            start += 1;
            continue;
        }
        let end = start
            + chars[start..]
                .iter()
                .take_while(|c| !c.is_whitespace())
                .count();
        let mut inside: Vec<&(Range<usize>, usize)> = hits
            .iter()
            .filter(|(h, _)| start <= h.start && h.end <= end)
            .collect();
        inside.sort_by_key(|(h, _)| h.start);
        let mut highlights: Vec<Range<usize>> = Vec::new();
        for (hit, _) in &inside {
            match highlights.last_mut() {
                Some(last) if hit.start <= last.end => last.end = last.end.max(hit.end),
                _ => highlights.push(hit.clone()),
            }
        }
        if let (Some(first), Some(last)) = (highlights.first(), highlights.last()) {
            let mut word = start..end;
            while word.start < first.start && oracle_punctuation(chars[word.start]) {
                word.start += 1;
            }
            while word.end > last.end && oracle_punctuation(chars[word.end - 1]) {
                word.end -= 1;
            }
            let mut found_terms: Vec<usize> = inside.iter().map(|(_, t)| *t).collect();
            found_terms.sort_unstable();
            found_terms.dedup();
            let bytes = |r: &Range<usize>| offsets[r.start]..offsets[r.end];
            words.push((
                bytes(&word),
                highlights.iter().map(bytes).collect(),
                found_terms,
            ));
        }
        start = end;
    }
    words
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Ogni stringa NFC senza spazi trova sé stessa con tutte le 8 combinazioni di opzioni;
    /// è rifiutata solo se, tolti i segni combinanti, resta vuota o fatta di soli jolly.
    #[test]
    fn term_finds_itself_in_every_mode(chars in prop::collection::vec(self_char(), 1..8)) {
        let raw: String = chars.into_iter().filter(|c| !c.is_whitespace()).collect();
        let text: String = raw.nfc().collect();
        for options in every_option() {
            let wildcards = options.wildcards == WildcardMode::Enabled;
            let Ok(term) = Term::parse(&text, options.wildcards) else {
                let only_marks_and_wildcards = text
                    .nfd()
                    .filter(|&c| !is_mark(c))
                    .all(|c| wildcards && matches!(c, '*' | '?'));
                prop_assert!(only_marks_and_wildcards, "{text:?} rifiutato con {options:?}");
                continue;
            };
            let query = Query::new(vec![term], options, Combine::Any).and_then(|q| q.compile());
            prop_assert!(query.is_ok(), "{text:?} {options:?}");
            if let Ok(query) = query {
                prop_assert!(!find_words(&query, &text).is_empty(), "{text:?} {options:?}");
            }
        }
    }

    /// Su testo ASCII (con punteggiatura e spazi Unicode) e termini senza spazi, i risultati
    /// sono quelli del matcher ingenuo: jolly, punteggiatura di bordo, unione di evidenziati
    /// e di termini nella stessa parola.
    #[test]
    fn find_words_matches_naive_oracle(
        text in text_strategy(ORACLE_TEXT),
        terms in prop::collection::vec(
            prop::collection::vec(prop::sample::select(ORACLE_TERM), 1..5).prop_map(|v| v.concat()),
            1..4,
        ),
        options in options_strategy(),
    ) {
        let Some(query) = compile(&terms, options) else { return Ok(()); };
        let actual: Vec<Found> = find_words(&query, &text)
            .into_iter()
            .map(|m| (m.word, m.highlights, m.terms.iter().collect()))
            .collect();
        prop_assert_eq!(actual, oracle_words(&terms, options, &text));
    }

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
