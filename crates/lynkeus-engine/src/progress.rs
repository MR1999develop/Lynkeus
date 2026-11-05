// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Avanzamento di una ricerca, letto dall'interfaccia senza mai bloccarsi.
//!
//! Il totale dei file non si conosce in anticipo: la scansione delle cartelle li conta
//! mentre i lettori li esaminano. Quando la scansione finisce (senza arresti) il totale
//! diventa noto ([`ProgressSnapshot::files_total`]) e chi mostra l'avanzamento può passare
//! da una barra indeterminata a una frazione con il tempo restante. Restano allora da
//! leggere solo i documenti già in coda o in lettura nei processi estrattori.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, TryLockError};

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
    files_total: OnceLock<u64>,
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
    /// File da esaminare in tutto: `Some` da quando la scansione delle cartelle è finita
    /// senza arresti (da lì è uguale a `files_seen`), `None` prima, o se la ricerca si è
    /// fermata durante la scansione (annullata, limite dei risultati).
    pub files_total: Option<u64>,
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
            files_total: self.files_total.get().copied(),
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

    /// La scansione delle cartelle è finita senza arresti: `files_seen` è il totale. Da
    /// chiamare quando tutti i thread della scansione sono finiti (i loro conteggi sono
    /// visibili a chi chiama); una seconda chiamata non cambia il totale.
    pub(crate) fn finish_scan(&self) {
        let total = self.files_seen.load(Ordering::Relaxed);
        // Solo la prima chiamata conta: il totale non cambia più.
        let _first = self.files_total.set(total);
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

    /// Il totale è ignoto finché la scansione non finisce, poi resta quello di allora.
    #[test]
    fn the_total_is_known_once_the_scan_ends() {
        let progress = Progress::default();
        progress.add(Counter::FilesSeen, 2);
        assert_eq!(progress.snapshot().files_total, None);
        progress.finish_scan();
        progress.add(Counter::FilesSearched, 2);
        assert_eq!(progress.snapshot().files_total, Some(2));
        progress.add(Counter::FilesSeen, 1);
        progress.finish_scan();
        assert_eq!(progress.snapshot().files_total, Some(2));
    }
}
