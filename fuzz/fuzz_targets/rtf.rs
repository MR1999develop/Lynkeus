// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Tokenizer RTF. Oracoli (in `lynkeus_extract::fuzzing`): due letture danno gli stessi
//! paragrafi, ognuno con testo e senza caratteri di controllo, al più un carattere per byte
//! dell'input. Corpus iniziale: `fixtures/formati/lettera.rtf`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    lynkeus_extract::fuzzing::rtf(data);
});
