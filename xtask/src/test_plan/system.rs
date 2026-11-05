// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Livello 4 del piano: la parte automatica dei test manuali, verificata da programma con
//! gli strumenti del sistema come oracolo (`Get-Acl`, `reg.exe`, `Get-FileHash`,
//! `sha256sum`), e l'elenco dei casi che restano da eseguire a mano.
//!
//! - M-18: descrittore delle cartelle private dell'utente, se esistono;
//! - M-22, M-24, M-26: test `#[ignore]` di `crates/lynkeus-app/tests/sistema.rs`;
//! - M-30/M-31, M-33, M-34: il pacchetto di `cargo xtask dist` (saltato con `--quick`):
//!   hash contro `SHA256SUMS`, mitigazioni dell'eseguibile estratto dallo zip, nessun dato
//!   locale nei file dello zip.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use super::capture::{self, Echo};
use super::{Entry, Level, Verdict};
use crate::dist;
use crate::plan_map::{Automation, MANUAL_CASES, ManualCase, SharedState, TestRef};
use crate::process::cargo;
use crate::scan;

/// Tempo massimo di un test di sistema.
const SYSTEM_TEST_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Tempo massimo di uno strumento del sistema.
const TOOL_TIMEOUT: Duration = Duration::from_secs(120);
/// Byte massimi letti da una voce dello zip del pacchetto (l'eseguibile è la più grande).
const MAX_PACKAGE_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
/// Rimando per i casi manuali.
const MANUAL_REFERENCE: &str = "da eseguire a mano: docs/piano-di-test.md, livello 4";
/// Nome dell'eseguibile nel pacchetto.
const APP_EXE: &str = "lynkeus-searcher";

/// Il pacchetto di `cargo xtask dist`: la cartella e le righe di `SHA256SUMS`.
struct Package {
    dir: PathBuf,
    /// (hash esadecimale, nome del file).
    sums: Vec<(String, String)>,
}

/// Esegue la parte automatica del livello 4 e aggiunge una voce per ogni caso M-xx; un caso
/// che modifica stato condiviso della macchina gira solo se `allowed` lo comprende.
pub fn run(
    root: &Path,
    report_dir: &Path,
    quick: bool,
    allowed: &[SharedState],
    entries: &mut Vec<Entry>,
) {
    let needs_package = MANUAL_CASES.iter().any(|case| {
        case.os.includes_host()
            && matches!(
                case.automation,
                Automation::Checksums | Automation::Mitigations | Automation::NoLocalData
            )
    });
    let package = if needs_package {
        let (entry, package) = build_package(root, quick);
        entries.push(entry);
        package
    } else {
        None
    };
    let checks = Checks {
        root,
        report_dir,
        quick,
        allowed,
        package: package.as_ref(),
    };
    for case in MANUAL_CASES {
        entries.push(case_entry(case, &checks));
    }
}

/// `cargo xtask dist` nel processo di xtask; il pacchetto, se costruito.
fn build_package(root: &Path, quick: bool) -> (Entry, Option<Package>) {
    let mut entry = Entry::new(
        Level::Four,
        "L4-dist",
        "pacchetto verificato (mitigazioni, DLL finte, dati locali nell'exe) e SHA256SUMS",
        "cargo xtask dist",
        Verdict::Passed,
    );
    if quick {
        entry.verdict = Verdict::Skipped("saltato con --quick".to_owned());
        return (entry, None);
    }
    eprintln!("xtask: ── L4-dist ── cargo xtask dist");
    let started = Instant::now();
    let built = dist::run(dist::Mode::Local).and_then(|()| read_package(root));
    entry.duration = Some(started.elapsed());
    match built {
        Ok(package) => (entry, Some(package)),
        Err(error) => {
            entry.verdict = Verdict::Failed(format!("{error:#}"));
            (entry, None)
        }
    }
}

/// La cartella `dist/` e le righe di `SHA256SUMS`.
fn read_package(root: &Path) -> Result<Package> {
    let dir = root.join("dist");
    let text = std::fs::read_to_string(dir.join("SHA256SUMS")).context("lettura di SHA256SUMS")?;
    let sums = parse_sums(&text)?;
    if sums.is_empty() {
        bail!("SHA256SUMS vuoto");
    }
    Ok(Package { dir, sums })
}

