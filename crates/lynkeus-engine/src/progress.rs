// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Avanzamento di una ricerca, letto dall'interfaccia senza mai bloccarsi.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, TryLockError};

use crate::doc::DocPath;

/// Contatori e file corrente, scritti dal worker e letti da chiunque.
///
/// I contatori sono atomici `Relaxed`: ognuno è esatto, ma una lettura può vedere
/// contatori diversi in istanti leggermente diversi (va bene per una barra di stato).
/// Il file corrente si aggiorna e si legge con `try_lock`: nessuno dei due lati aspetta.
#[derive(Debug, Default)]
pub struct Progress {
    files_seen: AtomicU64,
    files_searched: AtomicU64,
    files_skipped: AtomicU64,
    binary_skipped: AtomicU64,
    hits: AtomicU64,
    warnings: AtomicU64,
    current: Mutex<Option<Arc<DocPath>>>,
}

/// Copia dei contatori in un istante.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProgressSnapshot {
    /// File incontrati nella scansione.
    pub files_seen: u64,
    /// File letti e cercati.
    pub files_searched: u64,
    /// File esclusi dai filtri (nascosti, estensione, dimensione, solo online).
    pub files_skipped: u64,
    /// File saltati perché binari.
    pub binary_skipped: u64,
    /// Righe di risultato inviate.
    pub hits: u64,
    /// Avvisi inviati.
    pub warnings: u64,
}

/// Un contatore di [`Progress`] (uso interno del crate).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Counter {
    FilesSeen,
    FilesSearched,
    FilesSkipped,
    BinarySkipped,
    Hits,
    Warnings,
}

impl Progress {
    /// I contatori in questo istante.
    pub fn snapshot(&self) -> ProgressSnapshot {
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        ProgressSnapshot {
            files_seen: load(&self.files_seen),
            files_searched: load(&self.files_searched),
            files_skipped: load(&self.files_skipped),
            binary_skipped: load(&self.binary_skipped),
            hits: load(&self.hits),
            warnings: load(&self.warnings),
        }
    }

    /// Il documento in lettura; `None` se nessuno, o se proprio ora il worker lo sta
    /// cambiando (chi mostra lo stato tiene l'ultimo valore visto).
    pub fn current_file(&self) -> Option<Arc<DocPath>> {
        match self.current.try_lock() {
            Ok(guard) => guard.clone(),
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner().clone(),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    pub(crate) fn add(&self, counter: Counter, amount: u64) {
        let target = match counter {
            Counter::FilesSeen => &self.files_seen,
            Counter::FilesSearched => &self.files_searched,
            Counter::FilesSkipped => &self.files_skipped,
            Counter::BinarySkipped => &self.binary_skipped,
            Counter::Hits => &self.hits,
            Counter::Warnings => &self.warnings,
        };
        target.fetch_add(amount, Ordering::Relaxed);
    }

    /// Aggiorna il file corrente; se il lucchetto è occupato salta l'aggiornamento.
    pub(crate) fn set_current(&self, doc: Option<Arc<DocPath>>) {
        match self.current.try_lock() {
            Ok(mut guard) => *guard = doc,
            Err(TryLockError::Poisoned(poisoned)) => *poisoned.into_inner() = doc,
            Err(TryLockError::WouldBlock) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_and_current_file() {
        let progress = Progress::default();
        progress.add(Counter::Hits, 3);
        progress.add(Counter::FilesSeen, 1);
        let snap = progress.snapshot();
        assert_eq!((snap.hits, snap.files_seen, snap.warnings), (3, 1, 0));
        assert_eq!(progress.current_file(), None);
        let doc = Arc::new(DocPath::file("a.txt"));
        progress.set_current(Some(doc.clone()));
        assert_eq!(progress.current_file(), Some(doc));
    }
}
