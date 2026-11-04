// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Punti d'ingresso per i target di `fuzz/` (feature `fuzzing`, fuori dall'API normale):
//! i parser propri sui loro byte, senza il file composto né il contesto d'estrazione.
//!
//! Ogni funzione accetta byte qualsiasi e scarta il risultato: l'unica proprietà cercata è
//! che non vada mai in panic né in ciclo. Il frame del protocollo si prova con l'API
//! pubblica di [`protocol`](crate::protocol).

/// Piece table di un .doc: i primi 4 byte (little-endian) sono la lunghezza del flusso
/// `WordDocument`, il resto è la tabella (`0Table`/`1Table`).
pub fn doc_piece_table(data: &[u8]) {
    crate::extract::word97::fuzz(data);
}

/// Record di un .ppt: `data` è il flusso `PowerPoint Document`, preceduto da un record
/// `Current User` se il primo record è di quel tipo.
pub fn ppt_records(data: &[u8]) {
    crate::extract::ppt::fuzz(data);
}

/// Tokenizer RTF.
pub fn rtf(data: &[u8]) {
    crate::extract::rtf::fuzz(data);
}