/// Le righe `hash  nome` di un file nel formato di `sha256sum`.
fn parse_sums(text: &str) -> Result<Vec<(String, String)>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (hash, name) = line
                .split_once(' ')
                .context("riga di SHA256SUMS senza nome")?;
            let name = name.trim_start().trim_start_matches('*');
            let valid_hash = hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
            let valid_name = !name.is_empty() && !name.contains(['/', '\\']) && name != "..";
            if !valid_hash || !valid_name {
                bail!("riga di SHA256SUMS non valida");
            }
            Ok((hash.to_ascii_lowercase(), name.to_owned()))
        })
        .collect()
}

/// Ciò che serve per verificare i casi M-xx.
struct Checks<'a> {
    root: &'a Path,
    report_dir: &'a Path,
    quick: bool,
    /// Lo stato condiviso della macchina che si può modificare (opzioni del comando).
    allowed: &'a [SharedState],
    package: Option<&'a Package>,
}

/// La voce di un caso M-xx.
fn case_entry(case: &ManualCase, checks: &Checks<'_>) -> Entry {
    let Checks {
        root,
        report_dir,
        quick,
        allowed,
        package,
    } = *checks;
    let command = match case.automation {
        Automation::Manual => String::new(),
        Automation::IgnoredTest(test) => test.command(true),
        Automation::PrivateDirAcl => "Get-Acl sulle cartelle data e cache".to_owned(),
        Automation::Checksums if cfg!(windows) => "Get-FileHash contro SHA256SUMS".to_owned(),
        Automation::Checksums => "sha256sum -c SHA256SUMS".to_owned(),
        Automation::Mitigations => "cargo xtask mitigations <exe dello zip>".to_owned(),
        Automation::NoLocalData => "ricerca nei file dello zip".to_owned(),
    };
    let mut entry = Entry::new(Level::Four, case.id, case.title, &command, Verdict::Passed);
    if !case.rest.is_empty() {
        entry.note = format!("resta a mano: {}", case.rest);
    }
    if matches!(case.automation, Automation::Manual) {
        entry.verdict = Verdict::Manual(MANUAL_REFERENCE.to_owned());
        return entry;
    }
    if !case.os.includes_host() {
        entry.verdict =
            Verdict::NotApplicable(format!("parte automatica solo su {}", case.os.label()));
        return entry;
    }
    if let Some(state) = case.touches.filter(|state| !allowed.contains(state)) {
        entry.verdict = Verdict::Skipped(state.requirement());
        return entry;
    }
    let started = Instant::now();
    entry.verdict = match case.automation {
        Automation::Manual => Verdict::Manual(MANUAL_REFERENCE.to_owned()),
        Automation::IgnoredTest(test) => {
            let (verdict, excerpt) = ignored_test(root, &test);
            entry.excerpt = excerpt;
            verdict
        }
        Automation::PrivateDirAcl => private_dirs(),
        Automation::Checksums | Automation::Mitigations | Automation::NoLocalData => {
            match package {
                None if quick => Verdict::Skipped("pacchetto non costruito con --quick".to_owned()),
                None => Verdict::Failed("pacchetto non costruito (vedi L4-dist)".to_owned()),
                Some(package) => package_check(case.automation, package, root, report_dir),
            }
        }
    };
    if !matches!(entry.verdict, Verdict::Skipped(_)) {
        entry.duration = Some(started.elapsed());
    }
    entry
}

/// Un test `#[ignore]` di sistema, in release.
fn ignored_test(root: &Path, test: &TestRef) -> (Verdict, Option<String>) {
    eprintln!("xtask: ── {} ── {}", test.name, test.command(true));
    let mut command = cargo();
    command
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .args(["test", "--release", "--locked", "--workspace"])
        .args(test.cargo_args())
        .arg("--")
        .args(TestRef::harness_args(true));
    match capture::run(command, Echo::All, SYSTEM_TEST_TIMEOUT) {
        Err(error) => (Verdict::Failed(format!("{error:#}")), None),
        Ok(captured) if captured.success() && captured.output.contains(" ... ok") => {
            (Verdict::Passed, None)
        }
        Ok(captured) if captured.success() => (
            Verdict::Failed("il test non è stato eseguito (nome cambiato?)".to_owned()),
            Some(captured.tail(20)),
        ),
        Ok(captured) => (
            Verdict::Failed(format!("test fallito: {}", captured.status_text())),
            Some(captured.tail(40)),
        ),
    }
}

/// M-18: le cartelle private dell'app per l'utente corrente.
fn private_dirs() -> Verdict {
    #[cfg(windows)]
    {
        windows::private_dirs()
    }
    #[cfg(not(windows))]
    {
        Verdict::NotApplicable("solo Windows".to_owned())
    }
}

