// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Differenze tra Windows e Linux nell'app; nessun altro modulo usa `cfg`.
//!
//! - Font: i font di egui non sono inclusi (licenza di Ubuntu-Light fuori dall'allowlist).
//!   Su Windows Segoe UI Regular e Semibold da `C:\Windows\Fonts` (mai inclusi nel
//!   pacchetto); su Linux Noto Sans o DejaVu Sans se installati. `system_fonts` di eframe
//!   copre i caratteri mancanti (CJK e altri alfabeti).
//! - Cartella di configurazione (`settings.toml` e posizione della finestra).
//! - Flush della cartella dopo una rinomina durevole.
//! - Apertura di un file con il programma predefinito e "Mostra nella cartella".
//! - Ordine dei bottoni nei dialoghi.
//! - Ricerca delle DLL ristretta alla cartella di sistema (Windows).
//! - Avvio del processo estrattore: su Windows nel Job Object con `CreateProcessW`
//!   (`windows_ffi`), altrove con il lanciatore portabile dell'engine.
//! - Cartelle dell'app ([`AppDirs`]) e cartelle e file privati ([`private_fs`]: descrittore
//!   esplicito su Windows, 0700 e 0600 su Linux).
//! - Istanza unica: named pipe (Windows) o socket Unix (Linux) con tempi limite
//!   ([`InstanceListener`], [`connect_instance`]).
//! - Voce del clic destro: registro dell'utente (Windows) o script di Nautilus (Linux)
//!   ([`ContextMenu`]).

#[cfg(windows)]
mod windows_ffi;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};
use lynkeus_core::ByteSize;
use lynkeus_engine::{PrivateFs, WorkerLauncher};

use crate::theme::SEMIBOLD;

/// Limita la ricerca delle DLL alla cartella di sistema (Windows,
/// `SetDefaultDllDirectories`): va chiamata come prima istruzione di `main`, prima che una
/// libreria ne carichi. Altrove non fa nulla.
#[cfg(windows)]
pub fn restrict_dll_search() -> io::Result<()> {
    windows_ffi::restrict_dll_search()
}

/// Vedi la versione per Windows.
#[cfg(not(windows))]
pub fn restrict_dll_search() -> io::Result<()> {
    Ok(())
}

/// Il lanciatore del processo estrattore `exe` (percorso assoluto) con limite di memoria
/// `memory_limit`.
///
/// Windows: `CreateProcessW` dentro un Job Object per processo, con il limite imposto dal
/// sistema, `KILL_ON_JOB_CLOSE` e solo stdin e stdout ereditati. Linux: lanciatore portabile;
/// il limite lo impone il figlio a sé stesso (`RLIMIT_AS`).
#[cfg(windows)]
pub fn worker_launcher(
    exe: PathBuf,
    memory_limit: ByteSize,
) -> io::Result<Arc<dyn WorkerLauncher>> {
    Ok(Arc::new(job_launcher::JobLauncher::new(exe, memory_limit)?))
}

/// Vedi la versione per Windows.
#[cfg(not(windows))]
pub fn worker_launcher(
    exe: PathBuf,
    memory_limit: ByteSize,
) -> io::Result<Arc<dyn WorkerLauncher>> {
    Ok(Arc::new(lynkeus_engine::CommandLauncher::new(
        exe,
        Some(memory_limit),
    )?))
}

#[cfg(windows)]
mod job_launcher {
    use std::ffi::OsString;
    use std::io;
    use std::path::PathBuf;
    use std::time::Duration;

    use lynkeus_core::ByteSize;
    use lynkeus_engine::{LaunchedWorker, WorkerExit, WorkerLauncher, WorkerProcess, worker_args};

    use super::windows_ffi::{ChildProcess, Job, spawn_in_job};

    /// Avvia ogni processo estrattore nel suo Job Object.
    pub struct JobLauncher {
        exe: PathBuf,
        args: Vec<OsString>,
        memory_limit: usize,
    }

