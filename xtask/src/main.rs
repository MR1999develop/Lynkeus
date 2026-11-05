// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Automazione del progetto, avviata con `cargo xtask <comando>`.
//!
//! - `check`: gli stessi controlli in locale e in CI (nessun avvio di processi nei decoder,
//!   fmt, clippy, test, cargo-deny);
//! - `check-linux`: controllo di compilazione per Linux da qualsiasi macchina;
//! - `dist`: build di rilascio verificata e pacchetto zip con `SHA256SUMS`;
//! - `release`: come `dist`, con `cargo auditable` (elenco delle dipendenze
//!   nell'eseguibile) e SBOM CycloneDX: il rilascio in CI da un tag;
//! - `deny-review`: le eccezioni di `deny.toml` hanno una data di revisione non passata;
//! - `mitigations <exe>`: mitigazioni di un eseguibile Windows e prova con DLL finte.
//!
//! I programmi esterni si avviano sempre con argomenti separati, mai tramite shell.

mod check;
mod deny_review;
mod dist;
mod nospawn;
mod pe;
mod process;
mod remap;
mod scan;

use std::process::ExitCode;

const USAGE: &str = "\
Uso: cargo xtask <comando>

Comandi:
  check            nessun processo nei decoder, date delle eccezioni di deny.toml, fmt,
                   clippy -D warnings, test e cargo deny, fermandosi al primo errore
  check-linux      cargo check del workspace per x86_64-unknown-linux-gnu
  dist             build di rilascio verificata, zip e SHA256SUMS in dist/
  release          come dist, con cargo auditable e SBOM CycloneDX (rilascio in CI;
                   servono cargo-auditable e cargo-cyclonedx)
  deny-review      le eccezioni di deny.toml hanno una data di revisione non passata
  mitigations EXE  mitigazioni, DLL importate e prova con DLL finte di un eseguibile
                   Windows";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["check"] => check::run_all(),
        ["check-linux"] => check::run_linux(),
        ["dist"] => dist::run(dist::Mode::Local),
        ["release"] => dist::run(dist::Mode::Release),
        ["deny-review"] => deny_review::run(),
        ["mitigations", exe] => dist::print_mitigations(std::path::Path::new(exe)),
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
