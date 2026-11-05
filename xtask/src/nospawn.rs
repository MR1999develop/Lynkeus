// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Controllo "nessun avvio di processi nei decoder" (threat model §4.7, parte B, riga E).
//!
//! I sorgenti di `crates/lynkeus-extract/src` non devono nominare le API che avviano
//! processi: un decoder compromesso o ingannato non deve trovare nel proprio codice un modo
//! pronto per eseguire programmi. L'unico avvio è in `lynkeus-engine` (`launcher.rs`) e
//! nell'app. Le righe di commento (`//`, `///`, `//!`) non contano. Un `use` con un gruppo
//! su una riga (`use nix::unistd::{fork, execv};`) si controlla anche percorso per percorso,
//! come se fosse scritto per esteso. Le eccezioni sono in [`ALLOWED_FILES`], per percorso
//! relativo alla cartella controllata: oggi nessuna.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Cartella controllata, relativa alla radice del workspace.
pub const CHECKED_DIR: &str = "crates/lynkeus-extract/src";

/// File esclusi dal controllo, con `/` come separatore. Vuoto: aggiungere un file qui è una
/// decisione di sicurezza da motivare nel threat model.
const ALLOWED_FILES: &[&str] = &[];

/// Come si cerca un testo vietato.
#[derive(Debug, Clone, Copy)]
enum Match {
    /// Ovunque nella riga (`std::process`, `libc::fork`, ...).
    Anywhere,
    /// Solo come identificatore intero: `Command` sì, `CommandLauncher` no.
    Word,
}

/// I testi vietati: moduli e funzioni che avviano processi o eseguono programmi.
const FORBIDDEN: &[(&str, Match)] = &[
    ("std::process", Match::Anywhere),
    ("process::", Match::Anywhere),
    ("Command", Match::Word),
    ("libc::exec", Match::Anywhere),
    ("libc::fork", Match::Anywhere),
    ("libc::vfork", Match::Anywhere),
    ("libc::system", Match::Anywhere),
    ("unistd::fork", Match::Anywhere),
    ("unistd::exec", Match::Anywhere),
    ("std::os::unix::process", Match::Anywhere),
    ("posix_spawn", Match::Anywhere),
    ("CreateProcess", Match::Anywhere),
    ("ShellExecute", Match::Anywhere),
    ("WinExec", Match::Anywhere),
];

/// Un uso vietato trovato in una riga.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Numero di riga, da 1.
    pub line: usize,
    /// Il testo vietato trovato.
    pub pattern: &'static str,
}

/// Gli usi vietati in `source`, nell'ordine delle righe; le righe di commento si saltano.
/// Ogni testo vietato si segnala al più una volta per riga.
pub fn find_spawn_uses(source: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (index, line) in source.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let mut forms = vec![line.to_owned()];
        forms.extend(expand_use_group(line));
        for &(pattern, how) in FORBIDDEN {
            let found = forms.iter().any(|form| match how {
                Match::Anywhere => form.contains(pattern),
                Match::Word => contains_word(form, pattern),
            });
            if found {
                findings.push(Finding {
                    line: index + 1,
                    pattern,
                });
            }
        }
    }
    findings
}

/// I percorsi per esteso di un `use` con un gruppo su una riga:
/// `use std::{process, fs::File};` dà `std::process` e `std::fs::File`. Gruppi annidati e
/// `use` su più righe non si espandono (restano controllati come testo).
fn expand_use_group(line: &str) -> Vec<String> {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed
        .strip_prefix("pub use ")
        .or_else(|| trimmed.strip_prefix("use "))
    else {
        return Vec::new();
    };
    let Some((prefix, group)) = rest.split_once('{') else {
        return Vec::new();
    };
    let Some((items, _)) = group.split_once('}') else {
        return Vec::new();
    };
    let prefix = prefix.trim();
    items
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| format!("{prefix}{item}"))
        .collect()
}

