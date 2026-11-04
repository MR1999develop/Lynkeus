// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Proprietà Unicode usate dalla ricerca: segni combinanti non spaziati (`Mn`),
//! punteggiatura (`P`) e varianti accentate di una lettera.
//!
//! Le tabelle si costruiscono una volta sola, al primo uso (`LazyLock`), dalle tabelle di
//! `regex` (categorie generali) e di `unicode-normalization` (decomposizione canonica).

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::decompose_canonical;

/// Un carattere della categoria `Mn` (segno combinante non spaziato).
static NONSPACING_MARK: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\A\p{Mn}\z").ok());

/// Un carattere della categoria `P` (punteggiatura).
static PUNCTUATION: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\A\p{P}\z").ok());

/// Lettera di base → lettere precomposte la cui decomposizione canonica completa è la lettera
/// di base seguita solo da segni `Mn` (es. `e` → `è`, `é`, `ê`, …), in ordine di codice.
static ACCENTED_VARIANTS: LazyLock<HashMap<char, Vec<char>>> = LazyLock::new(build_variants);

/// Vero se `c` è un segno combinante non spaziato (categoria generale `Mn`).
pub(crate) fn is_nonspacing_mark(c: char) -> bool {
    // Fast path: nessun carattere ASCII è un segno combinante.
    !c.is_ascii() && matches_char(&NONSPACING_MARK, c)
}

/// Vero se `c` è un segno di punteggiatura (categoria generale `P`: `Pc`, `Pd`, `Ps`, `Pe`,
/// `Pi`, `Pf`, `Po`). I simboli (`$`, `+`, `<`, …, categoria `S`) non lo sono.
pub(crate) fn is_punctuation(c: char) -> bool {
    matches_char(&PUNCTUATION, c)
}

/// Le varianti accentate precomposte di `base` (vuoto se non ne ha).
pub(crate) fn accented_variants(base: char) -> &'static [char] {
    ACCENTED_VARIANTS.get(&base).map_or(&[], Vec::as_slice)
}

/// La decomposizione canonica completa (NFD) di `c`, se diversa da `c`.
pub(crate) fn canonical_decomposition(c: char) -> Option<String> {
    let decomposed: String = std::iter::once(c).nfd().collect();
    let mut chars = decomposed.chars();
    match (chars.next(), chars.next()) {
        (Some(first), None) if first == c => None,
        _ => Some(decomposed),
    }
}

fn matches_char(regex: &LazyLock<Option<Regex>>, c: char) -> bool {
    let mut buf = [0u8; 4];
    regex
        .as_ref()
        .is_some_and(|r| r.is_match(c.encode_utf8(&mut buf)))
}

fn build_variants() -> HashMap<char, Vec<char>> {
    let mut table: HashMap<char, Vec<char>> = HashMap::new();
    let mut parts: Vec<char> = Vec::with_capacity(4);
    for code in 0..=u32::from(char::MAX) {
        let Some(c) = char::from_u32(code) else {
            continue;
        };
        if c.is_ascii() {
            continue;
        }
        parts.clear();
        decompose_canonical(c, |part| parts.push(part));
        let Some((&base, marks)) = parts.split_first() else {
            continue;
        };
        if !marks.is_empty() && base.is_alphabetic() && marks.iter().all(|&m| is_nonspacing_mark(m))
        {
            table.entry(base).or_default().push(c);
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_and_punctuation() {
        assert!(is_nonspacing_mark('\u{0301}'));
        assert!(!is_nonspacing_mark('a'));
        assert!(!is_nonspacing_mark('\u{0903}'), "Mc non è Mn");
        for c in ['.', ',', '!', '«', '»', '“', '…', '–', '¿', '_', '(', ')'] {
            assert!(is_punctuation(c), "{c:?}");
        }
        for c in ['a', '$', '+', '<', ' ', '\u{0301}'] {
            assert!(!is_punctuation(c), "{c:?}");
        }
    }

    #[test]
    fn variants_come_from_canonical_decomposition() {
        let e = accented_variants('e');
        for c in ['è', 'é', 'ê', 'ë', 'ě', 'ẽ', 'ệ'] {
            assert!(e.contains(&c), "{c:?}");
        }
        assert!(!e.contains(&'E'));
        assert!(accented_variants('E').contains(&'É'));
        assert!(!accented_variants('o').contains(&'ø'), "ø non si decompone");
        assert!(accented_variants('1').is_empty());
    }

    #[test]
    fn decomposition_only_when_different() {
        assert_eq!(canonical_decomposition('a'), None);
        assert_eq!(canonical_decomposition('à').as_deref(), Some("a\u{0300}"));
        assert_eq!(
            canonical_decomposition('ǘ').as_deref(),
            Some("u\u{0308}\u{0301}")
        );
    }
}
