// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Differenze tra sistemi operativi (apertura dei file, limite di memoria del processo
//! estrattore); nessun altro modulo usa `cfg`.

use std::fs::File;
use std::io;
use std::path::Path;

/// Apre `path` in sola lettura per una lettura dall'inizio alla fine.
///
/// Su Windows la condivisione resta quella della libreria standard (lettura, scrittura e
/// cancellazione permesse agli altri programmi) e si chiede la lettura sequenziale, così
/// un file letto una volta non scaccia dalla cache i dati degli altri programmi.
pub fn open_for_reading(path: &Path) -> io::Result<File> {
    imp::open_for_reading(path)
}

/// Chi impone il limite di memoria del processo estrattore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryLimitOwner {
    /// Il processo stesso, con `RLIMIT_AS` (Linux e gli altri Unix).
    SelfImposed,
    /// Il processo principale, con il Job Object in cui avvia l'estrattore (Windows).
    Parent,
}

/// Limita lo spazio d'indirizzamento del processo a `bytes` (limite morbido e rigido: poi
/// non si può più alzare). Su Windows non fa nulla: il limite lo impone il Job Object del
/// processo principale.
pub fn limit_address_space(bytes: u64) -> io::Result<MemoryLimitOwner> {
    imp::limit_address_space(bytes)
}

#[cfg(windows)]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;

    use super::MemoryLimitOwner;

    const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;

    pub fn open_for_reading(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_SEQUENTIAL_SCAN)
            .open(path)
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "stessa firma della versione Unix, che può fallire"
    )]
    pub fn limit_address_space(_bytes: u64) -> io::Result<MemoryLimitOwner> {
        Ok(MemoryLimitOwner::Parent)
    }
}

#[cfg(not(windows))]
mod imp {
    use std::fs::File;
    use std::io;
    use std::path::Path;

    use super::MemoryLimitOwner;

    pub fn open_for_reading(path: &Path) -> io::Result<File> {
        File::open(path)
    }

    pub fn limit_address_space(bytes: u64) -> io::Result<MemoryLimitOwner> {
        rlimit::Resource::AS.set(bytes, bytes)?;
        Ok(MemoryLimitOwner::SelfImposed)
    }
}