    impl JobLauncher {
        pub fn new(exe: PathBuf, memory_limit: ByteSize) -> io::Result<JobLauncher> {
            if !exe.is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "eseguibile del processo estrattore senza percorso assoluto",
                ));
            }
            Ok(JobLauncher {
                exe,
                // Il figlio riceve il limite anche qui, dove lo impone il job: lo stesso
                // comando su ogni sistema.
                args: worker_args(Some(memory_limit)),
                memory_limit: usize::try_from(memory_limit.bytes()).unwrap_or(usize::MAX),
            })
        }
    }

    impl WorkerLauncher for JobLauncher {
        fn launch(&self) -> io::Result<LaunchedWorker> {
            let job = Job::new(self.memory_limit)?;
            let child = spawn_in_job(&self.exe, &self.args, &job)?;
            // Verifica che fallisce chiusa: un estrattore fuori dal job non si usa.
            if !child.process.is_in(&job).unwrap_or(false) {
                let _killed = child.process.terminate();
                return Err(io::Error::other("processo estrattore fuori dal Job Object"));
            }
            Ok(LaunchedWorker {
                stdin: Box::new(child.stdin),
                stdout: Box::new(child.stdout),
                process: Box::new(JobWorker {
                    job,
                    process: child.process,
                    memory_limit: self.memory_limit,
                    exited: None,
                }),
            })
        }
    }

    /// Un processo estrattore con il suo job: chiuderlo termina il processo.
    struct JobWorker {
        job: Job,
        process: ChildProcess,
        memory_limit: usize,
        exited: Option<WorkerExit>,
    }

    impl JobWorker {
        /// Vero se il processo è uscito per il limite di memoria: il messaggio del job, o un
        /// picco arrivato quasi al limite (entro 1/16) se il messaggio non c'è.
        fn hit_memory_limit(&mut self) -> bool {
            if self.job.memory_limit_hit() {
                return true;
            }
            let margin = self.memory_limit / 16;
            self.job
                .peak_process_memory()
                .is_ok_and(|peak| peak >= self.memory_limit.saturating_sub(margin))
        }
    }

    impl WorkerProcess for JobWorker {
        fn wait_timeout(&mut self, timeout: Duration) -> io::Result<Option<WorkerExit>> {
            if let Some(exit) = self.exited {
                return Ok(Some(exit));
            }
            let Some(code) = self.process.wait(timeout)? else {
                return Ok(None);
            };
            let exit = if code == 0 {
                WorkerExit::Success
            } else if self.hit_memory_limit() {
                WorkerExit::MemoryExceeded
            } else {
                WorkerExit::Failed {
                    code: Some(i64::from(code)),
                }
            };
            self.exited = Some(exit);
            Ok(Some(exit))
        }

        fn kill(&mut self) -> io::Result<()> {
            if self.exited.is_some() {
                return Ok(());
            }
            // Il job termina anche eventuali figli del processo; il processo da solo è il
            // ripiego.
            self.job.terminate().or_else(|_| self.process.terminate())
        }
    }
}

/// Nome della cartella dell'app sotto la cartella di configurazione dell'utente.
#[cfg(windows)]
const APP_DIR_NAME: &str = "Lynkeus Searcher";
#[cfg(not(windows))]
const APP_DIR_NAME: &str = "lynkeus-searcher";

/// Rende durevole la rinomina appena fatta in `dir`.
///
/// Linux: `fsync` della cartella, senza il quale la rinomina può andare persa in un crash.
/// Windows: nulla; NTFS registra la rinomina nel suo journal e `FlushFileBuffers` su una
/// cartella chiederebbe permessi di backup.
#[cfg(not(windows))]
pub fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

/// Vedi la versione per Linux.
#[cfg(windows)]
pub fn sync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Ordine dei bottoni nei dialoghi: su Windows l'azione prima di Annulla, su GNOME
/// Annulla prima dell'azione.
#[cfg(windows)]
pub const ACTION_BUTTON_FIRST: bool = true;
/// Vedi la versione per Windows.
#[cfg(not(windows))]
pub const ACTION_BUTTON_FIRST: bool = false;

/// Apre `path` (assoluto) con il programma predefinito del sistema. Blocca per poco ma non
/// garantisce un tempo: da chiamare fuori dal thread dell'interfaccia.
///
/// Windows: `ShellExecuteW` con il verbo "open" (crate `opener`), cioè l'associazione dei
/// tipi di file; nessun programma cercato per nome.
#[cfg(windows)]
pub fn open_with_default_program(path: &Path) -> std::io::Result<()> {
    opener::open(path).map_err(open_error)
}

/// Mostra `path` (assoluto) selezionato nella sua cartella.
///
/// Windows: `SHOpenFolderAndSelectItems` (crate `opener`).
#[cfg(windows)]
pub fn reveal_in_folder(path: &Path) -> std::io::Result<()> {
    opener::reveal(path).map_err(open_error)
}

#[cfg(windows)]
fn open_error(error: opener::OpenError) -> std::io::Error {
    match error {
        opener::OpenError::Io(error) => error,
        other => std::io::Error::other(other.to_string()),
    }
}

/// Linux: metodo `OpenFile` del portale XDG `org.freedesktop.portal.OpenURI`, con il file
/// passato come descrittore aperto (nessun programma cercato per nome, nessun URI da
/// comporre).
#[cfg(not(windows))]
pub fn open_with_default_program(path: &Path) -> std::io::Result<()> {
    call_open_uri_portal("OpenFile", path)
}

/// Linux: metodo `OpenDirectory` del portale XDG, che apre la cartella del file.
#[cfg(not(windows))]
pub fn reveal_in_folder(path: &Path) -> std::io::Result<()> {
    call_open_uri_portal("OpenDirectory", path)
}

/// Chiama `method(parent_window, fd, options)` del portale OpenURI sul bus di sessione.
#[cfg(not(windows))]
fn call_open_uri_portal(method: &str, path: &Path) -> std::io::Result<()> {
    use std::collections::HashMap;
    use std::os::fd::AsFd;

    use zbus::zvariant::{Fd, Value};

    let file = std::fs::File::open(path)?;
    let connection = zbus::blocking::Connection::session().map_err(std::io::Error::other)?;
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    connection
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.OpenURI"),
            method,
            &("", Fd::from(file.as_fd()), options),
        )
        .map_err(std::io::Error::other)?;
    Ok(())
}