/// Una verifica sul pacchetto.
fn package_check(
    automation: Automation,
    package: &Package,
    root: &Path,
    report_dir: &Path,
) -> Verdict {
    let result = match automation {
        Automation::Checksums => verify_checksums(package),
        Automation::Mitigations => {
            extract_exe(package, report_dir).and_then(|exe| dist::print_mitigations(&exe))
        }
        Automation::NoLocalData => verify_no_local_data(package, root),
        Automation::Manual | Automation::IgnoredTest(_) | Automation::PrivateDirAcl => Ok(()),
    };
    match result {
        Ok(()) => Verdict::Passed,
        Err(error) => Verdict::Failed(format!("{error:#}")),
    }
}

/// M-30 (Windows, `Get-FileHash`) e M-31 (Linux, `sha256sum -c`).
fn verify_checksums(package: &Package) -> Result<()> {
    #[cfg(windows)]
    {
        for (expected, name) in &package.sums {
            let actual = windows::file_hash(&package.dir.join(name))?;
            if !actual.eq_ignore_ascii_case(expected) {
                bail!("{name}: hash diverso da SHA256SUMS");
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let tool = ["/usr/bin/sha256sum", "/bin/sha256sum"]
            .iter()
            .map(Path::new)
            .find(|p| p.is_file())
            .context("sha256sum non trovato")?;
        let mut command = std::process::Command::new(tool);
        command.current_dir(&package.dir).args(["-c", "SHA256SUMS"]);
        let captured = capture::run(command, Echo::All, TOOL_TIMEOUT)?;
        if !captured.success() {
            bail!("sha256sum -c: {}", captured.status_text());
        }
        Ok(())
    }
}

/// Lo zip del pacchetto.
fn package_zip(package: &Package) -> Result<zip::ZipArchive<std::fs::File>> {
    let (_, name) = package
        .sums
        .iter()
        .find(|(_, name)| name.ends_with(".zip"))
        .context("nessuno zip in SHA256SUMS")?;
    let file = std::fs::File::open(package.dir.join(name))
        .with_context(|| format!("apertura di {name}"))?;
    zip::ZipArchive::new(file).with_context(|| format!("lettura di {name}"))
}

/// I byte della voce `index` dello zip, entro il limite.
fn entry_bytes(
    archive: &mut zip::ZipArchive<std::fs::File>,
    index: usize,
) -> Result<(String, Vec<u8>)> {
    let entry = archive.by_index(index).context("voce dello zip")?;
    let name = entry
        .name()
        .map_or_else(|_| format!("voce {index}"), std::borrow::Cow::into_owned);
    if entry.size() > MAX_PACKAGE_ENTRY_BYTES {
        bail!("{name}: voce troppo grande");
    }
    let mut bytes = Vec::new();
    entry
        .take(MAX_PACKAGE_ENTRY_BYTES)
        .read_to_end(&mut bytes)
        .with_context(|| format!("lettura di {name}"))?;
    Ok((name, bytes))
}

/// M-33: l'eseguibile estratto dallo zip in `work/pacchetto/`.
fn extract_exe(package: &Package, work: &Path) -> Result<PathBuf> {
    let exe_name = format!("{APP_EXE}{}", std::env::consts::EXE_SUFFIX);
    let mut archive = package_zip(package)?;
    for index in 0..archive.len() {
        let (name, bytes) = entry_bytes(&mut archive, index)?;
        if name.rsplit('/').next() == Some(exe_name.as_str()) {
            let dir = work.join("pacchetto");
            std::fs::create_dir_all(&dir).context("cartella dell'eseguibile estratto")?;
            let exe = dir.join(&exe_name);
            std::fs::write(&exe, bytes).context("scrittura dell'eseguibile estratto")?;
            return Ok(exe);
        }
    }
    bail!("{exe_name} non trovato nello zip")
}

/// M-34: nessun nome utente, nome del computer o percorso locale nei file dello zip.
fn verify_no_local_data(package: &Package, root: &Path) -> Result<()> {
    let needles = dist::local_needles(root)?;
    let mut archive = package_zip(package)?;
    let mut problems = Vec::new();
    for index in 0..archive.len() {
        let (name, bytes) = entry_bytes(&mut archive, index)?;
        for finding in scan::find_all(&bytes, &needles) {
            problems.push(format!("{name}: {} ({})", finding.label, finding.encoding));
        }
    }
    if !problems.is_empty() {
        bail!("dati locali nel pacchetto: {}", problems.join("; "));
    }
    Ok(())
}

/// Giudica il descrittore letto da `Get-Acl` (righe `PROTECTED=`, `USER=`, `OWNER=`,
/// `RULE=sid|diritti|ereditarietà|propagazione|tipo|ereditata`): DACL protetta, proprietario
/// l'utente, solo utente e SYSTEM con controllo completo ereditato da file e cartelle.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "usato solo su Windows e nei test")
)]
pub fn judge_private_acl(output: &str) -> Result<(), String> {
    const SYSTEM: &str = "S-1-5-18";
    let value = |key: &str| {
        output
            .lines()
            .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
            .map(str::trim)
    };
    let user = value("USER").ok_or("utente corrente non letto")?;
    let who = |sid: &str| match sid {
        s if s == user => "utente corrente".to_owned(),
        SYSTEM => "SYSTEM".to_owned(),
        other => format!("SID {other}"),
    };
    let mut problems = Vec::new();
    if value("PROTECTED") != Some("True") {
        problems.push("DACL non protetta (eredita dalla cartella superiore)".to_owned());
    }
    if value("OWNER") != Some(user) {
        problems.push("proprietario diverso dall'utente".to_owned());
    }
    let mut granted = Vec::new();
    for line in output.lines() {
        let Some(rule) = line.trim().strip_prefix("RULE=") else {
            continue;
        };
        let fields: Vec<&str> = rule.split('|').map(str::trim).collect();
        let [sid, rights, inheritance, _propagation, kind, inherited] = fields.as_slice() else {
            problems.push("regola illeggibile".to_owned());
            continue;
        };
        let target = who(sid);
        if *inherited != "False" {
            problems.push(format!("regola ereditata per {target}"));
        }
        if *kind != "Allow" {
            problems.push(format!("regola di negazione per {target}"));
        } else if *sid != user && *sid != SYSTEM {
            problems.push(format!("accesso concesso a {target}"));
        } else if *rights != "FullControl"
            || !inheritance.contains("ContainerInherit")
            || !inheritance.contains("ObjectInherit")
        {
            problems.push(format!("diritti di {target}: {rights}, {inheritance}"));
        } else {
            granted.push(*sid);
        }
    }
    for (sid, name) in [(user, "utente corrente"), (SYSTEM, "SYSTEM")] {
        if !granted.contains(&sid) {
            problems.push(format!("manca il controllo completo per {name}"));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

#[cfg(windows)]
mod windows {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use anyhow::{Context, Result, bail};

    use super::{TOOL_TIMEOUT, Verdict, judge_private_acl};
    use crate::test_plan::capture::{self, Echo};
    use crate::test_plan::environment::powershell;

    /// Variabile con il percorso passato agli script (mai interpolato nel testo).
    const PATH_VAR: &str = "LYNKEUS_PLAN_PATH";

    const ACL_SCRIPT: &str = r"
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$ErrorActionPreference = 'Stop'
$acl = Get-Acl -LiteralPath $env:LYNKEUS_PLAN_PATH
$sid = [Security.Principal.SecurityIdentifier]
'PROTECTED=' + $acl.AreAccessRulesProtected
'USER=' + [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
'OWNER=' + $acl.GetOwner($sid).Value
foreach ($r in $acl.GetAccessRules($true, $true, $sid)) {
  'RULE=' + $r.IdentityReference.Value + '|' + $r.FileSystemRights + '|' + $r.InheritanceFlags + '|' + $r.PropagationFlags + '|' + $r.AccessControlType + '|' + $r.IsInherited
}
";

    const HASH_SCRIPT: &str = r"
$ErrorActionPreference = 'Stop'
(Get-FileHash -Algorithm SHA256 -LiteralPath $env:LYNKEUS_PLAN_PATH).Hash
";

    /// Esegue uno script di PowerShell con `path` nella variabile [`PATH_VAR`].
    fn powershell_with_path(script: &str, path: &Path) -> Result<String> {
        let exe = powershell().context("PowerShell di sistema non trovato")?;
        let mut command = Command::new(exe);
        command
            .env(PATH_VAR, path)
            .args(["-NoProfile", "-NonInteractive", "-Command", script]);
        let captured = capture::run(command, Echo::Nothing, TOOL_TIMEOUT)?;
        if !captured.success() {
            bail!("PowerShell: {}", captured.status_text());
        }
        Ok(captured.output)
    }

    /// L'hash SHA-256 di `file` secondo `Get-FileHash`.
    pub fn file_hash(file: &Path) -> Result<String> {
        let output = powershell_with_path(HASH_SCRIPT, file)?;
        output
            .lines()
            .map(str::trim)
            .find(|line| line.len() == 64)
            .map(str::to_owned)
            .context("hash non letto da Get-FileHash")
    }

    /// M-18: `%LOCALAPPDATA%\Lynkeus Searcher\{data,cache}`, se esistono.
    pub fn private_dirs() -> Verdict {
        let Some(local) = std::env::var_os("LOCALAPPDATA") else {
            return Verdict::NotApplicable("LOCALAPPDATA non impostata".to_owned());
        };
        let base = PathBuf::from(local).join("Lynkeus Searcher");
        let present: Vec<(&str, PathBuf)> = ["data", "cache"]
            .into_iter()
            .map(|name| (name, base.join(name)))
            .filter(|(_, dir)| dir.is_dir())
            .collect();
        if present.is_empty() {
            return Verdict::NotApplicable(
                "cartelle private assenti: avviare l'app una volta e ripetere".to_owned(),
            );
        }
        let mut problems = Vec::new();
        for (name, dir) in &present {
            match powershell_with_path(ACL_SCRIPT, dir) {
                Ok(output) => {
                    if let Err(problem) = judge_private_acl(&output) {
                        problems.push(format!("{name}: {problem}"));
                    }
                }
                Err(error) => problems.push(format!("{name}: {error:#}")),
            }
        }
        if problems.is_empty() {
            Verdict::Passed
        } else {
            Verdict::Failed(problems.join("; "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "S-1-5-21-1-2-3-1001";

    fn acl(protected: &str, owner: &str, rules: &[&str]) -> String {
        let mut text = format!("PROTECTED={protected}\nUSER={USER}\nOWNER={owner}\n");
        for rule in rules {
            text.push_str(&format!("RULE={rule}\n"));
        }
        text
    }

    const FULL: &str = "FullControl|ContainerInherit, ObjectInherit|None|Allow|False";

    #[test]
    fn private_acl_partitions() {
        let user_rule = format!("{USER}|{FULL}");
        let system_rule = format!("S-1-5-18|{FULL}");
        let good = [user_rule.as_str(), system_rule.as_str()];
        assert_eq!(judge_private_acl(&acl("True", USER, &good)), Ok(()));

        let open = acl("False", USER, &good);
        assert!(judge_private_acl(&open).is_err_and(|e| e.contains("non protetta")));

        let owner = acl("True", "S-1-5-32-544", &good);
        assert!(judge_private_acl(&owner).is_err_and(|e| e.contains("proprietario")));

        let users = format!("S-1-5-32-545|{FULL}");
        let extra = acl("True", USER, &[&user_rule, &system_rule, &users]);
        assert!(judge_private_acl(&extra).is_err_and(|e| e.contains("SID S-1-5-32-545")));

        let inherited =
            format!("{USER}|FullControl|ContainerInherit, ObjectInherit|None|Allow|True");
        let with_inherited = acl("True", USER, &[&inherited, &system_rule]);
        assert!(judge_private_acl(&with_inherited).is_err_and(|e| e.contains("ereditata")));

        let read_only =
            format!("{USER}|ReadAndExecute|ContainerInherit, ObjectInherit|None|Allow|False");
        let weak = acl("True", USER, &[&read_only, &system_rule]);
        assert!(
            judge_private_acl(&weak)
                .is_err_and(|e| e.contains("manca il controllo completo per utente"))
        );

        let only_user = acl("True", USER, &[&user_rule]);
        assert!(judge_private_acl(&only_user).is_err_and(|e| e.contains("SYSTEM")));

        assert!(judge_private_acl("PROTECTED=True").is_err(), "senza utente");
    }

    #[test]
    fn sums_partitions() {
        let hash = "a".repeat(64);
        let parsed = parse_sums(&format!("{hash}  pacchetto.zip\n\n{hash} *altro.json\n"))
            .expect("righe valide");
        assert_eq!(
            parsed,
            [
                (hash.clone(), "pacchetto.zip".to_owned()),
                (hash.clone(), "altro.json".to_owned())
            ]
        );
        assert!(parse_sums("abc  x.zip").is_err(), "hash corto");
        assert!(
            parse_sums(&format!("{hash}  ../x.zip")).is_err(),
            "percorso"
        );
        assert!(parse_sums(&hash).is_err(), "senza nome");
    }
}
