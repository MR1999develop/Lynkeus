// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Differenze tra sistemi operativi (apertura dei file, descrittori ereditati, limite di
//! memoria e padre del processo estrattore); nessun altro modulo usa `cfg` né `unsafe`.

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

/// Chiama `on_gone` quando il processo padre `expected` (il pid passato dal processo
/// principale; senza, il padre letto all'avvio) non è più il padre di questo processo.
///
/// Linux: un thread legge il padre in `/proc/self/stat` subito e poi ogni
/// [`PARENT_CHECK_INTERVAL`]; appena è diverso da `expected` (il padre è terminato, anche
/// prima che questo processo iniziasse a sorvegliarlo, e il processo è passato a `init` o a
/// un sottomietitore) chiama `on_gone` ed esce. `on_gone` può terminare il processo: un
/// estrattore occupato in un decodificatore non arriverebbe a controllare un segnale.
///
/// `PR_SET_PDEATHSIG` non si usa. Oggi i processi nascono dal thread del pool, che vive
/// quanto il pool, quindi il segnale partirebbe al momento giusto; ma chiamare `prctl`
/// richiede in questo crate un modulo di gestione dei processi, che il controllo `nospawn` di
/// `xtask` non ammette, e il figlio dovrebbe comunque confrontare il padre con `expected`
/// per il padre morto prima della chiamata. Il controllo periodico copre entrambi i casi entro
/// [`PARENT_CHECK_INTERVAL`]. Windows: non fa nulla, il Job Object del padre
/// (`KILL_ON_JOB_CLOSE`) termina l'estrattore. Altri sistemi: non fa nulla.
///
/// Errore se il padre non si legge o il thread non parte.
pub fn watch_parent(
    expected: Option<u32>,
    on_gone: impl FnOnce() + Send + 'static,
) -> io::Result<()> {
    imp::watch_parent(expected, on_gone)
}

/// Chiude i descrittori di file ereditati dal processo padre oltre stdin, stdout e stderr,
/// prima di leggere qualunque richiesta: un estrattore compromesso da un file ostile non deve
/// avere in mano pipe o file aperti dal processo che ha avviato l'app (per esempio le pipe del
/// jobserver di cargo, o descrittori lasciati aperti da un file manager).
///
/// Linux: elenca `/proc/self/fd` e chiude ogni numero da 3 in su. Windows: non fa nulla, il
/// processo principale passa al figlio solo gli handle elencati. Altri sistemi: non fa nulla.
///
/// Errore se `/proc/self/fd` non si legge: chi chiama non serve richieste.
///
/// # Safety
///
/// Va chiamata per prima cosa nel `main` di un processo estrattore, con un solo thread e
/// senza file aperti dal programma: ogni descrittore da 3 in su deve essere ereditato, senza
/// un proprietario nel programma (`File`, `OwnedFd`, pipe, …) che lo userebbe o lo
/// chiuderebbe dopo.
#[allow(
    unsafe_code,
    reason = "la funzione chiude descrittori per numero: il chiamante garantisce che nessuno li possiede"
)]
pub unsafe fn close_inherited_descriptors() -> io::Result<()> {
    // SAFETY: le condizioni sono quelle di questa funzione, garantite dal chiamante.
    unsafe { imp::close_inherited_descriptors() }
}

