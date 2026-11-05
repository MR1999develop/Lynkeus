// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Nomi che vengono dai file (percorsi, voci d'archivio, fogli) resi sicuri da mostrare.
//!
//! Un nome si mostra, non si interpreta: i caratteri che potrebbero ingannare chi lo legge
//! diventano U+FFFD. È l'unico posto che decide quali sono; lo usano le etichette delle
//! voci (processo estrattore) e i percorsi mostrati (motore).

/// `c`, o U+FFFD se mostrato potrebbe ingannare:
///
/// - caratteri di controllo (categoria `Cc`), compresi gli a capo;
/// - separatori di riga e di paragrafo (U+2028, U+2029), che spezzano la riga;
/// - spazio a larghezza zero (U+200B) e BOM (U+FEFF), invisibili: due nomi diversi
///   sembrerebbero uguali;
/// - caratteri che cambiano la direzione del testo: incorporamenti, override e isolamenti
///   bidi (U+202A–U+202E, U+2066–U+2069) e i segni LRM, RLM e ALM. Un nome come
///   "fattura\u{202E}fdp.exe" apparirebbe altrimenti come "fatturaexe.pdf".
///
/// Restano i separatori a larghezza zero che uniscono o separano (ZWNJ U+200C, ZWJ U+200D):
/// servono alla grafia di lingue come il persiano e alle sequenze di emoji, e non spostano
/// né nascondono l'estensione.
pub fn neutralize_char(c: char) -> char {
    let deceptive = matches!(
        c,
        '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{061C}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{200B}'
            | '\u{FEFF}'
    );
    if c.is_control() || deceptive {
        char::REPLACEMENT_CHARACTER
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(name: &str) -> String {
        name.chars().map(neutralize_char).collect()
    }

    #[test]
    fn labels_neutralize_separators_and_zero_width() {
        assert_eq!(shown("a\u{2028}b\u{2029}c"), "a\u{FFFD}b\u{FFFD}c");
        assert_eq!(shown("fat\u{200B}tura"), "fat\u{FFFD}tura");
        assert_eq!(shown("\u{FEFF}nota.txt"), "\u{FFFD}nota.txt");
    }

    #[test]
    fn control_and_bidi_are_neutralized() {
        assert_eq!(shown("fattura\u{202E}fdp.exe"), "fattura\u{FFFD}fdp.exe");
        assert_eq!(shown("a\u{0}b\nc\u{7}"), "a\u{FFFD}b\u{FFFD}c\u{FFFD}");
        for c in [
            '\u{202A}', '\u{2066}', '\u{2069}', '\u{200E}', '\u{200F}', '\u{061C}',
        ] {
            assert_eq!(neutralize_char(c), '\u{FFFD}', "{c:?}");
        }
    }

    #[test]
    fn ordinary_text_is_kept() {
        for name in [
            "città nuove/perché.txt",
            "../../evil.txt",
            "می\u{200C}خواهم",
            "👨\u{200D}👩",
        ] {
            assert_eq!(shown(name), name);
        }
    }
}
