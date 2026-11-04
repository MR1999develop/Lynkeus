// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Traduzione di un termine nella sintassi del crate `regex`.
//!
//! Regole (specifica, sezione 4; `references/ricerca.md`):
//!
//! - il termine è prima portato in NFC; i letterali sono escapati;
//! - jolly attivi: `?` → `[^\s\p{Mn}]\p{Mn}*`, un carattere non spazio con i suoi
//!   eventuali segni combinanti; `*` → `(?:[^\s\p{Mn}]\p{Mn}*)*` (greedy), zero o più di
//!   questi caratteri. Equivalgono a `\S` e `\S*` su testo NFC, ma su testo NFD non
//!   separano una lettera dai suoi segni: "e*" non trova la "é" decomposta, come non trova
//!   quella precomposta;
//!   più `*` di fila valgono uno solo; jolly spenti: `*` e `?` sono letterali;
//! - ogni sequenza di spazi → `[^\S\r\n]+` (spazi orizzontali, mai un a capo);
//! - accenti distinti: una lettera con decomposizione canonica accetta la forma precomposta
//!   o quella NFD (`(?:à|a\x{300})`);
//! - accenti ignorati: il termine è decomposto (NFD) e i segni `Mn` sono tolti; ogni
//!   carattere diventa la classe di sé e delle sue varianti accentate, seguita da `\p{Mn}*`;
//!   con gli accenti distinti il pattern finisce con la guardia `(?P<next>[^\p{Mn}]|\z)`:
//!   la corrispondenza non può essere seguita da un segno combinante (nel testo NFD
//!   "citta" + `◌̀` è "città", che "citta" non deve trovare). Il crate `regex` non ha il
//!   lookahead, quindi la guardia consuma il carattere seguente e il chiamante taglia la
//!   corrispondenza all'inizio del gruppo `next`; dentro il termine non serve, perché
//!   nessun pezzo successivo può iniziare con un segno combinante;
//! - un `*` iniziale o finale è un gruppo con nome (`lead`, `trail`), perché il chiamante
//!   possa togliere la punteggiatura di bordo che il jolly ha inglobato.
//!
//! Le maiuscole non sono affare di questo modulo: le gestisce il flag del `RegexBuilder`.

use unicode_normalization::UnicodeNormalization;

use crate::query::{AccentMode, MatchOptions, WildcardMode};
use crate::unicode::{accented_variants, canonical_decomposition, is_nonspacing_mark};

/// Nome del gruppo del `*` iniziale.
pub(crate) const LEAD_GROUP: &str = "lead";
/// Nome del gruppo del `*` finale.
pub(crate) const TRAIL_GROUP: &str = "trail";
/// Nome del gruppo della guardia finale (accenti distinti).
pub(crate) const GUARD_GROUP: &str = "next";
/// La guardia finale: un carattere che non è un segno combinante, o la fine del testo.
/// Il nome del gruppo è [`GUARD_GROUP`].
const GUARD: &str = r"(?P<next>[^\p{Mn}]|\z)";

/// Spazi orizzontali: uno spazio Unicode che non è un a capo.
const HORIZONTAL_SPACES: &str = r"[^\S\r\n]+";
/// Segni combinanti non spaziati, zero o più.
const MARKS: &str = r"\p{Mn}*";
/// Il jolly `?`: un carattere che non è uno spazio né un segno combinante, con i suoi segni.
/// Un `\S` semplice potrebbe prendere da solo il segno di una lettera NFD.
const ONE_CHARACTER: &str = r"[^\s\p{Mn}]\p{Mn}*";
/// Il jolly `*`: zero o più caratteri come quelli di [`ONE_CHARACTER`].
const ANY_CHARACTERS: &str = r"(?:[^\s\p{Mn}]\p{Mn}*)*";

/// Un termine tradotto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TermPattern {
    /// Il pattern per `RegexBuilder` (senza flag delle maiuscole).
    pub(crate) pattern: String,
    /// Il termine inizia con `*` (jolly attivi): c'è il gruppo [`LEAD_GROUP`].
    pub(crate) lead_star: bool,
    /// Il termine finisce con `*` (jolly attivi): c'è il gruppo [`TRAIL_GROUP`].
    pub(crate) trail_star: bool,
    /// Accenti distinti: il pattern finisce con la guardia [`GUARD_GROUP`].
    pub(crate) guard: bool,
}

/// Pezzo di un termine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    Space,
    Star,
    Question,
    Literal(char),
}

/// Traduce `raw` (un termine già validato) con le opzioni date.
pub(crate) fn term_pattern(raw: &str, options: MatchOptions) -> TermPattern {
    let tokens = tokenize(raw, options);
    let lead_star = tokens.len() > 1 && tokens.first() == Some(&Token::Star);
    let trail_star = tokens.len() > 1 && tokens.last() == Some(&Token::Star);
    let last = tokens.len().saturating_sub(1);
    let mut pattern = String::with_capacity(raw.len() * 4);
    for (index, token) in tokens.iter().enumerate() {
        match *token {
            Token::Space => pattern.push_str(HORIZONTAL_SPACES),
            Token::Star if index == 0 && lead_star => push_star_group(&mut pattern, LEAD_GROUP),
            Token::Star if index == last && trail_star => {
                push_star_group(&mut pattern, TRAIL_GROUP);
            }
            Token::Star => pattern.push_str(ANY_CHARACTERS),
            Token::Question => pattern.push_str(ONE_CHARACTER),
            Token::Literal(c) => push_literal(&mut pattern, c, options.accents),
        }
    }
    let guard = options.accents == AccentMode::Distinct;
    if guard {
        pattern.push_str(GUARD);
    }
    TermPattern {
        pattern,
        lead_star,
        trail_star,
        guard,
    }
}

