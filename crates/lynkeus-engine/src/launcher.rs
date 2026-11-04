// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Avvio di un processo estrattore, dietro un trait di ruolo: l'engine resta senza `unsafe`
//! e non sa come il sistema crea il processo.
//!
//! - [`WorkerLauncher`] avvia un processo e restituisce i suoi capi di stdin e stdout più un
//!   [`WorkerProcess`] per terminarlo e conoscerne l'uscita.
//! - [`CommandLauncher`] è l'implementazione portabile con `std::process::Command`, usata su
//!   Linux e nei test: ambiente svuotato salvo [`platform::KEPT_ENV_VARS`], stdin e stdout
//!   collegati, stderr nullo, cartella corrente = cartella dell'eseguibile.
//! - Su Windows l'app usa un'implementazione sua (`CreateProcessW` dentro un Job Object con
//!   limite di memoria), perché `Command` fa ereditare al figlio tutti gli handle
//!   ereditabili del padre (experiments/jobobject/ESITI.md).
//!
//! Il processo estrattore è l'eseguibile dell'app con [`EXTRACT_WORKER_FLAG`] seguito da
//! [`worker_args`].

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use lynkeus_core::ByteSize;
use lynkeus_extract::worker::WorkerConfig;

use crate::platform;

/// Opzione nascosta che avvia l'eseguibile come processo estrattore.
pub const EXTRACT_WORKER_FLAG: &str = "--extract-worker";

/// Gli argomenti del processo estrattore: [`EXTRACT_WORKER_FLAG`] e il limite di memoria
/// (`--memory-limit-bytes`), che il figlio impone a sé stesso dove il padre non può (Linux).
pub fn worker_args(memory_limit: Option<ByteSize>) -> Vec<OsString> {
    let mut args = vec![OsString::from(EXTRACT_WORKER_FLAG)];
    args.extend(WorkerConfig { memory_limit }.to_args());
    args
}

/// Come è finito un processo estrattore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerExit {
    /// Uscito con successo (stdin chiuso dal padre).
    Success,
    /// Ucciso dal sistema perché ha superato il limite di memoria (dove riconoscibile: su
    /// Windows dal Job Object; su Linux un'allocazione rifiutata sembra un aborto qualsiasi).
    MemoryExceeded,
    /// Uscito con errore, abortito o terminato; `code` se il sistema lo dice.
    Failed {
        /// Codice d'uscita (su Windows il valore a 32 bit, es. `0xC0000409`).
        code: Option<i64>,
    },
}

/// Un processo estrattore avviato: lo possiede chi lo usa.
///
/// Chiuderlo (drop) non deve lasciarlo vivo: le implementazioni lo terminano.
pub trait WorkerProcess: Send {
    /// Aspetta l'uscita al massimo `timeout`; `Ok(None)` se è ancora vivo.
    fn wait_timeout(&mut self, timeout: Duration) -> io::Result<Option<WorkerExit>>;

    /// Termina il processo subito; nessun errore se è già uscito.
    fn kill(&mut self) -> io::Result<()>;
}

/// Un processo estrattore appena avviato, con i capi dei suoi canali.
pub struct LaunchedWorker {
    /// Lo stdin del figlio: chiuderlo è la richiesta di fermarsi.
    pub stdin: Box<dyn Write + Send>,
    /// Lo stdout del figlio, riservato al protocollo.
    pub stdout: Box<dyn Read + Send>,
    /// Il processo.
    pub process: Box<dyn WorkerProcess>,
}

impl std::fmt::Debug for LaunchedWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchedWorker").finish_non_exhaustive()
    }
}

/// Avvia processi estrattori.
pub trait WorkerLauncher: Send + Sync {
    /// Avvia un processo nuovo.
    fn launch(&self) -> io::Result<LaunchedWorker>;
}

/// Avvio portabile con `std::process::Command` (Linux e test).
#[derive(Debug, Clone)]
pub struct CommandLauncher {
    exe: PathBuf,
    args: Vec<OsString>,
}

impl CommandLauncher {
    /// Un avvio di `exe` (percorso assoluto) con [`worker_args`] di `memory_limit`.
    ///
    /// Errore se `exe` non è assoluto: un processo non si cerca mai nel `PATH` né nella
    /// cartella corrente.
    pub fn new(exe: PathBuf, memory_limit: Option<ByteSize>) -> io::Result<CommandLauncher> {
        CommandLauncher::with_args(exe, worker_args(memory_limit))
    }

