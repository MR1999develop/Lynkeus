// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Avvio di programmi esterni con argomenti separati (mai tramite shell).

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Il cargo che ha avviato xtask (variabile `CARGO`), altrimenti `cargo` dal `PATH`.
pub fn cargo() -> Command {
    let program = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    Command::new(program)
}

/// Esegue `command` con stdin, stdout e stderr ereditati; errore se esce con un codice non zero.
pub fn run_inherited(command: &mut Command, description: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("impossibile avviare: {description}"))?;
    if !status.success() {
        bail!("{description} è terminato con {status}");
    }
    Ok(())
}

/// Esegue `command` e restituisce lo stdout come testo; errore se esce con un codice non zero.
pub fn run_captured(command: &mut Command, description: &str) -> Result<String> {
    let output = command
        .output()
        .with_context(|| format!("impossibile avviare: {description}"))?;
    if !output.status.success() {
        bail!(
            "{description} è terminato con {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).with_context(|| format!("uscita non UTF-8 da: {description}"))
}

/// Directory radice del workspace, chiesta a cargo al momento.
pub fn workspace_root() -> Result<std::path::PathBuf> {
    let manifest = run_captured(
        cargo().args(["locate-project", "--workspace", "--message-format", "plain"]),
        "cargo locate-project",
    )?;
    let manifest = Path::new(manifest.trim());
    manifest
        .parent()
        .map(Path::to_path_buf)
        .context("percorso del manifest del workspace senza cartella")
}
