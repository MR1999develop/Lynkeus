// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Frame del protocollo tra processo principale ed estrattore, nei due versi: byte qualsiasi
//! letti come sequenza di richieste e come sequenza di risposte, fino al primo errore.

#![no_main]

use libfuzzer_sys::fuzz_target;
use lynkeus_extract::protocol::{FrameReader, Request, Response};

/// Frame letti al più da un input, per non girare a vuoto su flussi di frame minimi.
const MAX_FRAMES: usize = 1024;

fuzz_target!(|data: &[u8]| {
    let mut requests = FrameReader::new(data);
    for _ in 0..MAX_FRAMES {
        match requests.read::<Request>() {
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }
    let mut responses = FrameReader::new(data);
    for _ in 0..MAX_FRAMES {
        match responses.read::<Response>() {
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }
});
