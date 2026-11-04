// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Differenze tra sistemi operativi nell'apertura dei file; nessun altro modulo usa `cfg`.

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

#[cfg(windows)]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;

    const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;

    pub fn open_for_reading(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_SEQUENTIAL_SCAN)
            .open(path)
    }
}

#[cfg(not(windows))]
mod imp {
    use std::fs::File;
    use std::io;
    use std::path::Path;

    pub fn open_for_reading(path: &Path) -> io::Result<File> {
        File::open(path)
    }
}
