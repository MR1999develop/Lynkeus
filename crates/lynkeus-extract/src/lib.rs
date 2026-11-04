// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Lettura dei file per Lynkeus Searcher: testo e codifiche oggi; documenti, archivi e
//! processo estrattore isolato nelle fasi 4 e 5.
//!
//! Ogni byte letto qui è input non fidato: nessun panic sui dati dei file, limiti espliciti
//! su dimensioni e risorse, errori classificati in [`ExtractError`] secondo l'avviso che
//! l'utente vedrà.

#![forbid(unsafe_code)]

pub mod error;
pub mod limits;
mod platform;
pub mod text;

pub use error::ExtractError;
pub use limits::{ArchiveLimits, LimitKind};
pub use text::{DecodedText, TextChunk, TextEncoding, TextReader, open_text, read_text};
