// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! `cargo xtask dist`: build di rilascio verificata e pacchetto per la distribuzione.
//!
//! Passi: build `--locked` di `lynkeus-app` con i percorsi locali riscritti
//! (vedi [`crate::remap`]), con `cargo auditable` per il rilascio ([`Mode`]); su Windows
//! verifica delle mitigazioni lette dal file ([`crate::pe`]: ASLR, DEP, CFG, CETCOMPAT,
//! `DependentLoadFlags` con la sola cartella di sistema, nessun runtime C dinamico) e prova
//! sull'eseguibile con DLL finte accanto, che il sistema deve ignorare; ricerca di nome
//! utente e percorsi locali nei byte dell'eseguibile; zip con eseguibile, `LICENSE` e
//! `README.md`; per il rilascio lo SBOM CycloneDX, controllato allo stesso modo; file
//! `SHA256SUMS`.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use sha2::{Digest as _, Sha256};
use zip::write::SimpleFileOptions;

use crate::pe::Mitigations;
use crate::process::{cargo, run_captured, run_inherited, workspace_root};
use crate::remap;
use crate::scan::{self, Needle};

const APP_PACKAGE: &str = "lynkeus-app";
/// La cartella del pacchetto dell'app, dove `cargo cyclonedx` scrive il suo SBOM.
const APP_DIR: &str = "crates/lynkeus-app";
const APP_BINARY: &str = "lynkeus-searcher";
const DIST_DIR: &str = "dist";
const CHECKSUMS_FILE: &str = "SHA256SUMS";
/// Cartella (sotto la cartella di build) della prova con le DLL finte.
const PLANTING_DIR: &str = "xtask-dll-planting";
/// Contenuto delle DLL finte: non è un'immagine PE, quindi se il sistema ne caricasse una
/// il processo morirebbe con `STATUS_INVALID_IMAGE_NOT_MZ` (0xC000012F) prima di `main`.
const FAKE_DLL_BYTES: &[u8] = b"DLL finta di cargo xtask dist: non deve essere caricata";
/// Tempo massimo del processo estrattore nella prova con le DLL finte.
const PLANTING_TIMEOUT: Duration = Duration::from_secs(30);
/// Limite di memoria passato al processo estrattore nella prova (1 GiB, come nell'app).
const PLANTING_MEMORY_LIMIT: &str = "1073741824";

/// Che pacchetto si prepara.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `cargo xtask dist`: la verifica in locale, con `cargo build`.
    Local,
    /// `cargo xtask release` (CI su un tag): `cargo auditable build`, cioè l'elenco delle
    /// dipendenze dentro l'eseguibile (leggibile da `cargo audit bin`), e lo SBOM
    /// CycloneDX accanto allo zip. Servono `cargo-auditable` e `cargo-cyclonedx`.
    Release,
}

impl Mode {
    /// Gli argomenti di cargo che avviano la build.
    fn cargo_args(self) -> &'static [&'static str] {
        match self {
            Mode::Local => &["build"],
            Mode::Release => &["auditable", "build"],
        }
    }
}

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
pub fn run(mode: Mode) -> Result<()> {
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
        "xtask: build di rilascio di {APP_PACKAGE} {version} per {} (cargo {})",
        platform.triple,
        mode.cargo_args().join(" ")
    );
    if mode == Mode::Local {
        eprintln!(
            "xtask: verifica locale: niente elenco delle dipendenze né SBOM (cargo xtask release)"
        );
    }
    run_inherited(
        cargo()
            .current_dir(&root)
            .args(mode.cargo_args())
            .args(["--release", "--locked", "--package", APP_PACKAGE])
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
        let mitigations = verify_mitigations(&exe_bytes)?;
        let planting = target_dir(&root).join(PLANTING_DIR);
        verify_dll_planting(&exe, &mitigations.imports, &planting)?;
    }
    let needles = locations.needles();
    verify_no_local_data("l'eseguibile", &exe_bytes, &needles)?;

    let dist = root.join(DIST_DIR);
    fs::create_dir_all(&dist).context("creazione della cartella dist")?;
    let stem = format!("{APP_BINARY}-{version}-{}-x86_64", platform.os);
    let zip_name = format!("{stem}.zip");
    let zip_bytes = build_zip(&stem, &platform.exe_name, &exe_bytes, &root)?;
    write_atomically(&dist.join(&zip_name), &zip_bytes)?;
    let mut sums = checksum_line(&zip_bytes, &zip_name);
    if mode == Mode::Release {
        let sbom_name = format!("{stem}.cdx.json");
        let sbom = generate_sbom(&root, platform.triple)?;
        verify_no_local_data("lo SBOM", &sbom, &needles)?;
        write_atomically(&dist.join(&sbom_name), &sbom)?;
        sums.push_str(&checksum_line(&sbom, &sbom_name));
        eprintln!("xtask: creato {DIST_DIR}/{sbom_name}");
    }
    write_atomically(&dist.join(CHECKSUMS_FILE), sums.as_bytes())?;

    eprintln!("xtask: creati {DIST_DIR}/{zip_name} e {DIST_DIR}/{CHECKSUMS_FILE}");
    Ok(())
}

