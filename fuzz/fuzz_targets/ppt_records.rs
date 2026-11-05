// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Record del flusso `PowerPoint Document` (con un `Current User` facoltativo in testa).
//! Oracoli (in `lynkeus_extract::fuzzing`): due letture danno le stesse diapositive, con
//! numeri crescenti da 1 e testi senza caratteri di controllo. Corpus iniziale: i flussi di
//! `fixtures/formati/vecchia-presentazione.ppt`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    lynkeus_extract::fuzzing::ppt_records(data);
});
