// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Nucleo puro di Lynkeus Searcher: nessun I/O, nessuna dipendenza da interfaccia o file system.
//!
//! Flusso tipico:
//!
//! 1. [`Term::parse`] valida ogni termine scritto dall'utente;
//! 2. [`Query::new`] li riunisce con le [`MatchOptions`] e la regola [`Combine`];
//! 3. [`Query::compile`] produce una [`CompiledQuery`], condivisibile tra thread;
//! 4. [`find_words`] trova in un testo le parole che contengono i termini ([`WordMatch`]);
//! 5. [`word_text`] e [`snippet`] preparano parola e contesto da mostrare;
//! 6. [`FileAccumulator`] applica OR/AND ai risultati di un file.
//!
//! La semantica completa è in `docs/specifica.md`, sezioni 3–6.

#![forbid(unsafe_code)]

pub mod accumulate;
pub mod location;
pub mod matcher;
mod pattern;
pub mod query;
pub mod size;
mod unicode;
pub mod unit;

pub use accumulate::FileAccumulator;
pub use location::{Location, MailPart, column_name};
pub use matcher::{Snippet, TermSet, WordMatch, WordText, find_words, snippet, word_text};
pub use query::{
    AccentMode, CaseMode, Combine, CompiledQuery, MatchOptions, Query, QueryError, Term, TermError,
    WildcardMode,
};
pub use size::ByteSize;
pub use unit::TextUnit;