/// Carica nel contesto i font dell'interfaccia: Regular per il testo, SemiBold per titoli
/// ed evidenziati (famiglia [`SEMIBOLD`]), un monospazio se presente.
///
/// La famiglia SemiBold è sempre definita: senza il file Semibold ricade sul Regular, così
/// nessun testo resta senza font. Se non trova nemmeno il Regular lascia i font di ripiego
/// del sistema forniti da eframe e definisce comunque la famiglia SemiBold come quella
/// proporzionale.
pub fn install_fonts(ctx: &egui::Context) {
    let Some(regular) = first_readable(&regular_candidates()) else {
        let mut fonts = FontDefinitions::default();
        let proportional = fonts
            .families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default();
        fonts
            .families
            .insert(FontFamily::Name(SEMIBOLD.into()), proportional);
        ctx.set_fonts(fonts);
        return;
    };
    let semibold = first_readable(&semibold_candidates());
    let monospace = first_readable(&monospace_candidates());

    let mut fonts = FontDefinitions::empty();
    fonts
        .font_data
        .insert("ui".to_owned(), Arc::new(FontData::from_owned(regular)));
    let proportional = vec!["ui".to_owned()];
    let mut strong = Vec::new();
    if let Some(bytes) = semibold {
        fonts.font_data.insert(
            "ui-semibold".to_owned(),
            Arc::new(FontData::from_owned(bytes)),
        );
        strong.push("ui-semibold".to_owned());
    }
    strong.extend(proportional.iter().cloned());
    let mut mono = Vec::new();
    if let Some(bytes) = monospace {
        fonts
            .font_data
            .insert("mono".to_owned(), Arc::new(FontData::from_owned(bytes)));
        mono.push("mono".to_owned());
    }
    mono.extend(proportional.iter().cloned());

    fonts
        .families
        .insert(FontFamily::Proportional, proportional);
    fonts.families.insert(FontFamily::Monospace, mono);
    fonts
        .families
        .insert(FontFamily::Name(SEMIBOLD.into()), strong);
    ctx.set_fonts(fonts);
}

/// Il contenuto del primo file leggibile tra `candidates`.
fn first_readable(candidates: &[PathBuf]) -> Option<Vec<u8>> {
    candidates.iter().find_map(|path| std::fs::read(path).ok())
}

#[cfg(windows)]
fn fonts_dir() -> PathBuf {
    std::env::var_os("WINDIR")
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join("Fonts")
}

#[cfg(windows)]
fn regular_candidates() -> Vec<PathBuf> {
    vec![fonts_dir().join("segoeui.ttf")]
}

#[cfg(windows)]
fn semibold_candidates() -> Vec<PathBuf> {
    vec![fonts_dir().join("seguisb.ttf")]
}

#[cfg(windows)]
fn monospace_candidates() -> Vec<PathBuf> {
    vec![fonts_dir().join("consola.ttf")]
}

#[cfg(not(windows))]
fn regular_candidates() -> Vec<PathBuf> {
    [
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/opentype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    ]
    .map(PathBuf::from)
    .to_vec()
}

#[cfg(not(windows))]
fn semibold_candidates() -> Vec<PathBuf> {
    [
        "/usr/share/fonts/truetype/noto/NotoSans-SemiBold.ttf",
        "/usr/share/fonts/opentype/noto/NotoSans-SemiBold.ttf",
        "/usr/share/fonts/noto/NotoSans-SemiBold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
    ]
    .map(PathBuf::from)
    .to_vec()
}

#[cfg(not(windows))]
fn monospace_candidates() -> Vec<PathBuf> {
    [
        "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
    ]
    .map(PathBuf::from)
    .to_vec()
}

// --- Cartelle e file privati -----------------------------------------------------------

/// Le cartelle e i file privati della piattaforma: su Windows con il descrittore esplicito
/// (solo utente e SYSTEM, `windows_ffi`), su Linux 0700 e 0600 con la verifica delle
/// cartelle esistenti (engine).
#[cfg(windows)]
pub fn private_fs() -> Arc<dyn PrivateFs> {
    Arc::new(windows_ffi::WindowsPrivateFs)
}

/// Vedi la versione per Windows.
#[cfg(not(windows))]
pub fn private_fs() -> Arc<dyn PrivateFs> {
    Arc::new(lynkeus_engine::PortablePrivateFs)
}

/// Le cartelle dell'app nel profilo dell'utente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDirs {
    /// Configurazione: `settings.toml` e posizione della finestra.
    pub config: PathBuf,
    /// Dati locali: lock e file dell'istanza unica.
    pub data: PathBuf,
    /// Cache del testo dei documenti.
    pub cache: PathBuf,
    /// Log (con rotazione).
    pub logs: PathBuf,
    /// Rapporti di crash.
    pub crash_reports: PathBuf,
    /// File estratti dagli archivi per aprirli.
    pub opened_entries: PathBuf,
}

impl AppDirs {
    /// Le cartelle della piattaforma (`directories`); `None` se il sistema non indica una
    /// cartella dell'utente.
    ///
    /// Windows: `%APPDATA%\Lynkeus Searcher\config`, `%LOCALAPPDATA%\Lynkeus Searcher\{data,
    /// cache}` e sotto `data` le cartelle `logs`, `crash-reports`, `temp`. Linux:
    /// `~/.config/lynkeus-searcher`, `~/.cache/lynkeus-searcher`,
    /// `~/.local/share/lynkeus-searcher` con le stesse sottocartelle.
    pub fn for_user() -> Option<AppDirs> {
        let dirs = directories::ProjectDirs::from("", "", APP_DIR_NAME)?;
        let data = dirs.data_local_dir().to_owned();
        Some(AppDirs {
            config: dirs.config_dir().to_owned(),
            cache: dirs.cache_dir().to_owned(),
            logs: data.join("logs"),
            crash_reports: data.join("crash-reports"),
            opened_entries: data.join("temp"),
            data,
        })
    }
}

