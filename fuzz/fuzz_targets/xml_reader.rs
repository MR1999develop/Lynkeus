// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Lettore XML di docx, pptx, ODF ed EPUB, nei dialetti dei pacchetti e dei capitoli XHTML.
//! Oracoli: ogni chiusura corrisponde all'ultima apertura ancora aperta (stesso namespace e
//! nome locale), due letture degli stessi byte danno gli stessi eventi. Corpus iniziale: le
//! parti XML dei file di `fixtures/formati/` (docx, pptx, odt, ods, epub).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    lynkeus_extract::fuzzing::xml_reader(data);
});
