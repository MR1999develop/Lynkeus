// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Ricerca di testi sensibili (nome utente, percorsi locali) nei byte di un eseguibile.
//!
//! Ogni testo si cerca in ASCII/UTF-8 e in UTF-16 LE, senza distinguere maiuscole
//! e minuscole ASCII; i percorsi anche con `/` al posto di `\`.

/// Un testo da cercare, con l'etichetta che lo descrive nei messaggi.
#[derive(Debug, Clone)]
pub struct Needle {
    pub label: String,
    pub text: String,
}

/// Una corrispondenza trovata: quale testo, in quale codifica e a quale offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub label: String,
    pub encoding: &'static str,
    pub offset: usize,
}

/// Cerca tutti i `needles` in `haystack`; restituisce la prima corrispondenza per
/// testo e codifica. I testi vuoti si ignorano.
pub fn find_all(haystack: &[u8], needles: &[Needle]) -> Vec<Finding> {
    let lowered = haystack.to_ascii_lowercase();
    let mut findings = Vec::new();
    for needle in needles {
        for variant in variants(&needle.text) {
            let ascii = variant.to_ascii_lowercase().into_bytes();
            let utf16: Vec<u8> = variant
                .to_ascii_lowercase()
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect();
            for (encoding, pattern) in [("ASCII/UTF-8", ascii), ("UTF-16 LE", utf16)] {
                if let Some(offset) = position(&lowered, &pattern) {
                    let finding = Finding {
                        label: needle.label.clone(),
                        encoding,
                        offset,
                    };
                    if !findings.contains(&finding) {
                        findings.push(finding);
                    }
                }
            }
        }
    }
    findings
}

/// Il testo così com'è e, se contiene separatori, con `\` e `/` scambiati.
fn variants(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut list = vec![text.to_owned()];
    for alternative in [text.replace('\\', "/"), text.replace('/', "\\")] {
        if !list.contains(&alternative) {
            list.push(alternative);
        }
    }
    list
}

fn position(haystack: &[u8], pattern: &[u8]) -> Option<usize> {
    if pattern.is_empty() || pattern.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(pattern.len())
        .position(|window| window == pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn needle(text: &str) -> Vec<Needle> {
        vec![Needle {
            label: "test".to_owned(),
            text: text.to_owned(),
        }]
    }

    #[test]
    fn absent_text_gives_no_findings() {
        assert!(find_all(b"nothing here", &needle("someone")).is_empty());
    }

    #[test]
    fn empty_needle_is_ignored() {
        assert!(find_all(b"abc", &needle("")).is_empty());
    }

    #[test]
    fn ascii_match_ignores_case() {
        let found = find_all(b"xx C:\\USERS\\Someone\\x", &needle(r"c:\users\someone"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].encoding, "ASCII/UTF-8");
        assert_eq!(found[0].offset, 3);
    }

    #[test]
    fn forward_slash_variant_of_a_windows_path_is_found() {
        let found = find_all(b"--c:/users/someone/src", &needle(r"C:\Users\Someone"));
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn utf16le_match_ignores_case() {
        let haystack: Vec<u8> = "..SomeOne.."
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let found = find_all(&haystack, &needle("someone"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].encoding, "UTF-16 LE");
        assert_eq!(found[0].offset, 4);
    }

    #[test]
    fn needle_longer_than_haystack_is_not_found() {
        assert!(find_all(b"ab", &needle("abc")).is_empty());
    }
}