/// Vero se `word` compare in `line` senza lettere, cifre o `_` subito prima o dopo.
fn contains_word(line: &str, word: &str) -> bool {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    line.match_indices(word).any(|(start, _)| {
        let before = line.get(..start).and_then(|s| s.chars().next_back());
        let after = line
            .get(start + word.len()..)
            .and_then(|s| s.chars().next());
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

/// Controlla tutti i `.rs` di [`CHECKED_DIR`] sotto `root`; errore con l'elenco degli usi
/// vietati, o se la cartella non si legge (un controllo che non trova i file non passa).
pub fn run(root: &Path) -> Result<()> {
    let dir = root.join(CHECKED_DIR);
    let mut files = Vec::new();
    collect_rust_files(&dir, &mut files)?;
    if files.is_empty() {
        bail!("nessun sorgente in {CHECKED_DIR}");
    }
    files.sort();
    let mut report = Vec::new();
    for file in &files {
        let relative = relative_name(&dir, file);
        if ALLOWED_FILES.contains(&relative.as_str()) {
            continue;
        }
        let source = std::fs::read_to_string(file)
            .with_context(|| format!("lettura di {CHECKED_DIR}/{relative}"))?;
        for finding in find_spawn_uses(&source) {
            report.push(format!(
                "{CHECKED_DIR}/{relative}:{}: «{}»",
                finding.line, finding.pattern
            ));
        }
    }
    if !report.is_empty() {
        bail!(
            "avvio di processi nei decoder (vietato, threat model §4.7):\n  {}",
            report.join("\n  ")
        );
    }
    eprintln!(
        "xtask: nessun avvio di processi in {CHECKED_DIR} ({} file)",
        files.len()
    );
    Ok(())
}

fn collect_rust_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let listing = std::fs::read_dir(dir).with_context(|| format!("lettura di {dir:?}"))?;
    for item in listing {
        let item = item.with_context(|| format!("lettura di {dir:?}"))?;
        let path = item.path();
        let kind = item
            .file_type()
            .with_context(|| format!("tipo di {path:?}"))?;
        if kind.is_dir() {
            collect_rust_files(&path, files)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

/// Il percorso di `file` relativo a `dir`, con `/` come separatore.
fn relative_name(dir: &Path, file: &Path) -> String {
    let relative = file.strip_prefix(dir).unwrap_or(file);
    let parts: Vec<String> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterns(source: &str) -> Vec<&'static str> {
        find_spawn_uses(source)
            .into_iter()
            .map(|f| f.pattern)
            .collect()
    }

    #[test]
    fn forbidden_uses_are_found() {
        assert_eq!(
            patterns("use std::process::Command;"),
            ["std::process", "process::", "Command"]
        );
        assert_eq!(patterns("let c = Command::new(exe);"), ["Command"]);
        assert_eq!(
            patterns("std::process::exit(1);"),
            ["std::process", "process::"]
        );
        assert_eq!(patterns("unsafe { libc::execv(p, a) }"), ["libc::exec"]);
        assert_eq!(patterns("let pid = libc::fork();"), ["libc::fork"]);
        assert_eq!(patterns("CreateProcessW(...)"), ["CreateProcess"]);
        assert_eq!(
            patterns("let child = nix::unistd::fork()?;"),
            ["unistd::fork"]
        );
        assert_eq!(patterns("nix::unistd::execv(&p, &a)?;"), ["unistd::exec"]);
        assert_eq!(
            patterns("use std::os::unix::process::CommandExt;"),
            ["process::", "std::os::unix::process"]
        );
    }

    /// I `use` con un gruppo su una riga si controllano come scritti per esteso.
    #[test]
    fn grouped_uses_are_expanded() {
        assert_eq!(
            patterns("use nix::unistd::{fork, execv};"),
            ["unistd::fork", "unistd::exec"]
        );
        assert_eq!(patterns("use std::{fs, process};"), ["std::process"]);
        assert_eq!(
            patterns("    pub use std::os::unix::{fs::PermissionsExt, process};"),
            ["std::os::unix::process"]
        );
        assert!(patterns("use std::{fs, io::Read};").is_empty());
        assert!(patterns("use nix::unistd::{getpid, Pid};").is_empty());
    }

    #[test]
    fn similar_names_and_comments_are_not_findings() {
        assert!(patterns("struct CommandLauncher;").is_empty());
        assert!(patterns("let my_Command_x = 1;").is_empty());
        assert!(patterns("fn subcommand() {}").is_empty());
        assert!(patterns("    /// Nessun `std::process::Command` qui.").is_empty());
        assert!(patterns("// Command").is_empty());
        assert!(patterns("let preprocess = 1;").is_empty());
    }

    #[test]
    fn line_numbers_start_at_one() {
        let found = find_spawn_uses("fn a() {}\n\nlet c = Command::new(x);\n");
        assert_eq!(
            found,
            [Finding {
                line: 3,
                pattern: "Command"
            }]
        );
    }
}