/// Lo SBOM CycloneDX (JSON, specifica 1.5) del pacchetto dell'app per `triple`, con il
/// percorso del workspace sostituito da un nome neutro.
///
/// `cargo cyclonedx` scrive un file per ogni pacchetto del workspace, accanto al suo
/// `Cargo.toml`, con il nome scelto qui: si legge quello dell'app e si cancellano tutti.
fn generate_sbom(root: &Path, triple: &str) -> Result<Vec<u8>> {
    let name = format!("xtask-sbom-{}", std::process::id());
    let file_name = format!("{name}.json");
    let generated = run_inherited(
        cargo().current_dir(root).args([
            "cyclonedx",
            "--format",
            "json",
            "--spec-version",
            "1.5",
            "--target",
            triple,
            "--override-filename",
            &name,
            "-q",
        ]),
        "cargo cyclonedx (installare cargo-cyclonedx)",
    )
    .and_then(|()| {
        let path = root.join(APP_DIR).join(&file_name);
        fs::read_to_string(&path).with_context(|| format!("lettura di {}", path.display()))
    });
    remove_generated(root, &file_name)?;
    Ok(neutral_paths(&generated?, root).into_bytes())
}

/// Il testo di uno SBOM con il percorso del workspace `root` (scritto da cargo come URL
/// `path+file:///…`, con `/`) sostituito da `lynkeus`, come nei binari.
fn neutral_paths(sbom: &str, root: &Path) -> String {
    let native = root.display().to_string();
    let forward = native.replace('\\', "/");
    let escaped = native.replace('\\', "\\\\");
    sbom.replace(&forward, "lynkeus")
        .replace(&escaped, "lynkeus")
        .replace(&native, "lynkeus")
}

/// Cancella i file `file_name` scritti nelle cartelle dei pacchetti del workspace (fino a
/// due livelli sotto `root`, solo cartelle con un `Cargo.toml`).
fn remove_generated(root: &Path, file_name: &str) -> Result<()> {
    let first_level = subdirs(root);
    let second_level: Vec<PathBuf> = first_level.iter().flat_map(|dir| subdirs(dir)).collect();
    let dirs = std::iter::once(root.to_path_buf())
        .chain(first_level)
        .chain(second_level);
    for dir in dirs {
        let file = dir.join(file_name);
        if dir.join("Cargo.toml").is_file() && file.is_file() {
            fs::remove_file(&file).with_context(|| format!("pulizia di {}", file.display()))?;
        }
    }
    Ok(())
}

