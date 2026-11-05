// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! `cargo xtask dist`: build di rilascio verificata e pacchetto per la distribuzione.
//!
//! Passi: build `--locked` di `lynkeus-app` con i percorsi locali riscritti
//! (vedi [`crate::remap`]); su Windows verifica di CFG e CETCOMPAT con `dumpbin`;
//! ricerca di nome utente e percorsi locali nei byte dell'eseguibile; zip con
//! eseguibile, `LICENSE` e `README.md`; file `SHA256SUMS`.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use sha2::{Digest as _, Sha256};
use zip::write::SimpleFileOptions;

use crate::process::{cargo, run_captured, run_inherited, workspace_root};
use crate::remap;
use crate::scan::{self, Needle};

const APP_PACKAGE: &str = "lynkeus-app";
const APP_BINARY: &str = "lynkeus-searcher";
const DIST_DIR: &str = "dist";
const CHECKSUMS_FILE: &str = "SHA256SUMS";
const VSWHERE: &str = r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe";

/// La piattaforma di destinazione, uguale a quella su cui gira xtask.
struct Platform {
    triple: &'static str,
    os: &'static str,
    exe_name: String,
}

impl Platform {
    fn host() -> Result<Self> {
        let (triple, os) = if cfg!(all(windows, target_arch = "x86_64")) {
            ("x86_64-pc-windows-msvc", "windows")
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            ("x86_64-unknown-linux-gnu", "linux")
        } else {
            bail!("dist è supportato solo su Windows e Linux x86_64");
        };
        Ok(Self {
            triple,
            os,
            exe_name: format!("{APP_BINARY}{}", std::env::consts::EXE_SUFFIX),
        })
    }
}

/// Esegue l'intero rilascio; fallisce se un controllo sull'eseguibile non passa.
pub fn run() -> Result<()> {
    let root = workspace_root()?;
    let platform = Platform::host()?;
    reject_rustflags_env()?;
    for required in ["LICENSE", "README.md"] {
        if !root.join(required).is_file() {
            bail!("manca {required} nella radice del repository");
        }
    }
    let version = app_version(&root)?;

    let locations = Locations::detect(&root)?;
    let config = remap::config_arg(platform.triple, &locations.remaps());
    eprintln!(
        "xtask: build di rilascio di {APP_PACKAGE} {version} per {}",
        platform.triple
    );
    run_inherited(
        cargo()
            .current_dir(&root)
            .args(["build", "--release", "--locked", "--package", APP_PACKAGE])
            .args(["--bin", APP_BINARY, "--target", platform.triple])
            .arg("--config")
            .arg(&config),
        "cargo build --release",
    )?;

    let exe = target_dir(&root)
        .join(platform.triple)
        .join("release")
        .join(&platform.exe_name);
    let exe_bytes =
        fs::read(&exe).with_context(|| format!("lettura dell'eseguibile {}", exe.display()))?;

    if platform.os == "windows" {
        verify_mitigations(&exe)?;
    }
    verify_no_local_data(&exe_bytes, &locations.needles())?;

    let dist = root.join(DIST_DIR);
    fs::create_dir_all(&dist).context("creazione della cartella dist")?;
    let stem = format!("{APP_BINARY}-{version}-{}-x86_64", platform.os);
    let zip_name = format!("{stem}.zip");
    let zip_bytes = build_zip(&stem, &platform.exe_name, &exe_bytes, &root)?;
    write_atomically(&dist.join(&zip_name), &zip_bytes)?;
    let sums = checksum_line(&zip_bytes, &zip_name);
    write_atomically(&dist.join(CHECKSUMS_FILE), sums.as_bytes())?;

    eprintln!("xtask: creati {DIST_DIR}/{zip_name} e {DIST_DIR}/{CHECKSUMS_FILE}");
    Ok(())
}

/// `RUSTFLAGS` e `CARGO_ENCODED_RUSTFLAGS` sostituirebbero i rustflags di
/// `.cargo/config.toml` (CFG, CETCOMPAT) e quelli di xtask: si rifiutano.
fn reject_rustflags_env() -> Result<()> {
    for name in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"] {
        if std::env::var_os(name).is_some() {
            bail!("la variabile d'ambiente {name} è impostata: toglila prima di dist");
        }
    }
    Ok(())
}

