// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Punti d'ingresso per i target di `fuzz/` (feature `fuzzing`, fuori dall'API normale):
//! i parser propri sui loro byte, senza il file composto né il contesto d'estrazione.
//!
//! Ogni funzione accetta byte qualsiasi e non deve mai andare in panic né in ciclo; dove c'è
//! un oracolo oltre al crash lo controlla con un'asserzione (che il fuzzer vede come
//! fallimento). Il frame del protocollo, la ricerca e il testo si provano con l'API
//! pubblica dei crate.

/// Piece table di un .doc: i primi 4 byte (little-endian) sono la lunghezza del flusso
/// `WordDocument`, il resto è la tabella (`0Table`/`1Table`). Oracoli: due letture danno gli
/// stessi paragrafi, ognuno con testo e senza caratteri di controllo, al più un carattere per
/// CP del testo principale.
pub fn doc_piece_table(data: &[u8]) {
    crate::extract::word97::fuzz(data);
}

/// Record di un .ppt: `data` è il flusso `PowerPoint Document`, preceduto da un record
/// `Current User` se il primo record è di quel tipo. Oracoli: due letture danno le stesse
/// diapositive, con numeri crescenti da 1 e testi senza caratteri di controllo.
pub fn ppt_records(data: &[u8]) {
    crate::extract::ppt::fuzz(data);
}

/// Tokenizer RTF. Oracoli: due letture danno gli stessi paragrafi, ognuno con testo e senza
/// caratteri di controllo, al più un carattere per byte dell'input.
pub fn rtf(data: &[u8]) {
    crate::extract::rtf::fuzz(data);
}

/// Lettore XML (dialetto dei pacchetti e dei capitoli XHTML). Oracoli: ogni chiusura
/// corrisponde all'ultima apertura, due letture danno gli stessi eventi.
pub fn xml_reader(data: &[u8]) {
    crate::extract::xml::fuzz(data);
}
