// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Avvio di un comando con stdout e stderr nello stesso canale: le righe si mostrano su
//! stderr mentre arrivano (chi lancia il piano vede l'avanzamento) e si conservano, entro un
//! limite, per leggerle dopo. Ogni comando ha una scadenza.

use std::io::{BufRead, BufReader, Read, Write as _};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// Testo conservato al massimo per comando; oltre, le righe si mostrano ma non si tengono.
const MAX_KEPT_BYTES: usize = 64 * 1024 * 1024;
/// Lunghezza massima di una riga letta: una riga più lunga arriva a pezzi.
const MAX_LINE_BYTES: u64 = 64 * 1024;
/// Righe in attesa tra il thread che legge e chi le consuma.
const LINE_QUEUE: usize = 1024;

/// Quali righe mostrare su stderr mentre arrivano.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Echo {
    /// Tutte.
    All,
    /// Tutte tranne quelle dell'elenco dei test (`nome: test`), migliaia e senza interesse.
    WithoutTestList,
    /// Nessuna.
    Nothing,
}

impl Echo {
    fn shows(self, line: &str) -> bool {
        match self {
            Echo::All => true,
            Echo::WithoutTestList => !(line.ends_with(": test") || line.ends_with(": bench")),
            Echo::Nothing => false,
        }
    }
}

/// Esito di un comando.
#[derive(Debug)]
pub struct Captured {
    /// Lo stato d'uscita (assente se non si è potuto attendere dopo la scadenza).
    pub status: Option<ExitStatus>,
    /// stdout e stderr insieme, una riga per riga letta (senza `\r`).
    pub output: String,
    pub elapsed: Duration,
    /// Vero se è scaduto il tempo e il processo è stato terminato.
    pub timed_out: bool,
    /// Vero se parte dell'uscita non è stata conservata ([`MAX_KEPT_BYTES`]).
    pub truncated: bool,
}

impl Captured {
    /// Vero se il comando è finito in tempo con successo.
    pub fn success(&self) -> bool {
        !self.timed_out && self.status.is_some_and(|status| status.success())
    }

    /// Lo stato in parole, per i messaggi.
    pub fn status_text(&self) -> String {
        if self.timed_out {
            return format!("scaduto dopo {:.0} s", self.elapsed.as_secs_f64());
        }
        self.status
            .map_or_else(|| "stato sconosciuto".to_owned(), |s| s.to_string())
    }

    /// Le ultime `count` righe dell'uscita.
    pub fn tail(&self, count: usize) -> String {
        let lines: Vec<&str> = self.output.lines().collect();
        let start = lines.len().saturating_sub(count);
        lines[start..].join("\n")
    }
}

/// Esegue `command` (stdin chiuso) fino alla fine o fino a `timeout`; allo scadere termina
/// il processo avviato (i suoi figli, se ne ha, possono restare fino alla loro fine).
pub fn run(mut command: Command, echo: Echo, timeout: Duration) -> Result<Captured> {
    let description = describe(&command);
    let (reader, writer) = std::io::pipe().context("creazione del canale dell'uscita")?;
    let writer_for_stderr = writer.try_clone().context("copia del canale dell'uscita")?;
    command
        .stdin(Stdio::null())
        .stdout(writer)
        .stderr(writer_for_stderr);
    let started = Instant::now();
    let mut child = command
        .spawn()
        .with_context(|| format!("impossibile avviare: {description}"))?;
    // Il `Command` tiene le copie del lato di scrittura: chiuse queste, la lettura finisce
    // quando il processo (e chi ha ereditato il canale) ha finito.
    drop(command);

    let (lines_tx, lines_rx) = mpsc::sync_channel::<String>(LINE_QUEUE);
    thread::Builder::new()
        .name("uscita del comando".to_owned())
        .spawn(move || read_lines(reader, &lines_tx))
        .context("avvio del thread di lettura")?;

    let deadline = started + timeout;
    let mut output = String::new();
    let mut truncated = false;
    let mut timed_out = false;
    let mut stderr = std::io::stderr().lock();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match lines_rx.recv_timeout(left) {
            Ok(line) => {
                if echo.shows(&line) && writeln!(stderr, "{line}").is_err() {
                    // stderr chiuso: si continua a raccogliere senza mostrare.
                }
                if output.len().saturating_add(line.len()) < MAX_KEPT_BYTES {
                    output.push_str(&line);
                    output.push('\n');
                } else {
                    truncated = true;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                timed_out = true;
                if let Err(error) = child.kill() {
                    // Di solito già uscito: lo stato lo dice l'attesa che segue.
                    eprintln!("xtask: terminazione di {description} non riuscita: {error}");
                }
                break;
            }
        }
    }
    drop(stderr);
    let status = if timed_out {
        child.wait().ok()
    } else {
        Some(
            child
                .wait()
                .with_context(|| format!("attesa di: {description}"))?,
        )
    };
    Ok(Captured {
        status,
        output,
        elapsed: started.elapsed(),
        timed_out,
        truncated,
    })
}

/// Legge righe (a pezzi di al massimo [`MAX_LINE_BYTES`]) fino alla fine del canale o fino
/// a quando nessuno le riceve più.
fn read_lines(reader: impl Read, lines: &mpsc::SyncSender<String>) {
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match reader
            .by_ref()
            .take(MAX_LINE_BYTES)
            .read_until(b'\n', &mut buffer)
        {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                let text = String::from_utf8_lossy(&buffer);
                let line = text.trim_end_matches(['\r', '\n']).to_owned();
                if lines.send(line).is_err() {
                    return;
                }
            }
        }
    }
}

/// Programma e argomenti, per i messaggi.
pub fn describe(command: &Command) -> String {
    let program = std::path::Path::new(command.get_program())
        .file_name()
        .map_or_else(
            || command.get_program().to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
    let args: Vec<String> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    format!("{program} {}", args.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_partitions() {
        assert!(Echo::All.shows("x: test"));
        assert!(!Echo::WithoutTestList.shows("a::b: test"));
        assert!(!Echo::WithoutTestList.shows("a: bench"));
        assert!(Echo::WithoutTestList.shows("test a ... ok"));
        assert!(!Echo::Nothing.shows("test a ... ok"));
    }

    /// Il cargo che esegue i test: stdout e stderr arrivano entrambi, lo stato è quello vero.
    #[test]
    fn output_of_both_streams_is_kept() {
        let mut command = crate::process::cargo();
        command.args(["--version", "--verbose"]);
        let captured = run(command, Echo::Nothing, Duration::from_secs(60)).expect("cargo");
        assert!(captured.success(), "{}", captured.status_text());
        assert!(captured.output.starts_with("cargo "), "{}", captured.output);
        assert!(!captured.timed_out && !captured.truncated);
    }

    #[test]
    fn a_failing_command_is_not_a_success() {
        let mut command = crate::process::cargo();
        command.arg("comando-che-non-esiste-xtask");
        let captured = run(command, Echo::Nothing, Duration::from_secs(60)).expect("cargo");
        assert!(!captured.success());
        assert!(!captured.output.is_empty(), "il messaggio d'errore arriva");
    }
}