// --- Istanza unica: trasporto ------------------------------------------------------------

/// Prefisso del nome del canale dell'istanza unica.
const ENDPOINT_PREFIX: &str = "lynkeus-searcher-";

#[cfg(not(windows))]
pub use instance_unix::{InstanceConnection, InstanceListener, InstanceStopper, connect_instance};
#[cfg(windows)]
pub use instance_windows::{
    InstanceConnection, InstanceListener, InstanceStopper, connect_instance,
};

#[cfg(windows)]
mod instance_windows {
    use std::io;
    use std::sync::Arc;
    use std::time::Instant;

    use super::ENDPOINT_PREFIX;
    use super::windows_ffi::{
        PipeClient, PipeServer, StopSignal, allow_set_foreground, current_user, process_user,
    };
    use crate::instance::Channel;

    const PIPE_PREFIX: &str = r"\\.\pipe\";

    /// La pipe dell'istanza unica (`\\.\pipe\lynkeus-searcher-<casuale>`).
    #[derive(Debug)]
    pub struct InstanceListener {
        server: PipeServer,
        stop: Arc<StopSignal>,
        endpoint: String,
    }

    impl InstanceListener {
        /// Crea la pipe con il suffisso casuale `suffix`; errore se il nome esiste già.
        pub fn bind(suffix: &str) -> io::Result<InstanceListener> {
            let endpoint = format!("{PIPE_PREFIX}{ENDPOINT_PREFIX}{suffix}");
            Ok(InstanceListener {
                server: PipeServer::create(&endpoint)?,
                stop: Arc::new(StopSignal::new()?),
                endpoint,
            })
        }

        /// Il nome da scrivere nel file dell'istanza.
        pub fn endpoint(&self) -> &str {
            &self.endpoint
        }

        /// Chi può fermare [`InstanceListener::accept`] da un altro thread.
        pub fn stopper(&self) -> InstanceStopper {
            InstanceStopper {
                stop: Arc::clone(&self.stop),
            }
        }

        /// Aspetta un client; `None` dopo lo stop.
        pub fn accept(&self) -> io::Result<Option<InstanceConnection<'_>>> {
            Ok(self
                .server
                .accept(&self.stop)?
                .then_some(InstanceConnection::Server(&self.server)))
        }
    }

    /// Ferma il server dell'istanza.
    #[derive(Debug)]
    pub struct InstanceStopper {
        stop: Arc<StopSignal>,
    }

    impl InstanceStopper {
        /// Interrompe l'attesa di un client.
        pub fn stop(&self) {
            self.stop.set();
        }
    }

    /// Una connessione: lato server (chiusa nel `Drop`) o lato client.
    #[derive(Debug)]
    pub enum InstanceConnection<'a> {
        /// Il client connesso alla pipe del server.
        Server(&'a PipeServer),
        /// La pipe aperta dal client.
        Client(PipeClient),
    }

    impl Drop for InstanceConnection<'_> {
        fn drop(&mut self) {
            if let InstanceConnection::Server(server) = self {
                server.disconnect();
            }
        }
    }

    impl Channel for InstanceConnection<'_> {
        fn read_exact(&mut self, buffer: &mut [u8], deadline: Instant) -> io::Result<()> {
            match self {
                InstanceConnection::Server(server) => server.read_exact(buffer, deadline),
                InstanceConnection::Client(client) => client.read_exact(buffer, deadline),
            }
        }

        fn write_all(&mut self, bytes: &[u8], deadline: Instant) -> io::Result<()> {
            match self {
                InstanceConnection::Server(server) => server.write_all(bytes, deadline),
                InstanceConnection::Client(client) => client.write_all(bytes, deadline),
            }
        }
    }

    /// Si connette alla prima istanza: pipe aperta con SQoS Identification, server
    /// verificato come processo dello stesso utente prima di mandare il token, poi permesso
    /// di portarsi in primo piano.
    pub fn connect_instance(
        endpoint: &str,
        deadline: Instant,
    ) -> io::Result<InstanceConnection<'static>> {
        let valid_name = endpoint
            .strip_prefix(PIPE_PREFIX)
            .is_some_and(|name| name.starts_with(ENDPOINT_PREFIX) && !name.contains('\\'));
        if !valid_name {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "nome della pipe non valido",
            ));
        }
        let client = PipeClient::connect(endpoint, deadline)?;
        let pid = client.server_pid()?;
        if process_user(pid)? != current_user()? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "pipe di un altro utente",
            ));
        }
        if let Err(error) = allow_set_foreground(pid) {
            tracing::debug!(%error, "AllowSetForegroundWindow non riuscita");
        }
        Ok(InstanceConnection::Client(client))
    }
}

#[cfg(not(windows))]
mod instance_unix {
    use std::io::{self, Read, Write};
    use std::marker::PhantomData;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    use rustix::fs::{Mode, OFlags};

    use super::ENDPOINT_PREFIX;
    use crate::instance::Channel;

