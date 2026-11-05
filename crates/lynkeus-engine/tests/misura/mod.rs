// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Strumenti comuni delle misure del livello 3 del piano di test (`docs/piano-di-test.md`),
//! usati dai test `#[ignore]` di misura del motore e dell'app: la riga `MISURA` e la
//! distribuzione dei tempi (`formato.rs`, senza chiamate al sistema) più il picco di
//! memoria del processo, che qui si legge dal sistema.

#![allow(dead_code, reason = "ogni file di misura usa una parte del modulo")]

mod formato;

// Tutto il formato, anche dove un file di misura ne usa solo una parte.
pub use formato::*;

/// Il picco di memoria di questo processo, se il sistema lo dà senza privilegi.
pub fn peak_memory() -> Option<PeakMemory> {
    // Conversione solo per la stampa in MiB: la precisione di f64 basta fino ai petabyte.
    platform::peak_memory_bytes().map(|(bytes, kind)| PeakMemory {
        mib: bytes as f64 / (1024.0 * 1024.0),
        kind,
    })
}

/// Stampa la riga `MISURA` di `id` con la distribuzione, il picco di memoria del processo e
/// i valori in più (`extras`: chiave e valore, senza spazi).
pub fn print_line(id: &str, distribution: &Distribution, extras: &[(&str, String)]) {
    println!("{}", line(id, distribution, peak_memory(), extras));
}

#[cfg(windows)]
mod platform {
    /// Picco della memoria privata impegnata (commit) del processo, in byte.
    pub fn peak_memory_bytes() -> Option<(u64, &'static str)> {
        ffi::peak_private_bytes().map(|bytes| (bytes, "private"))
    }

    /// L'unica chiamata al sistema delle misure, con un'interfaccia safe.
    #[allow(unsafe_code, reason = "K32GetProcessMemoryInfo, una sola chiamata FFI")]
    mod ffi {
        use windows_sys::Win32::System::ProcessStatus::{
            K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;

        /// `PeakPagefileUsage`: il picco della memoria privata impegnata dal processo.
        pub fn peak_private_bytes() -> Option<u64> {
            let size = u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?;
            let mut counters = PROCESS_MEMORY_COUNTERS {
                cb: size,
                ..PROCESS_MEMORY_COUNTERS::default()
            };
            // SAFETY: `GetCurrentProcess` restituisce una pseudo-maniglia sempre valida, che
            // non si chiude; `counters` è una struttura di questo frame, scrivibile e grande
            // `size` byte, lo stesso valore passato come `cb` e come dimensione.
            let ok =
                unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &raw mut counters, size) };
            if ok == 0 {
                return None;
            }
            u64::try_from(counters.PeakPagefileUsage).ok()
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    /// `VmHWM` di `/proc/self/status`: il picco della memoria residente (RSS), in byte.
    pub fn peak_memory_bytes() -> Option<(u64, &'static str)> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
        let kib: u64 = line
            .trim_start_matches("VmHWM:")
            .trim()
            .trim_end_matches("kB")
            .trim()
            .parse()
            .ok()?;
        Some((kib.checked_mul(1024)?, "rss"))
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    /// Nessuna fonte del picco di memoria su questa piattaforma.
    pub fn peak_memory_bytes() -> Option<(u64, &'static str)> {
        None
    }
}
