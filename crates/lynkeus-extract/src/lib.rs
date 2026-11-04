// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Lettura dei file per Lynkeus Searcher: testo e codifiche, documenti, archivi e processo
//! estrattore isolato.
//!
//! Ogni byte letto qui è input non fidato: nessun panic sui dati dei file, limiti espliciti
//! su dimensioni e risorse, errori classificati in [`ExtractError`] secondo l'avviso che
//! l'utente vedrà.
//!
//! Moduli:
//!
//! - [`text`]: file di testo (codifiche, binari, lettura a blocchi), usato anche dal processo
//!   principale;
//! - [`format`]: scelta del formato dall'estensione, verifica dal contenuto, registro degli
//!   estrattori (il punto da estendere con nuovi formati);
//! - [`event`]: eventi d'estrazione ([`ExtractEvent`]) e il loro destinatario ([`EventSink`]);
//! - [`extract_path`]: estrazione nello stesso processo (per il processo estrattore e i test);
//! - [`protocol`]: frame e messaggi tra processo principale e processo estrattore;
//! - [`worker`]: il ciclo del processo estrattore.
//!
//! Il processo principale usa di questo crate solo [`text`], [`format`] (per decidere che cosa
//! mandare all'estrattore), [`event`] e [`protocol`]: i decoder dei formati complessi girano
//! solo nel processo estrattore (ADR 0001).

#![forbid(unsafe_code)]

pub mod error;
pub mod event;
mod extract;
pub mod format;
#[cfg(feature = "fuzzing")]
#[doc(hidden)]
pub mod fuzzing;
pub mod limits;
mod platform;
pub mod protocol;
pub mod text;
pub mod worker;

pub use error::ExtractError;
pub use event::{
    EventSink, ExtractEvent, SequenceCheck, SequenceError, SkipReason, Stopped, WarningKind,
    line_location,
};
pub use extract::{ExtractOptions, MAX_UNIT_TEXT_BYTES, extract_bytes, extract_path};
pub use format::{Format, Signature};
pub use limits::{ArchiveLimits, LimitKind};
pub use text::{DecodedText, TextChunk, TextEncoding, TextReader, open_text, read_text};