    /// `$XDG_RUNTIME_DIR` se è una cartella dell'utente senza permessi per altri; altrimenti
    /// l'istanza unica resta spenta (fallire chiusi).
    fn runtime_dir() -> io::Result<PathBuf> {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR"))?;
        let fd = rustix::fs::open(
            &dir,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let stat = rustix::fs::fstat(&fd)?;
        if stat.st_uid != rustix::process::geteuid().as_raw() || stat.st_mode & 0o077 != 0 {
            return Err(lynkeus_engine::NotPrivate::error());
        }
        Ok(dir)
    }

    /// Il socket dell'istanza unica in `$XDG_RUNTIME_DIR`, 0600 già alla creazione.
    #[derive(Debug)]
    pub struct InstanceListener {
        listener: UnixListener,
        path: PathBuf,
        endpoint: String,
        stop: Arc<AtomicBool>,
    }

    impl InstanceListener {
        /// Crea il socket con il suffisso casuale `suffix`; errore se il nome esiste già.
        pub fn bind(suffix: &str) -> io::Result<InstanceListener> {
            let path = runtime_dir()?.join(format!("{ENDPOINT_PREFIX}{suffix}.sock"));
            let endpoint = path
                .to_str()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "percorso"))?
                .to_owned();
            // La umask vale per tutto il processo: si chiama all'avvio, prima della
            // finestra; 0177 dà 0600 al socket.
            let previous = rustix::process::umask(Mode::from_bits_truncate(0o177));
            let bound = UnixListener::bind(&path);
            rustix::process::umask(previous);
            Ok(InstanceListener {
                listener: bound?,
                path,
                endpoint,
                stop: Arc::new(AtomicBool::new(false)),
            })
        }

        /// Il percorso da scrivere nel file dell'istanza.
        pub fn endpoint(&self) -> &str {
            &self.endpoint
        }

        /// Chi può fermare [`InstanceListener::accept`] da un altro thread.
        pub fn stopper(&self) -> InstanceStopper {
            InstanceStopper {
                stop: Arc::clone(&self.stop),
                path: self.path.clone(),
            }
        }

        /// Aspetta un client dello stesso utente (`SO_PEERCRED`); `None` dopo lo stop.
        pub fn accept(&self) -> io::Result<Option<InstanceConnection<'_>>> {
            loop {
                let (stream, _) = self.listener.accept()?;
                if self.stop.load(Ordering::Relaxed) {
                    return Ok(None);
                }
                let peer = rustix::net::sockopt::socket_peercred(&stream)?;
                if peer.uid != rustix::process::geteuid() {
                    tracing::warn!("istanza unica: connessione di un altro utente rifiutata");
                    continue;
                }
                return Ok(Some(InstanceConnection {
                    stream,
                    _server: PhantomData,
                }));
            }
        }
    }

    impl Drop for InstanceListener {
        fn drop(&mut self) {
            let _removed = std::fs::remove_file(&self.path);
        }
    }

    /// Ferma il server dell'istanza.
    #[derive(Debug)]
    pub struct InstanceStopper {
        stop: Arc<AtomicBool>,
        path: PathBuf,
    }

    impl InstanceStopper {
        /// Interrompe l'attesa di un client (connettendosi al socket per svegliarla).
        pub fn stop(&self) {
            self.stop.store(true, Ordering::Relaxed);
            let _woken = UnixStream::connect(&self.path);
        }
    }

    /// Una connessione con tempi limite.
    #[derive(Debug)]
    pub struct InstanceConnection<'a> {
        stream: UnixStream,
        _server: PhantomData<&'a ()>,
    }

    /// Il tempo che resta fino a `deadline`, o `TimedOut`.
    fn remaining(deadline: Instant) -> io::Result<std::time::Duration> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            Err(io::Error::from(io::ErrorKind::TimedOut))
        } else {
            Ok(left)
        }
    }

    impl Channel for InstanceConnection<'_> {
        fn read_exact(&mut self, buffer: &mut [u8], deadline: Instant) -> io::Result<()> {
            let mut filled = 0;
            while filled < buffer.len() {
                self.stream.set_read_timeout(Some(remaining(deadline)?))?;
                let rest = buffer.get_mut(filled..).unwrap_or_default();
                match self.stream.read(rest) {
                    Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
                    Ok(read) => filled += read,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        }

        fn write_all(&mut self, bytes: &[u8], deadline: Instant) -> io::Result<()> {
            self.stream.set_write_timeout(Some(remaining(deadline)?))?;
            self.stream.write_all(bytes)
        }
    }

    /// Si connette alla prima istanza (socket in una cartella dell'utente).
    pub fn connect_instance(
        endpoint: &str,
        deadline: Instant,
    ) -> io::Result<InstanceConnection<'static>> {
        let path = Path::new(endpoint);
        let valid = path.is_absolute()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(ENDPOINT_PREFIX));
        if !valid {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "percorso del socket non valido",
            ));
        }
        let stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        Ok(InstanceConnection {
            stream,
            _server: PhantomData,
        })
    }
}

// --- Clic destro --------------------------------------------------------------------------

#[cfg(not(windows))]
pub use context_menu_nautilus::ContextMenu;
#[cfg(windows)]
pub use context_menu_windows::ContextMenu;