/// Versione del pacchetto dell'app, chiesta a cargo; errore chiaro se il pacchetto non esiste.
fn app_version(root: &Path) -> Result<String> {
    let pkgid = run_captured(
        cargo()
            .current_dir(root)
            .args(["pkgid", "--package", APP_PACKAGE]),
        "cargo pkgid",
    )
    .with_context(|| {
        format!("il pacchetto {APP_PACKAGE} (binario {APP_BINARY}) non esiste ancora nel workspace")
    })?;
    parse_pkgid_version(&pkgid)
        .with_context(|| format!("versione non trovata in «{}»", pkgid.trim()))
}

/// Estrae la versione da un id di pacchetto (`...#0.1.0` oppure `...#nome@0.1.0`).
fn parse_pkgid_version(pkgid: &str) -> Option<String> {
    let fragment = pkgid.trim().rsplit_once('#')?.1;
    let version = fragment.rsplit_once('@').map_or(fragment, |(_, v)| v);
    (!version.is_empty()).then(|| version.to_owned())
}

fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from)
}

/// Percorsi e nomi locali di questa macchina, letti al momento e mai salvati.
struct Locations {
    user_name: Option<String>,
    home: Option<PathBuf>,
    cargo_home: PathBuf,
    rustup_home: PathBuf,
    project: PathBuf,
}

