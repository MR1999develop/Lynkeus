// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Posizione di un'unità di testo dentro un documento, una variante per famiglia di formati.
//!
//! Tutti i numeri sono 1-based, come li legge l'utente. Il testo mostrato ("riga 12",
//! "Foglio1!B3") lo compone l'interfaccia con Fluent.

use serde::{Deserialize, Serialize};

/// Parte di un messaggio e-mail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MailPart {
    /// Oggetto.
    Subject,
    /// Mittente.
    From,
    /// Destinatari.
    To,
    /// Corpo del messaggio.
    Body,
}

/// Dove si trova un'unità di testo.
///
/// L'ordine derivato (variante, poi campi) è quello di presentazione dentro un file;
/// per i fogli di calcolo i fogli sono in ordine di nome.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Location {
    /// File di testo: riga.
    Line(u32),
    /// PDF: pagina e riga nella pagina.
    PageLine {
        /// Pagina, 1-based.
        page: u32,
        /// Riga nella pagina, 1-based.
        line: u32,
    },
    /// Fogli di calcolo: foglio e cella.
    Cell {
        /// Nome del foglio, come nel file (etichetta non fidata).
        sheet: String,
        /// Riga, 1-based.
        row: u32,
        /// Colonna, 1-based (1 = A); vedi [`column_name`].
        col: u32,
    },
    /// Presentazioni: diapositiva.
    Slide(u32),
    /// Word, ODT, RTF, EPUB: paragrafo.
    Paragraph(u32),
    /// E-mail: parte del messaggio e riga dentro la parte.
    Mail {
        /// Oggetto, mittente, destinatari o corpo.
        part: MailPart,
        /// Riga nella parte, 1-based.
        line: u32,
    },
}

/// Nome della colonna di un foglio di calcolo: 1 → "A", 26 → "Z", 27 → "AA".
/// Restituisce una stringa vuota per 0, che non è una colonna.
pub fn column_name(col: u32) -> String {
    let mut letters = Vec::new();
    let mut n = col;
    while n > 0 {
        let rem = (n - 1) % 26;
        // `rem` < 26: la conversione non perde nulla.
        letters.push(char::from(b'A' + u8::try_from(rem).unwrap_or(0)));
        n = (n - 1) / 26;
    }
    letters.iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_names() {
        assert_eq!(column_name(0), "");
        assert_eq!(column_name(1), "A");
        assert_eq!(column_name(26), "Z");
        assert_eq!(column_name(27), "AA");
        assert_eq!(column_name(702), "ZZ");
        assert_eq!(column_name(703), "AAA");
        assert_eq!(column_name(u32::MAX), "MWLQKWU");
    }

    #[test]
    fn ordering_inside_a_file() {
        assert!(Location::Line(2) < Location::Line(10));
        assert!(Location::PageLine { page: 1, line: 9 } < Location::PageLine { page: 2, line: 1 });
    }
}