/// Le sottocartelle dirette di `dir` (nessuna se non si legge).
fn subdirs(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default()
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

/// Verifica le mitigazioni lette dal file ([`Mitigations::problems`]); fallisce se ne
/// manca una o se il file non si legge.
fn verify_mitigations(exe_bytes: &[u8]) -> Result<Mitigations> {
    let mitigations = Mitigations::read(exe_bytes)?;
    let problems = mitigations.problems();
    if !problems.is_empty() {
        bail!("all'eseguibile mancano: {}", problems.join("; "));
    }
    eprintln!(
        "xtask: ASLR, DEP, CFG, CETCOMPAT e DependentLoadFlags {:#x} presenti; {} DLL \
         importate, nessun runtime C dinamico",
        mitigations.dependent_load_flags,
        mitigations.imports.len()
    );
    Ok(mitigations)
}

/// Stampa le mitigazioni e le DLL importate di `exe` (`cargo xtask mitigations`) e, su
/// Windows, fa la prova con le DLL finte accanto; errore se una mitigazione manca o la
/// prova fallisce.
pub fn print_mitigations(exe: &Path) -> Result<()> {
    let bytes = fs::read(exe).with_context(|| format!("lettura di {}", exe.display()))?;
    let mitigations = Mitigations::read(&bytes)?;
    println!(
        "DllCharacteristics: {:#06x}",
        mitigations.dll_characteristics
    );
    println!("GuardFlags:         {:#010x}", mitigations.guard_flags);
    println!(
        "DependentLoadFlags: {:#06x}",
        mitigations.dependent_load_flags
    );
    println!("CETCOMPAT:          {}", mitigations.cet_compatible);
    println!("DLL importate:      {}", mitigations.imports.join(", "));
    let mut problems = mitigations.problems();
    if cfg!(windows) {
        let planting = target_dir(&workspace_root()?).join(PLANTING_DIR);
        if let Err(error) = verify_dll_planting(exe, &mitigations.imports, &planting) {
            problems.push(format!("{error:#}"));
        }
    }
    if problems.is_empty() {
        println!("nessuna mitigazione mancante, DLL finte ignorate");
        Ok(())
    } else {
        bail!("mancano: {}", problems.join("; "))
    }
}

/// La prova del caricamento delle DLL sull'eseguibile vero: una copia in `work` con accanto
/// una DLL finta per ogni DLL importata, avviata come processo estrattore con lo stdin
/// chiuso. Deve uscire con successo entro [`PLANTING_TIMEOUT`]: se il sistema caricasse una
/// DLL finta, il processo morirebbe prima di `main`.
fn verify_dll_planting(exe: &Path, imports: &[String], work: &Path) -> Result<()> {
    if work.exists() {
        fs::remove_dir_all(work).with_context(|| format!("pulizia di {}", work.display()))?;
    }
    fs::create_dir_all(work).with_context(|| format!("creazione di {}", work.display()))?;
    let copy = work.join(exe.file_name().context("eseguibile senza nome")?);
    fs::copy(exe, &copy).with_context(|| format!("copia in {}", copy.display()))?;
    for dll in imports {
        // Solo nomi semplici: la DLL finta resta nella cartella di prova.
        if dll.contains(['/', '\\', ':']) || dll.starts_with('.') {
            bail!("nome di DLL importata non valido: {dll}");
        }
        fs::write(work.join(dll), FAKE_DLL_BYTES).with_context(|| format!("DLL finta {dll}"))?;
    }
    let mut child = Command::new(&copy)
        .args([
            "--extract-worker",
            "--memory-limit-bytes",
            PLANTING_MEMORY_LIMIT,
        ])
        .current_dir(work)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("avvio di {}", copy.display()))?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().context("attesa del processo di prova")? {
            break status;
        }
        if started.elapsed() > PLANTING_TIMEOUT {
            let _killed = child.kill();
            let _reaped = child.wait();
            bail!("il processo estrattore con le DLL finte accanto non è uscito in tempo");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if !status.success() {
        let code = status.code().map_or_else(
            || "nessun codice".to_owned(),
            |code| format!("{:#010x}", code.cast_unsigned()),
        );
        bail!(
            "con DLL finte accanto all'eseguibile il processo estrattore è uscito con {code}: \
             il sistema cerca le DLL importate nella cartella dell'eseguibile"
        );
    }
    let _cleaned = fs::remove_dir_all(work);
    eprintln!(
        "xtask: {} DLL finte accanto all'eseguibile ignorate dal sistema",
        imports.len()
    );
    Ok(())
}

/// Cerca nome utente e percorsi locali nei byte di `what` (l'eseguibile, lo SBOM).
fn verify_no_local_data(what: &str, bytes: &[u8], needles: &[Needle]) -> Result<()> {
    let findings = scan::find_all(bytes, needles);
    if findings.is_empty() {
        eprintln!("xtask: nessun nome utente o percorso locale ({what})");
        return Ok(());
    }
    let report: Vec<String> = findings
        .iter()
        .map(|f| format!("{} ({}, offset {:#x})", f.label, f.encoding, f.offset))
        .collect();
    bail!("{what} contiene dati locali: {}", report.join("; "));
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

    /// Il percorso del workspace nello SBOM (URL con `/`, JSON con `\\` o testo nativo)
    /// diventa `lynkeus`.
    #[test]
    fn sbom_paths_become_neutral() {
        let (root, sbom) = if cfg!(windows) {
            (
                Path::new(r"D:\progetti\lynkeus"),
                r#"{"bom-ref":"path+file:///D:/progetti/lynkeus/crates/app#0.1.0","p":"D:\\progetti\\lynkeus\\x"}"#,
            )
        } else {
            (
                Path::new("/home/u/progetti/lynkeus"),
                r#"{"bom-ref":"path+file:///home/u/progetti/lynkeus/crates/app#0.1.0"}"#,
            )
        };
        let neutral = neutral_paths(sbom, root);
        assert!(!neutral.contains("progetti"), "{neutral}");
        assert!(neutral.contains("lynkeus/crates/app#0.1.0"), "{neutral}");
    }

    #[test]
    fn checksum_line_matches_sha256sum_format() {
        assert_eq!(
            checksum_line(b"abc", "a.zip"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  a.zip\n"
        );
    }
}
