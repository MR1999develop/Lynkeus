// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! FIB, CLX e piece table di un .doc: i primi 4 byte sono la lunghezza del flusso
//! `WordDocument`, il resto la tabella. Oracoli (in `lynkeus_extract::fuzzing`): due letture
//! danno gli stessi paragrafi, ognuno con testo e senza caratteri di controllo, al più un
//! carattere per CP del testo principale. Corpus iniziale: i flussi di
//! `fixtures/formati/vecchia-lettera.doc`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    lynkeus_extract::fuzzing::doc_piece_table(data);
});
