// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! L'ambiente delle misure, per il rapporto: commit, sistema, CPU e core, RAM, tipo di
//! disco, antivirus (se si legge senza privilegi), toolchain e profilo di build.
//!
//! Solo dati tecnici: niente nome del computer, nome utente o percorsi locali. Ciò che non
//! si riesce a leggere diventa "non rilevato", mai un errore del piano.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::capture::{self, Echo};
use crate::process::cargo;

/// Tempo massimo di ogni strumento interrogato.
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

/// Il testo per un dato che non si è potuto leggere.
pub const UNKNOWN: &str = "non rilevato";

/// L'ambiente: coppie (voce, valore) nell'ordine del rapporto.
#[derive(Debug, Clone)]
pub struct Environment {
    pub fields: Vec<(&'static str, String)>,
}

impl Environment {
    /// Il valore di `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Legge l'ambiente di questa macchina.
pub fn collect(root: &Path) -> Environment {
    let system = platform::probe();
    let cores = std::thread::available_parallelism()
        .map_or_else(|_| UNKNOWN.to_owned(), |n| n.get().to_string());
    let xtask_profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    Environment {
        fields: vec![
            ("Commit", commit(root)),
            ("Sistema", system.os),
            ("CPU", system.cpu),
            ("Core logici", cores),
            ("RAM", system.ram),
            ("Dischi", system.disks),
            ("Antivirus", system.antivirus),
            ("Toolchain", toolchain()),
            (
                "Profilo",
                format!("release (suite e misure); xtask in {xtask_profile}"),
            ),
        ],
    }
}

/// Dati letti dal sistema.
struct SystemProbe {
    os: String,
    cpu: String,
    ram: String,
    disks: String,
    antivirus: String,
}

/// `cargo --version` del cargo che esegue xtask.
fn toolchain() -> String {
    let mut command = cargo();
    command.arg("--version");
    first_line(command).unwrap_or_else(|| UNKNOWN.to_owned())
}

/// Il commit: `GITHUB_SHA` in CI, altrimenti `git rev-parse`, con l'indicazione delle
/// modifiche non salvate in un commit.
fn commit(root: &Path) -> String {
    if let Ok(sha) = std::env::var("GITHUB_SHA")
        && sha.len() >= 12
        && sha.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return sha[..12].to_owned();
    }
    let Some(git) = find_in_path("git") else {
        return UNKNOWN.to_owned();
    };
    let mut head = Command::new(&git);
    head.current_dir(root)
        .args(["rev-parse", "--short=12", "HEAD"]);
    let Some(hash) = first_line(head) else {
        return UNKNOWN.to_owned();
    };
    let mut status = Command::new(&git);
    status
        .current_dir(root)
        .args(["status", "--porcelain", "--untracked-files=no"]);
    match capture::run(status, Echo::Nothing, PROBE_TIMEOUT) {
        Ok(out) if out.success() && out.output.trim().is_empty() => hash,
        Ok(out) if out.success() => format!("{hash} con modifiche locali"),
        _ => format!("{hash} (stato del lavoro non rilevato)"),
    }
}

/// Un programma cercato solo nelle cartelle assolute del `PATH` (mai nella cartella
/// corrente), per percorso completo.
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
}

/// La prima riga non vuota dell'uscita di `command`, se riesce.
fn first_line(command: Command) -> Option<String> {
    let captured = capture::run(command, Echo::Nothing, PROBE_TIMEOUT).ok()?;
    if !captured.success() {
        return None;
    }
    captured
        .output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

/// GiB con un decimale e la virgola.
fn gib(bytes: u64) -> String {
    // Solo per la stampa: la precisione di f64 basta.
    let value = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    format!("{value:.1} GiB").replace('.', ",")
}

/// Lo stato di un antivirus da `productState` del Centro sicurezza di Windows: il secondo
/// byte vale 0x10 o 0x11 quando la protezione in tempo reale è attiva.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "usato solo su Windows e nei test")
)]
pub fn antivirus_enabled(product_state: u32) -> bool {
    (product_state >> 8) & 0x10 != 0
}

#[cfg(windows)]
mod platform {
    use std::path::PathBuf;
    use std::process::Command;

    use super::{SystemProbe, UNKNOWN, antivirus_enabled, capture, gib};