/// Il termine in NFC (NFD senza segni `Mn` con gli accenti ignorati), diviso in pezzi;
/// spazi e `*` consecutivi diventano un pezzo solo.
fn tokenize(raw: &str, options: MatchOptions) -> Vec<Token> {
    let normalized: String = match options.accents {
        AccentMode::Distinct => raw.nfc().collect(),
        AccentMode::Ignored => raw.nfd().filter(|&c| !is_nonspacing_mark(c)).collect(),
    };
    let wildcards = options.wildcards == WildcardMode::Enabled;
    let mut tokens: Vec<Token> = Vec::with_capacity(normalized.len());
    for c in normalized.chars() {
        let token = match c {
            c if c.is_whitespace() => Token::Space,
            '*' if wildcards => Token::Star,
            '?' if wildcards => Token::Question,
            c => Token::Literal(c),
        };
        let repeated = matches!(token, Token::Space | Token::Star) && tokens.last() == Some(&token);
        if !repeated {
            tokens.push(token);
        }
    }
    tokens
}

fn push_literal(pattern: &mut String, c: char, accents: AccentMode) {
    match accents {
        AccentMode::Distinct => match canonical_decomposition(c) {
            Some(decomposed) => {
                pattern.push_str("(?:");
                push_escaped(pattern, c);
                pattern.push('|');
                pattern.push_str(&regex::escape(&decomposed));
                pattern.push(')');
            }
            None => push_escaped(pattern, c),
        },
        AccentMode::Ignored => {
            let variants = accented_variants(c);
            if variants.is_empty() {
                push_escaped(pattern, c);
            } else {
                pattern.push('[');
                push_escaped(pattern, c);
                for &variant in variants {
                    push_escaped(pattern, variant);
                }
                pattern.push(']');
            }
            pattern.push_str(MARKS);
        }
    }
}

/// `*` come gruppo con nome: `(?P<name>…)`.
fn push_star_group(pattern: &mut String, name: &str) {
    pattern.push_str("(?P<");
    pattern.push_str(name);
    pattern.push('>');
    pattern.push_str(ANY_CHARACTERS);
    pattern.push(')');
}

fn push_escaped(pattern: &mut String, c: char) {
    let mut buf = [0u8; 4];
    pattern.push_str(&regex::escape(c.encode_utf8(&mut buf)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(accents: AccentMode, wildcards: WildcardMode) -> MatchOptions {
        MatchOptions {
            accents,
            wildcards,
            ..MatchOptions::default()
        }
    }

    /// Pattern con le opzioni predefinite (accenti distinti), senza la guardia finale.
    fn pattern(raw: &str) -> String {
        let p = term_pattern(raw, MatchOptions::default());
        assert!(p.guard);
        p.pattern
            .strip_suffix(GUARD)
            .expect("guardia finale con gli accenti distinti")
            .to_owned()
    }

    #[test]
    fn guard_group_name_matches() {
        assert!(GUARD.starts_with(&format!("(?P<{GUARD_GROUP}>")));
        let ignored = term_pattern("a", options(AccentMode::Ignored, WildcardMode::Enabled));
        assert!(!ignored.guard && !ignored.pattern.contains(GUARD));
    }

    #[test]
    fn literal_is_escaped() {
        assert_eq!(pattern("a.b"), r"a\.b");
        assert_eq!(pattern("(x)"), r"\(x\)");
    }

    #[test]
    fn wildcards_enabled() {
        assert_eq!(
            pattern("c?rt*"),
            format!("c{ONE_CHARACTER}rt(?P<trail>{ANY_CHARACTERS})")
        );
        let p = term_pattern("c?rt*", MatchOptions::default());
        assert!(p.trail_star && !p.lead_star);
        assert_eq!(
            pattern("**a*b**"),
            format!("(?P<lead>{ANY_CHARACTERS})a{ANY_CHARACTERS}b(?P<trail>{ANY_CHARACTERS})")
        );
    }

    #[test]
    fn wildcards_literal() {
        let p = term_pattern("a*?", options(AccentMode::Distinct, WildcardMode::Literal));
        assert_eq!(p.pattern, format!(r"a\*\?{GUARD}"));
        assert!(!p.lead_star && !p.trail_star);
    }

    #[test]
    fn spaces_collapse_to_horizontal_spaces() {
        assert_eq!(pattern("la \t car"), r"la[^\S\r\n]+car");
    }

    #[test]
    fn distinct_accents_accept_nfd() {
        assert_eq!(pattern("tà"), "t(?:à|a\u{300})");
        // Il termine scritto in NFD è portato in NFC.
        assert_eq!(pattern("ta\u{300}"), "t(?:à|a\u{300})");
    }

    #[test]
    fn ignored_accents_use_variant_classes() {
        let p = term_pattern("tà", options(AccentMode::Ignored, WildcardMode::Enabled)).pattern;
        assert!(p.starts_with("[tţť"), "{p}");
        assert!(p.contains("[aàáâ") && p.ends_with(r"]\p{Mn}*"), "{p}");
        // Senza varianti: il carattere escapato, seguito comunque dai segni combinanti.
        let p = term_pattern("1.", options(AccentMode::Ignored, WildcardMode::Enabled)).pattern;
        assert_eq!(p, r"1\p{Mn}*\.\p{Mn}*");
    }
}
