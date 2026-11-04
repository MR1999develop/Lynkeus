// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Tokenizer RTF. Corpus iniziale: `fixtures/formati/lettera.rtf`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    lynkeus_extract::fuzzing::rtf(data);
});
