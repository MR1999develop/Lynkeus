// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Differenze tra sistemi operativi (apertura dei file, limite di memoria e padre del
//! processo estrattore); nessun altro modulo usa `cfg`.

use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

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

/// Mette `parent_gone` a vero quando il processo che ha avviato questo termina.
///
/// Linux: un thread legge il padre in `/proc/self/stat` ogni [`PARENT_CHECK_INTERVAL`];
/// quando cambia (il padre è terminato e il processo è passato a `init` o a un
/// sottomietitore) imposta il segnale ed esce. Il segnale lo legge chi serve le richieste.
/// Non si usano `PR_SET_PDEATHSIG` né un `pidfd`: il primo partirebbe alla fine del *thread*
/// che ha avviato il processo (il motore li avvia da thread che finiscono con la ricerca,
/// mentre il processo resta nel pool), e in questo crate nessun modulo di gestione dei
/// processi è ammesso (controllo `nospawn` di `xtask`). Windows: non fa nulla, il Job
/// Object del padre (`KILL_ON_JOB_CLOSE`) termina l'estrattore. Altri sistemi: non fa nulla.
///
/// Errore se il padre non si legge o il thread non parte.
pub fn watch_parent(parent_gone: Arc<AtomicBool>) -> io::Result<()> {
    imp::watch_parent(parent_gone)
}

/// Intervallo dei controlli del padre.
#[cfg(any(target_os = "linux", test))]
const PARENT_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// Il pid del padre dalla riga di `/proc/<pid>/stat` ("pid (nome) stato ppid …"): il nome
/// può contenere spazi e parentesi, quindi i campi si contano dopo l'ultima `)`.
#[cfg(any(target_os = "linux", test))]
fn parent_from_stat(stat: &str) -> Option<u32> {
    let (_, fields) = stat.rsplit_once(')')?;
    let mut fields = fields.split_whitespace();
    let _state = fields.next()?;
    fields.next()?.parse().ok()
}

#[cfg(windows)]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

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

    #[expect(
        clippy::unnecessary_wraps,
        reason = "stessa firma della versione Linux, che può fallire"
    )]
    pub fn watch_parent(_parent_gone: Arc<AtomicBool>) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use std::fs::File;
    use std::io;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use super::MemoryLimitOwner;

    pub fn open_for_reading(path: &Path) -> io::Result<File> {
        File::open(path)
    }

    pub fn limit_address_space(bytes: u64) -> io::Result<MemoryLimitOwner> {
        rlimit::Resource::AS.set(bytes, bytes)?;
        Ok(MemoryLimitOwner::SelfImposed)
    }

    #[cfg(target_os = "linux")]
    pub fn watch_parent(parent_gone: Arc<AtomicBool>) -> io::Result<()> {
        use std::sync::atomic::Ordering;
        use std::thread;

        use super::PARENT_CHECK_INTERVAL;

        /// Pila del thread di sorveglianza: legge un file corto.
        const WATCH_STACK_BYTES: usize = 64 * 1024;

        let parent = current_parent()?;
        thread::Builder::new()
            .name("lynkeus-padre".to_owned())
            .stack_size(WATCH_STACK_BYTES)
            .spawn(move || {
                // Un padre che non si legge più conta come cambiato: meglio fermarsi.
                while current_parent().ok() == Some(parent) {
                    thread::sleep(PARENT_CHECK_INTERVAL);
                }
                parent_gone.store(true, Ordering::Relaxed);
            })?;
        Ok(())
    }

    /// Il pid del padre di questo processo, da `/proc/self/stat`.
    #[cfg(target_os = "linux")]
    fn current_parent() -> io::Result<u32> {
        let stat = std::fs::read_to_string("/proc/self/stat")?;
        super::parent_from_stat(&stat)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "/proc/self/stat"))
    }

    #[cfg(not(target_os = "linux"))]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "stessa firma della versione Linux, che può fallire"
    )]
    pub fn watch_parent(_parent_gone: Arc<AtomicBool>) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_from_stat_partitions() {
        assert_eq!(
            parent_from_stat("1234 (lynkeus) S 99 1234 1234 0"),
            Some(99)
        );
        // Nome con spazi e parentesi.
        assert_eq!(parent_from_stat("7 (a) b (c)) R 4321 7 7"), Some(4321));
        assert_eq!(parent_from_stat("7 (troncato"), None);
        assert_eq!(parent_from_stat("7 (x) S"), None);
        assert_eq!(parent_from_stat("7 (x) S -"), None);
        assert!(PARENT_CHECK_INTERVAL.as_millis() > 0);
    }
}
