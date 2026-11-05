// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Frame del protocollo tra processo principale ed estrattore, nei due versi: byte qualsiasi
//! letti come sequenza di richieste e come sequenza di risposte, fino al primo errore.
//!
//! Oracolo di andata e ritorno: ogni messaggio letto, scritto di nuovo con `encode_frame`, si
//! rilegge uguale (e la scrittura non fallisce: un messaggio letto sta in un frame).

#![no_main]

use libfuzzer_sys::fuzz_target;
use lynkeus_extract::protocol::{FrameReader, Request, Response, encode_frame};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Frame letti al più da un input, per non girare a vuoto su flussi di frame minimi.
const MAX_FRAMES: usize = 1024;

/// Legge messaggi `T` da `data` fino al primo errore e controlla l'andata e ritorno.
fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(data: &[u8]) {
    let mut reader = FrameReader::new(data);
    for _ in 0..MAX_FRAMES {
        let Ok(Some(message)) = reader.read::<T>() else {
            break;
        };
        let mut frame = Vec::new();
        assert!(
            encode_frame(&message, &mut frame).is_ok(),
            "un messaggio letto non si riscrive"
        );
        let again = FrameReader::new(frame.as_slice()).read::<T>();
        assert!(
            matches!(&again, Ok(Some(back)) if *back == message),
            "andata e ritorno diversa: {message:?} → {again:?}"
        );
    }
}

fuzz_target!(|data: &[u8]| {
    round_trip::<Request>(data);
    round_trip::<Response>(data);
});
