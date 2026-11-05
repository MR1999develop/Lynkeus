// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Strumenti comuni delle misure del livello 3 del piano di test (`docs/piano-di-test.md`),
//! usati dai test `#[ignore]` di misura del motore e dell'app.
//!
//! Metodo del piano: build di rilascio, una esecuzione scartata, poi le ripetizioni; si
//! riportano p50, p95 e massimo (mai la sola media) e il picco di memoria del processo.
//!
//! Ogni misura stampa su stdout una sola riga che `cargo xtask test-plan` raccoglie:
//!
//! ```text
//! MISURA P-01 samples=15 p50_ms=812.345000 p95_ms=901.002000 max_ms=950.100000 mem_mb=123.4 mem_kind=private [chiave=valore …]
//! ```
//!
//! Valori senza spazi; tempi in millisecondi con sei decimali; `mem_mb=not_measured` (e
//! `mem_kind=none`) dove il sistema non dà il picco. Le soglie non stanno qui: le confronta
//! `cargo xtask test-plan` (`xtask/src/plan_map.rs`), così il test misura e basta.

#![allow(dead_code, reason = "ogni file di misura usa una parte del modulo")]

use std::time::{Duration, Instant};

/// Prefisso della riga raccolta da `cargo xtask test-plan`.
pub const LINE_PREFIX: &str = "MISURA";

/// Esegue `run` una volta in più all'inizio (scartata: cache, processi, allocatore a regime)
/// e restituisce i tempi delle `repetitions` esecuzioni successive.
pub fn repeat(repetitions: usize, mut run: impl FnMut() -> Duration) -> Vec<Duration> {
    let _discarded = run();
    (0..repetitions).map(|_| run()).collect()
}

/// Il tempo di `work`, con il suo risultato.
pub fn timed<T>(work: impl FnOnce() -> T) -> (T, Duration) {
    let started = Instant::now();
    let value = work();
    (value, started.elapsed())
}

/// Distribuzione di una serie di tempi: p50, p95 e massimo con il metodo del rango più
/// vicino (nessuna interpolazione: ogni valore riportato è stato osservato).
#[derive(Debug, Clone)]
pub struct Distribution {
    sorted_ms: Vec<f64>,
}

impl Distribution {
    /// La distribuzione di `samples`; almeno un campione (invariante dei test di misura).
    pub fn new(samples: &[Duration]) -> Distribution {
        assert!(!samples.is_empty(), "nessun campione da riassumere");
        let mut sorted_ms: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
        sorted_ms.sort_by(f64::total_cmp);
        Distribution { sorted_ms }
    }

    /// Numero di campioni.
    pub fn len(&self) -> usize {
        self.sorted_ms.len()
    }

    /// Il percentile `q` (da 1 a 100) con il rango più vicino: il campione di posto
    /// `ceil(q/100 · n)`.
    pub fn percentile_ms(&self, q: usize) -> f64 {
        let n = self.sorted_ms.len();
        let rank = (q.clamp(1, 100) * n).div_ceil(100).clamp(1, n);
        self.sorted_ms[rank - 1]
    }

    pub fn p50_ms(&self) -> f64 {
        self.percentile_ms(50)
    }

    pub fn p95_ms(&self) -> f64 {
        self.percentile_ms(95)
    }

    pub fn max_ms(&self) -> f64 {
        self.percentile_ms(100)
    }
}

/// Il rango più vicino su 20 campioni da 1 a 20 ms (in disordine) e su un solo campione.
#[test]
fn nearest_rank_percentiles() {
    let samples: Vec<Duration> = (1..=20).rev().map(Duration::from_millis).collect();
    let distribution = Distribution::new(&samples);
    let at = |q| distribution.percentile_ms(q).round();
    assert_eq!((at(50), at(95), at(100)), (10.0, 19.0, 20.0));
    assert_eq!(at(0), 1.0, "sotto 1 vale come 1");
    let one = Distribution::new(&[Duration::from_millis(7)]);
    let rounded = [one.p50_ms(), one.p95_ms(), one.max_ms()].map(f64::round);
    assert_eq!(rounded, [7.0, 7.0, 7.0]);
}

/// Picco di memoria del processo dall'avvio.
#[derive(Debug, Clone, Copy)]
pub struct PeakMemory {
    /// Mebibyte.
    pub mib: f64,
    /// `private` (Windows: memoria privata impegnata) o `rss` (Linux: VmHWM).
    pub kind: &'static str,
}

/// Il picco di memoria di questo processo, se il sistema lo dà senza privilegi.
pub fn peak_memory() -> Option<PeakMemory> {
    // Conversione solo per la stampa in MiB: la precisione di f64 basta fino ai petabyte.
    platform::peak_memory_bytes().map(|(bytes, kind)| PeakMemory {
        mib: bytes as f64 / (1024.0 * 1024.0),
        kind,
    })
}

/// Un millisecondo in testo per la riga di misura.
pub fn ms(value: f64) -> String {
    format!("{value:.6}")
}

/// La durata in testo per la riga di misura (millisecondi).
pub fn duration_ms(value: Duration) -> String {
    ms(value.as_secs_f64() * 1000.0)
}

/// Stampa la riga `MISURA` di `id` con la distribuzione, il picco di memoria del processo e
/// i valori in più (`extras`: chiave e valore, senza spazi).
pub fn print_line(id: &str, distribution: &Distribution, extras: &[(&str, String)]) {
    let memory = peak_memory().map_or_else(
        || ("not_measured".to_owned(), "none"),
        |peak| (format!("{:.1}", peak.mib), peak.kind),
    );
    let mut line = format!(
        "{LINE_PREFIX} {id} samples={} p50_ms={} p95_ms={} max_ms={} mem_mb={} mem_kind={}",
        distribution.len(),
        ms(distribution.p50_ms()),
        ms(distribution.p95_ms()),
        ms(distribution.max_ms()),
        memory.0,
        memory.1,
    );
    for (key, value) in extras {
        assert!(
            !key.contains([' ', '=']) && !value.contains(char::is_whitespace),
            "chiave o valore con spazi: {key}={value}"
        );
        line.push_str(&format!(" {key}={value}"));
    }
    println!("{line}");
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
