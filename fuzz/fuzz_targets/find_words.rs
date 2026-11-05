// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Ricerca (`find_words`) su testo e termini qualsiasi. Il primo byte sceglie le opzioni e
//! il taglio a blocchi; il resto, letto come UTF-8 con sostituzione, è "termini NUL testo",
//! con i termini (al più 4) separati da tabulazioni.
//!
//! Oracoli, gli invarianti di `crates/lynkeus-core/tests/proprieta.rs`: la parola contiene
//! gli evidenziati e sta nella sua riga, nessuna riga contiene un a capo, il numero di riga
//! è giusto, tutti gli offset cadono su confini UTF-8, parole ed evidenziati sono ordinati,
//! disgiunti e non vuoti, e cercare a blocchi di righe dà gli stessi risultati che
//! sull'intero testo.

#![no_main]

use std::ops::Range;

use libfuzzer_sys::fuzz_target;
use lynkeus_core::{
    AccentMode, CaseMode, Combine, CompiledQuery, MatchOptions, Query, Term, WildcardMode,
    WordMatch, find_words,
};

/// Termini al più per input.
const MAX_TERMS: usize = 4;

fn options(flags: u8) -> MatchOptions {
    MatchOptions {
        case: if flags & 1 == 0 {
            CaseMode::Insensitive
        } else {
            CaseMode::Sensitive
        },
        accents: if flags & 2 == 0 {
            AccentMode::Distinct
        } else {
            AccentMode::Ignored
        },
        wildcards: if flags & 4 == 0 {
            WildcardMode::Enabled
        } else {
            WildcardMode::Literal
        },
    }
}

fn compile(terms: &str, options: MatchOptions) -> Option<CompiledQuery> {
    let terms: Vec<Term> = terms
        .split('\t')
        .take(MAX_TERMS)
        .filter_map(|t| Term::parse(t, options.wildcards).ok())
        .collect();
    Query::new(terms, options, Combine::Any)
        .and_then(|q| q.compile())
        .ok()
}

fn on_boundaries(text: &str, range: &Range<usize>) -> bool {
    range.start <= range.end
        && text.is_char_boundary(range.start)
        && text.is_char_boundary(range.end)
}

/// Gli invarianti di ogni parola trovata in `text`.
fn check_words(text: &str, found: &[WordMatch]) {
    let mut previous_end = 0;
    for m in found {
        assert!(on_boundaries(text, &m.word) && on_boundaries(text, &m.line_range));
        assert!(m.word.start < m.word.end, "parola vuota");
        assert!(m.line_range.start <= m.word.start && m.word.end <= m.line_range.end);
        assert!(
            previous_end <= m.word.start,
            "parole sovrapposte o in disordine"
        );
        previous_end = m.word.end;
        assert!(
            !text[m.line_range.clone()].contains('\n'),
            "a capo nella riga"
        );
        assert_eq!(
            m.line as usize,
            text[..m.line_range.start].matches('\n').count()
        );
        assert!(!m.terms.is_empty() && !m.highlights.is_empty());
        let mut last_end = m.word.start;
        for (i, h) in m.highlights.iter().enumerate() {
            assert!(
                on_boundaries(text, h) && h.start < h.end,
                "evidenziato non valido"
            );
            assert!(
                h.start > last_end || (i == 0 && h.start == last_end),
                "evidenziati"
            );
            assert!(h.end <= m.word.end, "evidenziato fuori dalla parola");
            assert!(
                !text[h.clone()].contains(['\n', '\r']),
                "a capo nell'evidenziato"
            );
            last_end = h.end;
        }
    }
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

fuzz_target!(|data: &[u8]| {
    let Some((&flags, rest)) = data.split_first() else {
        return;
    };
    let input = String::from_utf8_lossy(rest);
    let Some((terms, text)) = input.split_once('\0') else {
        return;
    };
    let Some(query) = compile(terms, options(flags)) else {
        return;
    };
    let whole = find_words(&query, text);
    check_words(text, &whole);
    // A blocchi: taglio dopo l'a capo scelto dai bit alti del primo byte.
    let line_ends: Vec<usize> = text.match_indices('\n').map(|(i, _)| i + 1).collect();
    if line_ends.is_empty() {
        return;
    }
    let cut = line_ends[usize::from(flags >> 3) % line_ends.len()];
    let lines_before = u32::try_from(text[..cut].matches('\n').count()).unwrap_or(u32::MAX);
    let mut by_blocks = find_words(&query, &text[..cut]);
    by_blocks.extend(
        find_words(&query, &text[cut..])
            .into_iter()
            .map(|m| shifted(m, cut, lines_before)),
    );
    assert_eq!(by_blocks, whole, "a blocchi diverso dall'intero");
});
