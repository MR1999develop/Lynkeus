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
//! `textfile`). I documenti (PDF, Office, archivi) li legge un [`DocumentSource`]: la
//! [`TextCache`] se ha già il loro testo, altrimenti un [`ExtractorPool`] di processi
//! estrattori avviati da un [`WorkerLauncher`] (portabile: [`CommandLauncher`]; su Windows
//! l'app ne fornisce uno con il Job Object).

#![forbid(unsafe_code)]

mod batch;
mod cache;
mod doc;
mod docsearch;
mod documents;
mod event;
mod filter;
mod halt;
mod handle;
mod launcher;
mod platform;
mod pool;
mod progress;
mod request;
mod roots;
mod step;
mod textfile;
mod worker;

pub use cache::{
    CACHE_FORMAT_VERSION, CacheKey, DEFAULT_CACHE_SIZE, MAX_ENTRY_BYTES, TextCache, WriteFailure,
    WriteFailures,
};
pub use doc::DocPath;
pub use documents::{DocumentSource, Origin};
pub use event::{Hit, Notify, Outcome, SearchEvent, Summary, Warning, WarningKind};
pub use handle::{Received, SearchHandle, start_search};
pub use launcher::{
    CommandLauncher, EXTRACT_WORKER_FLAG, LaunchedWorker, WorkerExit, WorkerLauncher,
    WorkerProcess, worker_args,
};
/// Cartelle e file privati, per la cache e per i file dell'app: il trait [`PrivateFs`]
/// (l'app ne inietta uno con il descrittore esplicito su Windows) e il rifiuto di ogni
/// creazione ([`RefusingFs`]), il default sicuro dove questo crate non sa fare di meglio.
pub use platform::{NotPrivate, PrivateFs, RefusingFs};
/// La versione portabile delle cartelle private, solo fuori da Windows (0700 e 0600).
#[cfg(not(windows))]
pub use platform::{PortablePrivateFs, create_new_private_file, create_private_dir};
pub use pool::{
    EntryEnd, ExtractEnd, ExtractorPool, MAX_POOL_SIZE, STOP_GRACE, WorkerFailure,
    default_pool_size,
};
pub use progress::{Progress, ProgressSnapshot};
pub use request::{
    Extension, ExtensionError, ExtensionFilter, Filters, HiddenPolicy, Limits, OnlineFilesPolicy,
    SearchRequest,
};

pub use lynkeus_extract::{ArchiveLimits, LimitKind};
