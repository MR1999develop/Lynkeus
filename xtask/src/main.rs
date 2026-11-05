// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Automazione del progetto, avviata con `cargo xtask <comando>`.
//!
//! - `check`: gli stessi controlli in locale e in CI (nessun avvio di processi nei decoder,
//!   fmt, clippy, test, cargo-deny);
//! - `check-linux`: controllo di compilazione per Linux da qualsiasi macchina;
//! - `dist`: build di rilascio verificata e pacchetto zip con `SHA256SUMS`.
//!
//! I programmi esterni si avviano sempre con argomenti separati, mai tramite shell.

mod check;
mod dist;
mod nospawn;
mod process;
mod remap;
mod scan;

use std::process::ExitCode;

const USAGE: &str = "\
Uso: cargo xtask <comando>

Comandi:
  check        nessun processo nei decoder, fmt, clippy -D warnings, test e cargo deny,
               fermandosi al primo errore
  check-linux  cargo check del workspace per x86_64-unknown-linux-gnu
  dist         build di rilascio verificata, zip e SHA256SUMS in dist/";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["check"] => check::run_all(),
        ["check-linux"] => check::run_linux(),
        ["dist"] => dist::run(),
        ["help" | "--help" | "-h"] => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: errore: {error:#}");
            ExitCode::FAILURE
        }
    }
}