    /// Lo script di PowerShell: righe `chiave=valore`, nessun dato dell'utente.
    const SCRIPT: &str = r"
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$ErrorActionPreference = 'SilentlyContinue'
$os = Get-CimInstance Win32_OperatingSystem
'os=' + $os.Caption + ' ' + $os.Version
'cpu=' + (Get-CimInstance Win32_Processor | Select-Object -First 1).Name
'ram=' + (Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory
foreach ($d in Get-PhysicalDisk) { 'disk=' + $d.MediaType }
foreach ($a in Get-CimInstance -Namespace root/SecurityCenter2 -ClassName AntiVirusProduct) {
  'av=' + $a.displayName + '|' + $a.productState
}
$mp = Get-MpComputerStatus
if ($mp) { 'defender=' + $mp.RealTimeProtectionEnabled }
";

    /// PowerShell di sistema, per percorso assoluto.
    pub fn powershell() -> Option<PathBuf> {
        let root = std::env::var_os("SystemRoot")?;
        let exe = PathBuf::from(root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        exe.is_file().then_some(exe)
    }

    pub fn probe() -> SystemProbe {
        let values = powershell()
            .and_then(|exe| {
                let mut command = Command::new(exe);
                command.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT]);
                capture::run(command, capture::Echo::Nothing, super::PROBE_TIMEOUT).ok()
            })
            .map(|captured| captured.output)
            .unwrap_or_default();
        let all = |key: &str| -> Vec<String> {
            values
                .lines()
                .filter_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .collect()
        };
        let first = |key: &str| {
            all(key)
                .into_iter()
                .next()
                .unwrap_or_else(|| UNKNOWN.to_owned())
        };
        let ram = all("ram")
            .first()
            .and_then(|v| v.parse::<u64>().ok())
            .map_or_else(|| UNKNOWN.to_owned(), gib);
        let disks = all("disk");
        SystemProbe {
            os: first("os"),
            cpu: first("cpu"),
            ram,
            disks: if disks.is_empty() {
                UNKNOWN.to_owned()
            } else {
                disks.join(", ")
            },
            antivirus: antivirus(&all("av"), all("defender").first().map(String::as_str)),
        }
    }

    /// Gli antivirus registrati nel Centro sicurezza (non c'è su Windows Server) o, in
    /// mancanza, Microsoft Defender.
    fn antivirus(products: &[String], defender_realtime: Option<&str>) -> String {
        let listed: Vec<String> = products
            .iter()
            .filter_map(|entry| {
                let (name, state) = entry.rsplit_once('|')?;
                let state: u32 = state.trim().parse().ok()?;
                let status = if antivirus_enabled(state) {
                    "attivo"
                } else {
                    "spento"
                };
                Some(format!("{} ({status})", name.trim()))
            })
            .collect();
        if !listed.is_empty() {
            return listed.join(", ");
        }
        match defender_realtime {
            Some("True") => "Microsoft Defender (protezione in tempo reale attiva)".to_owned(),
            Some("False") => "Microsoft Defender (protezione in tempo reale spenta)".to_owned(),
            _ => format!("{UNKNOWN} senza privilegi"),
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{SystemProbe, UNKNOWN, gib};

    pub fn probe() -> SystemProbe {
        let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let pretty = os_release
            .lines()
            .find_map(|line| line.strip_prefix("PRETTY_NAME="))
            .map(|value| value.trim_matches('"').to_owned());
        let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|k| k.trim().to_owned())
            .ok();
        let os = match (pretty, kernel) {
            (Some(name), Some(kernel)) => format!("{name}, kernel {kernel}"),
            (Some(name), None) => name,
            (None, Some(kernel)) => format!("Linux, kernel {kernel}"),
            (None, None) => UNKNOWN.to_owned(),
        };
        let cpu = std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|info| {
                info.lines()
                    .find_map(|line| line.strip_prefix("model name"))
                    .and_then(|rest| rest.split_once(':'))
                    .map(|(_, name)| name.trim().to_owned())
            })
            .unwrap_or_else(|| UNKNOWN.to_owned());
        let ram = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|info| {
                let line = info.lines().find(|l| l.starts_with("MemTotal:"))?;
                let kib: u64 = line
                    .trim_start_matches("MemTotal:")
                    .trim()
                    .trim_end_matches("kB")
                    .trim()
                    .parse()
                    .ok()?;
                kib.checked_mul(1024)
            })
            .map_or_else(|| UNKNOWN.to_owned(), gib);
        SystemProbe {
            os,
            cpu,
            ram,
            disks: disks(),
            antivirus: format!("{UNKNOWN} (nessuna interfaccia comune su Linux)"),
        }
    }

    /// SSD o HDD dei dischi fisici (`/sys/block/*/queue/rotational`).
    fn disks() -> String {
        let Ok(entries) = std::fs::read_dir("/sys/block") else {
            return UNKNOWN.to_owned();
        };
        let mut kinds: Vec<&str> = entries
            .flatten()
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                !(name.starts_with("loop") || name.starts_with("ram") || name.starts_with("zram"))
            })
            .filter_map(|entry| {
                let rotational =
                    std::fs::read_to_string(entry.path().join("queue/rotational")).ok()?;
                Some(if rotational.trim() == "1" {
                    "HDD"
                } else {
                    "SSD"
                })
            })
            .collect();
        kinds.sort_unstable();
        if kinds.is_empty() {
            UNKNOWN.to_owned()
        } else {
            kinds.join(", ")
        }
    }
}

#[cfg(windows)]
pub use platform::powershell;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_state_partitions() {
        // Valori tipici del Centro sicurezza: 0x061100 attivo, 0x060100 spento,
        // 0x041000 attivo, 0x040000 spento.
        assert!(antivirus_enabled(0x0006_1100));
        assert!(!antivirus_enabled(0x0006_0100));
        assert!(antivirus_enabled(0x0004_1000));
        assert!(!antivirus_enabled(0x0004_0000));
    }

    #[test]
    fn gib_uses_one_decimal_and_the_comma() {
        assert_eq!(gib(16 * 1024 * 1024 * 1024), "16,0 GiB");
        assert_eq!(gib(1024 * 1024 * 1024 / 2), "0,5 GiB");
    }
}