    /// Un avvio di `exe` (assoluto) con argomenti scelti dal chiamante (test).
    pub fn with_args(exe: PathBuf, args: Vec<OsString>) -> io::Result<CommandLauncher> {
        if !exe.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "eseguibile del processo estrattore senza percorso assoluto",
            ));
        }
        Ok(CommandLauncher { exe, args })
    }

    /// L'eseguibile avviato.
    pub fn exe(&self) -> &Path {
        &self.exe
    }

    /// Il comando che verrebbe eseguito (senza avviarlo).
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.exe);
        command
            .args(&self.args)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for name in platform::KEPT_ENV_VARS {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        // Una cartella dell'utente non resta aperta né fa da base a percorsi relativi.
        if let Some(dir) = self.exe.parent() {
            command.current_dir(dir);
        }
        command
    }
}

impl WorkerLauncher for CommandLauncher {
    fn launch(&self) -> io::Result<LaunchedWorker> {
        let mut child = self.command().spawn()?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            // Non succede con `Stdio::piped`; il figlio non resta vivo comunque.
            let _killed = child.kill();
            let _reaped = child.wait();
            return Err(io::Error::other("canali del processo estrattore mancanti"));
        };
        Ok(LaunchedWorker {
            stdin: Box::new(stdin),
            stdout: Box::new(stdout),
            process: Box::new(CommandProcess {
                child,
                exited: None,
            }),
        })
    }
}

/// Intervallo dei controlli di [`CommandProcess::wait_timeout`]: `std` non ha un'attesa con
/// scadenza, e l'attesa serve solo in chiusura (al massimo 500 ms).
const WAIT_POLL: Duration = Duration::from_millis(5);

/// Un processo avviato con [`CommandLauncher`].
struct CommandProcess {
    child: Child,
    exited: Option<WorkerExit>,
}

impl WorkerProcess for CommandProcess {
    fn wait_timeout(&mut self, timeout: Duration) -> io::Result<Option<WorkerExit>> {
        if let Some(exit) = self.exited {
            return Ok(Some(exit));
        }
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait()? {
                let exit = if status.success() {
                    WorkerExit::Success
                } else {
                    WorkerExit::Failed {
                        code: status.code().map(i64::from),
                    }
                };
                self.exited = Some(exit);
                return Ok(Some(exit));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            thread::sleep(left.min(WAIT_POLL));
        }
    }

    fn kill(&mut self) -> io::Result<()> {
        if self.exited.is_some() {
            return Ok(());
        }
        match self.child.kill() {
            Ok(()) => Ok(()),
            // Già uscito: `kill` su un processo raccolto o finito non è un errore qui.
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => Ok(()),
            Err(error) => Err(error),
        }
    }
}

impl Drop for CommandProcess {
    fn drop(&mut self) {
        if self.exited.is_none() {
            // Rete di sicurezza: nessun figlio resta vivo né da raccogliere.
            let _killed = self.kill();
            let _reaped = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_executables_are_refused() {
        let error = CommandLauncher::new(PathBuf::from("lynkeus-searcher"), None);
        assert_eq!(
            error.map(|_| ()).map_err(|e| e.kind()),
            Err(io::ErrorKind::InvalidInput)
        );
    }

    #[test]
    fn worker_args_carry_the_memory_limit() {
        assert_eq!(worker_args(None), [OsString::from(EXTRACT_WORKER_FLAG)]);
        let args = worker_args(Some(ByteSize::from_mib(256)));
        assert_eq!(
            args,
            [
                OsString::from(EXTRACT_WORKER_FLAG),
                OsString::from("--memory-limit-bytes"),
                OsString::from("268435456"),
            ]
        );
    }

    #[test]
    fn the_command_has_a_minimal_environment() {
        let exe = std::env::current_exe().expect("eseguibile del test");
        let launcher = CommandLauncher::new(exe.clone(), None).expect("assoluto");
        let command = launcher.command();
        let names: Vec<_> = command
            .get_envs()
            .map(|(name, _)| name.to_os_string())
            .collect();
        for name in &names {
            assert!(
                platform::KEPT_ENV_VARS
                    .iter()
                    .any(|kept| name.as_os_str() == *kept),
                "{name:?}"
            );
        }
        assert_eq!(command.get_program(), exe.as_os_str());
        assert_eq!(command.get_current_dir(), exe.parent());
    }
}