impl Locations {
    fn detect(root: &Path) -> Result<Self> {
        let home = env_path(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
        let default_under_home = |dir: &str| home.as_ref().map(|h| h.join(dir));
        let cargo_home = env_path("CARGO_HOME")
            .or_else(|| default_under_home(".cargo"))
            .context("né CARGO_HOME né la cartella utente sono noti")?;
        let rustup_home = env_path("RUSTUP_HOME")
            .or_else(|| default_under_home(".rustup"))
            .context("né RUSTUP_HOME né la cartella utente sono noti")?;
        let user_name = ["USERNAME", "USER"]
            .iter()
            .find_map(|name| std::env::var(name).ok())
            .filter(|name| !name.is_empty());
        Ok(Self {
            user_name,
            home,
            cargo_home,
            rustup_home,
            project: root.to_path_buf(),
        })
    }

    fn remaps(&self) -> Vec<remap::Remap> {
        remap::remaps(
            self.home.clone(),
            self.cargo_home.clone(),
            self.rustup_home.clone(),
            self.project.clone(),
        )
    }

    fn needles(&self) -> Vec<Needle> {
        let mut list = Vec::new();
        let mut add = |label: &str, text: String| {
            list.push(Needle {
                label: label.to_owned(),
                text,
            });
        };
        // Il nome utente da solo può essere una parola comune, presente per esempio nei
        // dizionari di compressione o nelle tabelle Unicode incluse dalle dipendenze:
        // conta solo dentro un percorso, cioè tra separatori.
        if let Some(name) = &self.user_name {
            add("nome utente in un percorso", format!(r"\{name}\"));
            add("nome utente in un percorso", format!("/{name}/"));
        }
        if let Some(home) = &self.home {
            add("cartella utente", home.display().to_string());
        }
        add("cartella del progetto", self.project.display().to_string());
        add("CARGO_HOME", self.cargo_home.display().to_string());
        add("RUSTUP_HOME", self.rustup_home.display().to_string());
        list
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Verifica CFG e CETCOMPAT con `dumpbin /headers`; se `dumpbin` non si trova lo
/// dice e prosegue, se lo trova e un flag manca fallisce.
fn verify_mitigations(exe: &Path) -> Result<()> {
    let Some(dumpbin) = find_dumpbin() else {
        eprintln!(
            "xtask: attenzione: dumpbin non trovato tramite vswhere, CFG e CETCOMPAT non verificati"
        );
        return Ok(());
    };
    let headers = run_captured(
        Command::new(&dumpbin)
            .args(["/headers", "/nologo"])
            .arg(exe),
        "dumpbin /headers",
    )?;
    let mut missing = Vec::new();
    if !headers.contains("Control Flow Guard") {
        missing.push("Control Flow Guard");
    }
    if !headers.contains("CET compatible") {
        missing.push("CETCOMPAT");
    }
    if !missing.is_empty() {
        bail!(
            "all'eseguibile mancano le mitigazioni: {}",
            missing.join(", ")
        );
    }
    eprintln!("xtask: CFG e CETCOMPAT presenti");
    Ok(())
}

/// Percorso assoluto di `dumpbin.exe` dagli strumenti MSVC più recenti, tramite vswhere.
fn find_dumpbin() -> Option<PathBuf> {
    let output = Command::new(VSWHERE)
        .args(["-latest", "-products", "*", "-requires"])
        .arg("Microsoft.VisualStudio.Component.VC.Tools.x86.x64")
        .args(["-find", r"VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .rfind(|path| path.is_absolute() && path.is_file())
}

fn verify_no_local_data(exe_bytes: &[u8], needles: &[Needle]) -> Result<()> {
    let findings = scan::find_all(exe_bytes, needles);
    if findings.is_empty() {
        eprintln!("xtask: nessun nome utente o percorso locale nell'eseguibile");
        return Ok(());
    }
    let report: Vec<String> = findings
        .iter()
        .map(|f| format!("{} ({}, offset {:#x})", f.label, f.encoding, f.offset))
        .collect();
    bail!("l'eseguibile contiene dati locali: {}", report.join("; "));
}

/// Zip con una cartella `<stem>/` che contiene eseguibile, `LICENSE` e `README.md`.
fn build_zip(stem: &str, exe_name: &str, exe_bytes: &[u8], root: &Path) -> Result<Vec<u8>> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let entries: [(&str, Vec<u8>, u32); 3] = [
        (exe_name, exe_bytes.to_vec(), 0o755),
        ("LICENSE", read(root, "LICENSE")?, 0o644),
        ("README.md", read(root, "README.md")?, 0o644),
    ];
    for (name, bytes, mode) in entries {
        writer
            .start_file(format!("{stem}/{name}"), options.unix_permissions(mode))
            .with_context(|| format!("zip: aggiunta di {name}"))?;
        writer
            .write_all(&bytes)
            .with_context(|| format!("zip: scrittura di {name}"))?;
    }
    Ok(writer.finish().context("zip: chiusura")?.into_inner())
}

fn read(root: &Path, name: &str) -> Result<Vec<u8>> {
    fs::read(root.join(name)).with_context(|| format!("lettura di {name}"))
}

/// Una riga di `SHA256SUMS` nel formato di `sha256sum`: `<hex>  <nome>\n`.
fn checksum_line(bytes: &[u8], file_name: &str) -> String {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("{hex}  {file_name}\n")
}

/// Scrittura atomica: file temporaneo nella stessa cartella, `sync_all`, rinomina.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut temp_name = path.as_os_str().to_owned();
    temp_name.push(".tmp");
    let temp = PathBuf::from(temp_name);
    let mut file =
        fs::File::create(&temp).with_context(|| format!("creazione di {}", temp.display()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .with_context(|| format!("scrittura di {}", temp.display()))?;
    drop(file);
    fs::rename(&temp, path).with_context(|| format!("rinomina in {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkgid_with_name_and_version() {
        assert_eq!(
            parse_pkgid_version("path+file:///x/crates/app#lynkeus-app@0.1.0\n").as_deref(),
            Some("0.1.0")
        );
    }

    #[test]
    fn pkgid_with_version_only() {
        assert_eq!(
            parse_pkgid_version("path+file:///x/crates/lynkeus-app#0.2.3").as_deref(),
            Some("0.2.3")
        );
    }

    #[test]
    fn pkgid_without_fragment_is_rejected() {
        assert_eq!(
            parse_pkgid_version("path+file:///x/crates/lynkeus-app"),
            None
        );
    }

    #[test]
    fn checksum_line_matches_sha256sum_format() {
        assert_eq!(
            checksum_line(b"abc", "a.zip"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  a.zip\n"
        );
    }
}
