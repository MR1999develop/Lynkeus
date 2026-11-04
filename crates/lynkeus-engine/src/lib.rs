// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Motore di ricerca di Lynkeus Searcher: scansione delle radici, filtri, lettura dei file,
//! ricerca in background con annulla, avanzamento ed eventi.
//!
//! Uso:
//!
//! 1. si prepara una [`SearchRequest`] (radici, query compilata, [`Filters`], [`Limits`]);
//! 2. [`start_search`] avvia un thread con nome e restituisce un [`SearchHandle`];
//! 3. il chiamante svuota gli eventi con [`SearchHandle::try_recv`] quando la funzione
//!    [`Notify`] lo avvisa, e legge [`SearchHandle::progress`] quando vuole;
//! 4. l'ultimo evento è sempre [`SearchEvent::Finished`], salvo panic del worker
//!    ([`Received::WorkerLost`]).
//!
//! La scansione è parallela e legge i file di testo a blocchi (vedi `worker` e
//! `textfile`); la fase 4 aggiunge i documenti nel processo estrattore e la cache.

#![forbid(unsafe_code)]

mod doc;
mod event;
mod filter;
mod handle;
mod platform;
mod progress;
mod request;
mod roots;
mod textfile;
mod worker;

pub use doc::DocPath;
pub use event::{Hit, Outcome, SearchEvent, Summary, Warning, WarningKind};
pub use handle::{Notify, Received, SearchHandle, start_search};
pub use progress::{Progress, ProgressSnapshot};
pub use request::{
    Extension, ExtensionError, ExtensionFilter, Filters, HiddenPolicy, Limits, OnlineFilesPolicy,
    SearchRequest,
};

pub use lynkeus_extract::{ArchiveLimits, LimitKind};
