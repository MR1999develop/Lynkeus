// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! I passi della ricerca in un testo: pezzi di al massimo [`STEP_MAX_LINES`] righe e
//! [`STEP_MAX_BYTES`] byte, tagliati a fine riga; una riga più lunga si taglia dopo uno
//! spazio ([`soft_cut`]).
//!
//! Tra un passo e l'altro chi cerca controlla l'annullamento, e
//! [`find_words`](lynkeus_core::find_words) lavora su un passo alla volta: i suoi risultati
//! di un passo sono tutti in memoria insieme, quindi la dimensione del passo limita la
//! memoria di ogni thread che cerca.

use lynkeus_extract::text::soft_cut;

/// Righe massime di un passo, tra due controlli dell'annullamento.
pub(crate) const STEP_MAX_LINES: usize = 10_000;

/// Byte massimi di un passo: limita anche la memoria dei risultati di
/// [`find_words`](lynkeus_core::find_words).
pub(crate) const STEP_MAX_BYTES: usize = 1024 * 1024;

/// Un passo di un testo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Step<'t> {
    /// Il testo del passo, mai vuoto.
    pub text: &'t str,
    /// Gli a capo del testo diviso prima di questo passo: il numero (0-based) della riga in
    /// cui il passo comincia.
    pub lines_before: u64,
}

/// I passi di `text`, in ordine: insieme lo coprono tutto, senza sovrapporsi.
pub(crate) fn steps(text: &str) -> Steps<'_> {
    Steps {
        rest: text,
        lines_before: 0,
    }
}

/// L'iteratore di [`steps`].
#[derive(Debug, Clone)]
pub(crate) struct Steps<'t> {
    rest: &'t str,
    lines_before: u64,
}

impl<'t> Iterator for Steps<'t> {
    type Item = Step<'t>;

    fn next(&mut self) -> Option<Step<'t>> {
        if self.rest.is_empty() {
            return None;
        }
        let (text, rest) = self.rest.split_at(step_len(self.rest));
        let step = Step {
            text,
            lines_before: self.lines_before,
        };
        self.lines_before = self.lines_before.saturating_add(count_newlines(text));
        self.rest = rest;
        Some(step)
    }
}

/// Lunghezza del prossimo passo di `rest`: fino alla fine della riga numero
/// [`STEP_MAX_LINES`], entro [`STEP_MAX_BYTES`]; una riga più lunga si taglia dopo uno
/// spazio. Sempre almeno un carattere se `rest` non è vuoto.
fn step_len(rest: &str) -> usize {
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
    use super::*;

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

    /// I passi coprono il testo in ordine e contano le righe che li precedono.
    #[test]
    fn steps_cover_the_text_and_count_lines() {
        assert_eq!(steps("").count(), 0);
        let text = "x\n".repeat(STEP_MAX_LINES + 5);
        let found: Vec<(usize, u64)> = steps(&text)
            .map(|s| (s.text.len(), s.lines_before))
            .collect();
        let first_lines = u64::try_from(STEP_MAX_LINES).unwrap_or(u64::MAX);
        assert_eq!(found, [(STEP_MAX_LINES * 2, 0), (10, first_lines)]);
    }
}