#[cfg(windows)]
mod context_menu_windows {
    use std::io;
    use std::path::Path;

    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};

    use crate::context_menu::{
        ContextMenuStore, FoundItem, MenuEntry, MenuState, MenuTarget, classify_windows,
    };

    /// Le voci in `HKCU\Software\Classes` (mai tramite `HKEY_CLASSES_ROOT`, che fonde
    /// utente e macchina). La chiave si apre a ogni operazione (`RegKey` non è `Sync`).
    pub struct ContextMenu {
        classes_path: String,
    }

    impl ContextMenu {
        /// Le voci dell'utente corrente.
        pub fn for_current_user() -> io::Result<ContextMenu> {
            let menu = ContextMenu::with_classes_path(r"Software\Classes");
            menu.classes()?;
            Ok(menu)
        }

        /// Le voci sotto `HKCU\<classes_path>` al posto di `Software\Classes` (test su una
        /// radice propria).
        pub fn with_classes_path(classes_path: &str) -> ContextMenu {
            ContextMenu {
                classes_path: classes_path.to_owned(),
            }
        }

        fn classes(&self) -> io::Result<RegKey> {
            RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey_with_flags(&self.classes_path, KEY_READ | KEY_WRITE)
        }

        fn read(&self, target: MenuTarget) -> io::Result<Option<FoundItem>> {
            let key = match self.classes()?.open_subkey(target.key_path()) {
                Ok(key) => key,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            };
            let label = key.get_value::<String, _>("").ok();
            let command = key
                .open_subkey("command")
                .ok()
                .and_then(|command| command.get_value::<String, _>("").ok());
            Ok(Some(FoundItem { label, command }))
        }
    }

    impl ContextMenuStore for ContextMenu {
        fn state(&self, entry: &MenuEntry) -> io::Result<MenuState> {
            let found = [
                self.read(MenuTarget::ALL[0])?,
                self.read(MenuTarget::ALL[1])?,
                self.read(MenuTarget::ALL[2])?,
            ];
            Ok(classify_windows(&found, entry))
        }

        fn install(&self, entry: &MenuEntry) -> io::Result<()> {
            let icon = entry.exe().display().to_string();
            let classes = self.classes()?;
            for target in MenuTarget::ALL {
                let (key, _) = classes.create_subkey(target.key_path())?;
                key.set_value("", &entry.label())?;
                key.set_value("Icon", &icon)?;
                let (command, _) = key.create_subkey("command")?;
                command.set_value("", &entry.windows_command(target))?;
            }
            Ok(())
        }

        fn remove(&self, _entry: &MenuEntry) -> io::Result<()> {
            let classes = self.classes()?;
            for target in MenuTarget::ALL {
                match classes.delete_subkey_all(target.key_path()) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        }

        fn exe_dir_writable_by_others(&self, exe: &Path) -> io::Result<bool> {
            let dir = exe
                .parent()
                .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
            super::windows_ffi::dir_writable_by_others(dir)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::context_menu::{disable, enable, refresh};

        /// Una radice di prova propria in `HKCU\Software`, tolta alla fine.
        struct TestRoot {
            parent: RegKey,
            name: String,
        }

        impl TestRoot {
            fn new() -> TestRoot {
                let mut bytes = [0u8; 8];
                getrandom::fill(&mut bytes).expect("casuale");
                let suffix: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                let parent = RegKey::predef(HKEY_CURRENT_USER)
                    .open_subkey_with_flags("Software", KEY_READ | KEY_WRITE)
                    .expect("HKCU\\Software");
                TestRoot {
                    parent,
                    name: format!("LynkeusSearcherTest-{suffix}"),
                }
            }

            fn menu(&self) -> ContextMenu {
                self.parent.create_subkey(&self.name).expect("radice");
                ContextMenu::with_classes_path(&format!(r"Software\{}", self.name))
            }

            fn key(&self, path: &str) -> io::Result<RegKey> {
                self.parent.open_subkey(format!("{}\\{path}", self.name))
            }
        }

        impl Drop for TestRoot {
            fn drop(&mut self) {
                let _removed = self.parent.delete_subkey_all(&self.name);
            }
        }

        fn entry(exe: &str) -> MenuEntry {
            MenuEntry::new(Path::new(exe), "Cerca con Lynkeus", &[]).expect("voce")
        }

        /// Il registro scritto e riletto: formato atteso con spazi e Unicode nel percorso,
        /// eseguibile spostato riscritto, ciclo accendi-spegni che lascia la radice vuota.
        #[test]
        fn registry_entries_are_written_read_and_removed() {
            let root = TestRoot::new();
            let menu = root.menu();
            let exe = r"C:\Programmi portatili\Città è\lynkeus-searcher.exe";
            let first = entry(exe);
            assert_eq!(menu.state(&first).expect("stato"), MenuState::Missing);
            menu.install(&first).expect("scritta");
            assert_eq!(menu.state(&first).expect("stato"), MenuState::Current);

            let folder = root
                .key(r"Directory\shell\LynkeusSearcher")
                .expect("chiave");
            assert_eq!(
                folder.get_value::<String, _>("").expect("testo"),
                "Cerca con Lynkeus"
            );
            assert_eq!(folder.get_value::<String, _>("Icon").expect("icona"), exe);
            let command = root
                .key(r"Directory\shell\LynkeusSearcher\command")
                .and_then(|k| k.get_value::<String, _>(""))
                .expect("comando");
            assert_eq!(command, format!("\"{exe}\" --add \"%1\""));
            let background = root
                .key(r"Directory\Background\shell\LynkeusSearcher\command")
                .and_then(|k| k.get_value::<String, _>(""))
                .expect("comando");
            assert_eq!(background, format!("\"{exe}\" --add \"%V\""));
            assert!(root.key(r"*\shell\LynkeusSearcher\command").is_ok());

            // Eseguibile spostato: la voce si riconosce come nostra e si riscrive.
            let moved = entry(r"D:\Nuova cartella\lynkeus-searcher.exe");
            assert_eq!(menu.state(&moved).expect("stato"), MenuState::Outdated);
            menu.install(&moved).expect("riscritta");
            assert_eq!(menu.state(&moved).expect("stato"), MenuState::Current);

            menu.remove(&moved).expect("tolta");
            assert_eq!(menu.state(&moved).expect("stato"), MenuState::Missing);
            let classes = root.key("").expect("radice");
            assert_eq!(
                classes.enum_keys().count(),
                2,
                "restano solo `*` e `Directory`"
            );
            for class in [
                r"*\shell",
                r"Directory\shell",
                r"Directory\Background\shell",
            ] {
                let shell = root.key(class).expect("shell");
                assert_eq!(shell.enum_keys().count(), 0, "{class}");
            }
        }

        /// Una voce con il nostro nome ma di un altro programma: non sovrascritta, non tolta.
        #[test]
        fn a_foreign_entry_is_left_alone() {
            let root = TestRoot::new();
            let menu = root.menu();
            let classes = root.key("").expect("radice");
            let (foreign, _) = classes
                .create_subkey(r"Directory\shell\LynkeusSearcher\command")
                .expect("chiave");
            foreign
                .set_value("", &r#""C:\altro\programma.exe" "%1""#)
                .expect("valore");
            let ours = entry(r"C:\L\lynkeus-searcher.exe");
            assert_eq!(menu.state(&ours).expect("stato"), MenuState::Foreign);
            assert_eq!(
                refresh(&NotWritable(&menu), &ours),
                Err(crate::context_menu::MenuError::Foreign)
            );
            assert_eq!(
                enable(&NotWritable(&menu), &ours),
                Err(crate::context_menu::MenuError::Foreign)
            );
            assert_eq!(disable(&menu, &ours), Ok(()));
            let still = foreign.get_value::<String, _>("").expect("valore");
            assert_eq!(still, r#""C:\altro\programma.exe" "%1""#);
        }

        /// Lo stesso registro, con la cartella dell'eseguibile considerata sicura (quella
        /// dell'eseguibile di prova non esiste).
        struct NotWritable<'a>(&'a ContextMenu);

        impl ContextMenuStore for NotWritable<'_> {
            fn state(&self, entry: &MenuEntry) -> io::Result<MenuState> {
                self.0.state(entry)
            }
            fn install(&self, entry: &MenuEntry) -> io::Result<()> {
                self.0.install(entry)
            }
            fn remove(&self, entry: &MenuEntry) -> io::Result<()> {
                self.0.remove(entry)
            }
            fn exe_dir_writable_by_others(&self, _exe: &Path) -> io::Result<bool> {
                Ok(false)
            }
        }
    }
}

#[cfg(not(windows))]
mod context_menu_nautilus {
    use std::fs::{self, DirBuilder, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
    use std::path::{Path, PathBuf};

    use crate::context_menu::{ContextMenuStore, MenuEntry, MenuState, classify_scripts};

    /// Byte massimi letti da uno script esistente.
    const MAX_SCRIPT_BYTES: u64 = 64 * 1024;

    /// Gli script di Nautilus in `~/.local/share/nautilus/scripts`.
    pub struct ContextMenu {
        scripts: PathBuf,
    }

    impl ContextMenu {
        /// La cartella degli script dell'utente.
        pub fn for_current_user() -> io::Result<ContextMenu> {
            let base = directories::BaseDirs::new()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "cartella utente"))?;
            Ok(ContextMenu {
                scripts: base.data_dir().join("nautilus").join("scripts"),
            })
        }

        /// Gli script in `dir` (test su una cartella propria).
        pub fn with_scripts_dir(dir: PathBuf) -> ContextMenu {
            ContextMenu { scripts: dir }
        }

        /// La cartella degli script, creata (0700) se manca; rifiutata se è di altri o
        /// scrivibile da gruppo e altri.
        fn checked_dir(&self) -> io::Result<&Path> {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&self.scripts)?;
            let meta = fs::symlink_metadata(&self.scripts)?;
            let mine = meta.uid() == rustix::process::geteuid().as_raw();
            if !meta.is_dir() || !mine || meta.mode() & 0o022 != 0 {
                return Err(lynkeus_engine::NotPrivate::error());
            }
            Ok(&self.scripts)
        }

        fn names(entry: &MenuEntry) -> impl Iterator<Item = &str> {
            std::iter::once(entry.label()).chain(entry.other_labels().iter().map(String::as_str))
        }

        /// Il contenuto dei file con i nomi della voce, se ci sono (senza seguire
        /// collegamenti).
        fn found(&self, entry: &MenuEntry) -> io::Result<Vec<(String, String)>> {
            let mut found = Vec::new();
            for name in Self::names(entry) {
                if !safe_file_name(name) {
                    continue;
                }
                let opened = OpenOptions::new()
                    .read(true)
                    .custom_flags(i32::try_from(rustix::fs::OFlags::NOFOLLOW.bits()).unwrap_or(0))
                    .open(self.scripts.join(name));
                let mut file = match opened {
                    Ok(file) => file,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(_) => {
                        // Un collegamento o un file illeggibile con il nostro nome: altrui.
                        found.push((name.to_owned(), String::new()));
                        continue;
                    }
                };
                let mut bytes = Vec::new();
                (&mut file).take(MAX_SCRIPT_BYTES).read_to_end(&mut bytes)?;
                found.push((
                    name.to_owned(),
                    String::from_utf8_lossy(&bytes).into_owned(),
                ));
            }
            Ok(found)
        }
    }

    /// Un nome di file senza separatori, punti iniziali né caratteri di controllo.
    fn safe_file_name(name: &str) -> bool {
        !name.is_empty()
            && !name.starts_with('.')
            && !name.contains('/')
            && !name.chars().any(char::is_control)
    }

    impl ContextMenuStore for ContextMenu {
        fn state(&self, entry: &MenuEntry) -> io::Result<MenuState> {
            Ok(classify_scripts(&self.found(entry)?, entry))
        }

        fn install(&self, entry: &MenuEntry) -> io::Result<()> {
            if !safe_file_name(entry.label()) {
                return Err(io::Error::from(io::ErrorKind::InvalidInput));
            }
            let dir = self.checked_dir()?;
            // Temporaneo nuovo (0700, senza seguire collegamenti), poi rinomina.
            let temp = dir.join(format!(".lynkeus-searcher-{}.tmp", std::process::id()));
            let _stale = fs::remove_file(&temp);
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o700)
                .custom_flags(i32::try_from(rustix::fs::OFlags::NOFOLLOW.bits()).unwrap_or(0))
                .open(&temp)?;
            let written = file
                .write_all(entry.nautilus_script().as_bytes())
                .and_then(|()| file.sync_all())
                .and_then(|()| fs::rename(&temp, dir.join(entry.label())));
            if let Err(error) = written {
                let _removed = fs::remove_file(&temp);
                return Err(error);
            }
            // Gli script nostri con il nome di un'altra lingua.
            for (name, content) in self.found(entry)? {
                if name != entry.label()
                    && content.lines().nth(1) == Some(crate::context_menu::SCRIPT_MARKER)
                {
                    fs::remove_file(dir.join(name))?;
                }
            }
            Ok(())
        }

        fn remove(&self, entry: &MenuEntry) -> io::Result<()> {
            for (name, content) in self.found(entry)? {
                if content.lines().nth(1) == Some(crate::context_menu::SCRIPT_MARKER) {
                    fs::remove_file(self.scripts.join(name))?;
                }
            }
            Ok(())
        }

        fn exe_dir_writable_by_others(&self, exe: &Path) -> io::Result<bool> {
            let dir = exe
                .parent()
                .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
            let meta = fs::metadata(dir)?;
            let owner_ok = meta.uid() == 0 || meta.uid() == rustix::process::geteuid().as_raw();
            Ok(!owner_ok || meta.mode() & 0o022 != 0)
        }
    }

    #[cfg(test)]
    mod tests {
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;

        use super::*;

        fn entry(exe: &Path) -> MenuEntry {
            MenuEntry::new(
                exe,
                "Cerca con Lynkeus",
                &["Search with Lynkeus".to_owned()],
            )
            .expect("voce")
        }

        /// Lo script generato, eseguito con nomi di file ostili, passa all'eseguibile
        /// esattamente i percorsi ricevuti.
        #[test]
        fn the_script_passes_paths_verbatim() {
            let dir = tempfile::tempdir().expect("cartella");
            let bin = dir.path().join("bin con 'apice'");
            fs::create_dir(&bin).expect("cartella");
            let out = dir.path().join("argomenti");
            let fake = bin.join("lynkeus-searcher");
            fs::write(
                &fake,
                format!(
                    "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\0' \"$a\"; done > '{}'\n",
                    out.display()
                ),
            )
            .expect("eseguibile finto");
            fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).expect("chmod");
            let menu = ContextMenu::with_scripts_dir(dir.path().join("scripts"));
            let entry = entry(&fake);
            menu.install(&entry).expect("installato");
            assert_eq!(menu.state(&entry).expect("stato"), MenuState::Current);
            let script = dir.path().join("scripts").join(entry.label());
            let mode = fs::metadata(&script).expect("script").permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
            let names = ["a'b", "$(x)", "-r", "a b", "riga\nnuova", "--add"];
            let status = Command::new("/bin/sh")
                .arg(&script)
                .args(names)
                .status()
                .expect("script eseguito");
            assert!(status.success());
            let received = fs::read(&out).expect("argomenti");
            let received: Vec<&[u8]> = received
                .split(|b| *b == 0)
                .filter(|a| !a.is_empty())
                .collect();
            let mut expected: Vec<&[u8]> = vec![b"--add", b"--"];
            expected.extend(names.iter().map(|n| n.as_bytes()));
            assert_eq!(received, expected);
            menu.remove(&entry).expect("tolto");
            assert_eq!(menu.state(&entry).expect("stato"), MenuState::Missing);
        }
    }
}