/// Intervallo dei controlli del padre.
#[cfg(any(target_os = "linux", test))]
const PARENT_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// Legge il padre con `read_parent` finché è `expected`, chiamando `pause` tra una lettura e
/// l'altra; torna appena è diverso o non si legge, anche alla prima lettura.
#[cfg(any(target_os = "linux", test))]
fn wait_while_parent_is(
    expected: u32,
    mut read_parent: impl FnMut() -> io::Result<u32>,
    mut pause: impl FnMut(),
) {
    while read_parent().ok() == Some(expected) {
        pause();
    }
}

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
        reason = "stessa firma della versione Linux, che può fallire"
    )]
    #[allow(unsafe_code, reason = "stessa firma della versione Linux")]
    pub unsafe fn close_inherited_descriptors() -> io::Result<()> {
        Ok(())
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
    pub fn watch_parent(
        _expected: Option<u32>,
        _on_gone: impl FnOnce() + Send + 'static,
    ) -> io::Result<()> {
        Ok(())
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

    #[cfg(target_os = "linux")]
    #[allow(
        unsafe_code,
        reason = "chiudere un descrittore ereditato, che nessun oggetto del programma possiede"
    )]
    pub unsafe fn close_inherited_descriptors() -> io::Result<()> {
        /// Il primo descrittore dopo stdin, stdout e stderr.
        const FIRST_INHERITED: i32 = 3;

        // Prima l'elenco, poi le chiusure: l'elenco tiene aperto un descrittore suo, che
        // risulta nell'elenco ed è già chiuso quando si chiude (EBADF, ignorato).
        let mut inherited = Vec::new();
        for entry in std::fs::read_dir("/proc/self/fd")? {
            let name = entry?.file_name();
            if let Some(fd) = name.to_str().and_then(|name| name.parse::<i32>().ok())
                && fd >= FIRST_INHERITED
            {
                inherited.push(fd);
            }
        }
        for fd in inherited {
            // SAFETY: per il contratto della funzione `fd` è un descrittore ereditato (o quello
            // dell'elenco, già chiuso): nessun oggetto del programma lo possiede né lo userà, e
            // con un solo thread nessuno può aprire un file che riceva lo stesso numero prima
            // della chiusura. `close` su un numero già chiuso fallisce con EBADF senza effetti.
            unsafe { rustix::io::close(fd) };
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "stessa firma della versione Linux, che può fallire"
    )]
    #[allow(unsafe_code, reason = "stessa firma della versione Linux")]
    pub unsafe fn close_inherited_descriptors() -> io::Result<()> {
        Ok(())
    }

    pub fn limit_address_space(bytes: u64) -> io::Result<MemoryLimitOwner> {
        rlimit::Resource::AS.set(bytes, bytes)?;
        Ok(MemoryLimitOwner::SelfImposed)
    }

    #[cfg(target_os = "linux")]
    pub fn watch_parent(
        expected: Option<u32>,
        on_gone: impl FnOnce() + Send + 'static,
    ) -> io::Result<()> {
        use std::thread;

        use super::{PARENT_CHECK_INTERVAL, wait_while_parent_is};

        /// Pila del thread di sorveglianza: legge un file corto.
        const WATCH_STACK_BYTES: usize = 64 * 1024;

        let expected = match expected {
            Some(pid) => pid,
            None => current_parent()?,
        };
        thread::Builder::new()
            .name("lynkeus-padre".to_owned())
            .stack_size(WATCH_STACK_BYTES)
            .spawn(move || {
                // Un padre che non si legge più conta come cambiato: meglio fermarsi.
                wait_while_parent_is(expected, current_parent, || {
                    thread::sleep(PARENT_CHECK_INTERVAL);
                });
                on_gone();
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
    pub fn watch_parent(
        _expected: Option<u32>,
        _on_gone: impl FnOnce() + Send + 'static,
    ) -> io::Result<()> {
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

    /// Letture del padre in sequenza; finite, il padre non si legge più.
    fn parents<const N: usize>(sequence: [u32; N]) -> impl FnMut() -> io::Result<u32> {
        let mut sequence = sequence.into_iter();
        move || sequence.next().ok_or_else(|| io::Error::other("finito"))
    }

    /// Il padre atteso si confronta dalla prima lettura: un padre già morto prima della
    /// sorveglianza (processo già passato a `init`) si vede subito; un padre che cambia dopo
    /// si vede alla lettura seguente; un padre che non si legge conta come cambiato.
    #[test]
    fn parent_watch_partitions() {
        let mut pauses = 0;
        wait_while_parent_is(42, parents([1]), || pauses += 1);
        assert_eq!(pauses, 0, "padre già cambiato prima di iniziare");
        let mut pauses = 0;
        wait_while_parent_is(42, parents([42, 42, 1]), || pauses += 1);
        assert_eq!(pauses, 2);
        let mut pauses = 0;
        wait_while_parent_is(42, parents([42]), || pauses += 1);
        assert_eq!(pauses, 1, "padre illeggibile alla seconda lettura");
    }

    /// Su Linux, con un padre atteso che non è il padre vero, `on_gone` arriva subito.
    #[cfg(target_os = "linux")]
    #[test]
    fn wrong_expected_parent_is_gone_at_once() {
        let (tx, rx) = std::sync::mpsc::channel();
        // Nessun processo ha questo pid (il massimo di Linux è 2^22).
        let not_my_parent = u32::MAX;
        let started = watch_parent(Some(not_my_parent), move || {
            let _sent = tx.send(());
        });
        assert!(started.is_ok());
        assert!(rx.recv_timeout(std::time::Duration::from_secs(5)).is_ok());
    }
}
