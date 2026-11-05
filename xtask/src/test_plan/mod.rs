// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! `cargo xtask test-plan`: la parte automatizzabile del piano di test
//! (`docs/piano-di-test.md`) con un solo comando, e un rapporto Markdown.
//!
//! - Livelli 1 e 2 ([`suites`]): `cargo xtask check` (salvo `--quick`), le suite in release,
//!   gli esiti delle voci T-xx secondo la mappa (`crate::plan_map`) e il controllo della mappa;
//! - livello 3 ([`measures`]): le misure P-xx, un test `#[ignore]` per processo, con p50, p95,
//!   massimo, picco di memoria e confronto con le soglie (avvisi, non fallimenti);
//! - livello 4 ([`system`]): la parte automatica dei casi M-xx, gli altri elencati da fare a
//!   mano;
//! - livello 5: i compiti con le persone, elencati.
//!
//! Il rapporto va in `target/test-plan/rapporto-<data e ora UTC>.md`, il riepilogo su stderr;
//! il comando fallisce se fallisce una voce obbligatoria (tutte tranne le soglie delle misure).

mod capture;
mod environment;
mod libtest;
mod measures;
mod report;
mod suites;
mod system;

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use crate::plan_map::{SharedState, USABILITY_TASKS};
use crate::process::workspace_root;
use measures::MeasureLine;

/// Cartella dei rapporti, sotto la radice del workspace (ignorata da git con `target/`).
const REPORT_DIR: &str = "target/test-plan";

/// I livelli del piano.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    One,
    Two,
    Three,
    Four,
    Five,
}

impl Level {
    pub const ALL: [Level; 5] = [
        Level::One,
        Level::Two,
        Level::Three,
        Level::Four,
        Level::Five,
    ];

    pub fn number(self) -> u8 {
        match self {
            Level::One => 1,
            Level::Two => 2,
            Level::Three => 3,
            Level::Four => 4,
            Level::Five => 5,
        }
    }

    fn from_number(number: u8) -> Option<Level> {
        Level::ALL
            .into_iter()
            .find(|level| level.number() == number)
    }

    /// Il titolo della sezione del rapporto.
    pub fn title(self) -> &'static str {
        match self {
            Level::One => "Livello 1: automatici già presenti",
            Level::Two => "Livello 2: automatici scritti dal piano (T-xx)",
            Level::Three => "Livello 3: prestazioni",
            Level::Four => "Livello 4: test manuali (parte automatica e casi a mano)",
            Level::Five => "Livello 5: usabilità con tre persone",
        }
    }
}

/// Le opzioni del comando.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Senza `cargo xtask check`, `check-linux`, misure lunghe e pacchetto.
    pub quick: bool,
    /// I livelli da eseguire, in ordine e senza ripetizioni.
    pub levels: Vec<Level>,
    /// Lo stato condiviso della macchina che le voci possono modificare (`--clipboard`,
    /// `--registry`), in ordine e senza ripetizioni; per default nessuno.
    pub allowed: Vec<SharedState>,
}

impl Options {
    /// Legge gli argomenti dopo `test-plan`: `--quick`, `--level 1,2,3` (o `--level=1,2`),
    /// `--clipboard`, `--registry`; senza `--level`, tutti i livelli.
    pub fn parse(args: &[&str]) -> Result<Options, String> {
        let mut quick = false;
        let mut levels: Option<Vec<Level>> = None;
        let mut allowed = Vec::new();
        let mut rest = args.iter();
        while let Some(arg) = rest.next() {
            if let Some(state) = SharedState::ALL.into_iter().find(|s| s.option() == *arg) {
                if !allowed.contains(&state) {
                    allowed.push(state);
                }
                continue;
            }
            let list = match *arg {
                "--quick" => {
                    quick = true;
                    continue;
                }
                "--level" => rest
                    .next()
                    .ok_or("--level vuole un elenco di livelli, per esempio 1,2,3")?,
                other => match other.strip_prefix("--level=") {
                    Some(list) => list,
                    None => return Err(format!("opzione sconosciuta: {other}")),
                },
            };
            if levels.is_some() {
                return Err("--level ripetuto".to_owned());
            }
            levels = Some(parse_levels(list)?);
        }
        allowed.sort_unstable();
        Ok(Options {
            quick,
            levels: levels.unwrap_or_else(|| Level::ALL.to_vec()),
            allowed,
        })
    }

    fn has(&self, level: Level) -> bool {
        self.levels.contains(&level)
    }

    /// Il comando come si riscrive per ripeterlo.
    fn invocation(&self) -> String {
        let mut text = "cargo xtask test-plan".to_owned();
        if self.quick {
            text.push_str(" --quick");
        }
        if self.levels != Level::ALL {
            let numbers: Vec<String> = self.levels.iter().map(|l| l.number().to_string()).collect();
            text.push_str(&format!(" --level {}", numbers.join(",")));
        }
        for state in &self.allowed {
            text.push(' ');
            text.push_str(state.option());
        }
        text
    }
}

/// `1,2,3` → livelli ordinati, senza ripetizioni.
fn parse_levels(list: &str) -> Result<Vec<Level>, String> {
    let mut levels = Vec::new();
    for part in list.split(',') {
        let level = part
            .trim()
            .parse::<u8>()
            .ok()
            .and_then(Level::from_number)
            .ok_or_else(|| format!("livello non valido: «{}» (da 1 a 5)", part.trim()))?;
        if !levels.contains(&level) {
            levels.push(level);
        }
    }
    levels.sort_unstable();
    Ok(levels)
}

/// L'esito di una voce.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Passed,
    /// Voce obbligatoria fallita: il comando esce con errore.
    Failed(String),
    /// Misura riuscita ma sopra la soglia proposta dal piano: un avviso.
    AboveThreshold(String),
    /// Non eseguita per un'opzione (`--quick`) o perché manca il prerequisito.
    Skipped(String),
    /// Non prevista su questo sistema o in questo stato della macchina.
    NotApplicable(String),
    /// Verificata fuori da questo comando (per esempio il fuzzing in CI).
    Elsewhere(String),
    /// Da eseguire a mano.
    Manual(String),
}

impl Verdict {
    /// L'esito in parole, per il rapporto.
    pub fn text(&self) -> String {
        match self {
            Verdict::Passed => "superata".to_owned(),
            Verdict::Failed(detail) => format!("FALLITA: {detail}"),
            Verdict::AboveThreshold(detail) => format!("sopra soglia: {detail}"),
            Verdict::Skipped(reason) => format!("non eseguita: {reason}"),
            Verdict::NotApplicable(reason) => format!("non applicabile: {reason}"),
            Verdict::Elsewhere(reason) => format!("altrove: {reason}"),
            Verdict::Manual(reference) => format!("a mano: {reference}"),
        }
    }
}

/// Una voce del rapporto.
#[derive(Debug, Clone)]
pub struct Entry {
    pub level: Level,
    /// ID del piano (T-A01, P-01, M-18) o della voce del comando (L1-check).
    pub id: String,
    pub title: String,
    /// Il comando che la ripete, se c'è.
    pub command: String,
    pub verdict: Verdict,
    pub duration: Option<Duration>,
    /// La riga della misura (livello 3).
    pub measure: Option<MeasureLine>,
    pub note: String,
    /// Le ultime righe dell'uscita, per un fallimento.
    pub excerpt: Option<String>,
}

impl Entry {
    pub fn new(level: Level, id: &str, title: &str, command: &str, verdict: Verdict) -> Entry {
        Entry {
            level,
            id: id.to_owned(),
            title: title.to_owned(),
            command: command.to_owned(),
            verdict,
            duration: None,
            measure: None,
            note: String::new(),
            excerpt: None,
        }
    }
}

/// Esegue il piano secondo `options`, scrive il rapporto e stampa il riepilogo; errore se
/// fallisce una voce obbligatoria.
pub fn run(options: &Options) -> Result<()> {
    let root = workspace_root()?;
    let report_dir = root.join(REPORT_DIR);
    let stamp = report::UtcStamp::now()?;
    let started = Instant::now();
    let invocation = options.invocation();
    eprintln!("xtask: {invocation}");
    let environment = environment::collect(&root);

    let mut entries = Vec::new();
    if options.has(Level::One) || options.has(Level::Two) {
        suites::run(
            &root,
            options.quick,
            options.has(Level::One),
            options.has(Level::Two),
            &mut entries,
        );
    }
    if options.has(Level::Three) {
        measures::run_all(&root, options.quick, &options.allowed, &mut entries);
    }
    if options.has(Level::Four) {
        system::run(
            &root,
            &report_dir,
            options.quick,
            &options.allowed,
            &mut entries,
        );
    }
    if options.has(Level::Five) {
        entries.extend(USABILITY_TASKS.iter().map(|(id, title)| {
            Entry::new(
                Level::Five,
                id,
                title,
                "",
                Verdict::Manual("con tre persone: docs/piano-di-test.md, livello 5".to_owned()),
            )
        }));
    }
    link_package_items(&mut entries);

    let redaction = report::Redaction::for_machine(&root);
    let text = report::render(&report::Report {
        stamp,
        invocation: &invocation,
        environment: &environment,
        entries: &entries,
        total: started.elapsed(),
        redaction: &redaction,
    });
    let path = report::write(&report_dir, stamp, &text)?;
    let shown = path.strip_prefix(&root).unwrap_or(Path::new(REPORT_DIR));
    eprintln!(
        "{}",
        report::summary(
            &entries,
            &shown.display().to_string().replace('\\', "/"),
            &redaction
        )
    );
    let failed: Vec<&str> = entries
        .iter()
        .filter(|e| matches!(e.verdict, Verdict::Failed(_)))
        .map(|e| e.id.as_str())
        .collect();
    if !failed.is_empty() {
        bail!(
            "{} voci obbligatorie fallite: {}",
            failed.len(),
            failed.join(", ")
        );
    }
    Ok(())
}

/// Le voci del livello 2 verificate solo sul pacchetto (T-D03) prendono l'esito di
/// `cargo xtask dist` (L4-dist), se il pacchetto è stato costruito.
fn link_package_items(entries: &mut [Entry]) {
    let Some(dist) = entries
        .iter()
        .find(|e| e.id == "L4-dist" && !matches!(e.verdict, Verdict::Skipped(_)))
        .map(|e| (e.verdict.clone(), e.duration))
    else {
        return;
    };
    for entry in entries.iter_mut() {
        if entry.level == Level::Two && entry.command == "cargo xtask dist" {
            entry.verdict = dist.0.clone();
            entry.duration = dist.1;
            entry.note = "verificata da cargo xtask dist (L4-dist)".to_owned();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args)
    }

    #[test]
    fn options_partitions() {
        assert_eq!(
            parse(&[]),
            Ok(Options {
                quick: false,
                levels: Level::ALL.to_vec(),
                allowed: Vec::new()
            })
        );
        assert_eq!(
            parse(&[
                "--clipboard",
                "--quick",
                "--level",
                "3,1,2,1",
                "--clipboard"
            ]),
            Ok(Options {
                quick: true,
                levels: vec![Level::One, Level::Two, Level::Three],
                allowed: vec![SharedState::Clipboard]
            })
        );
        assert_eq!(
            parse(&["--level=4"]).map(|o| o.levels),
            Ok(vec![Level::Four])
        );
        assert!(parse(&["--level"]).is_err(), "elenco mancante");
        assert!(parse(&["--level", "0"]).is_err(), "sotto l'intervallo");
        assert!(parse(&["--level", "6"]).is_err(), "sopra l'intervallo");
        assert!(parse(&["--level", "1,,2"]).is_err(), "voce vuota");
        assert!(
            parse(&["--level", "1", "--level", "2"]).is_err(),
            "ripetuto"
        );
        assert!(parse(&["--veloce"]).is_err(), "sconosciuta");
    }

    #[test]
    fn invocation_round_trip() {
        for args in [
            vec![],
            vec!["--quick"],
            vec!["--level", "1,2,3"],
            vec!["--quick", "--level", "4"],
            vec!["--registry", "--clipboard"],
        ] {
            let options = parse(&args).expect("valide");
            let text = options.invocation();
            let again: Vec<&str> = text.split(' ').skip(3).collect();
            assert_eq!(parse(&again), Ok(options), "{text}");
        }
    }

    #[test]
    fn dist_items_take_the_package_verdict() {
        let mut entries = vec![
            Entry::new(
                Level::Two,
                "T-D03",
                "dll",
                "cargo xtask dist",
                Verdict::Skipped("non costruito".to_owned()),
            ),
            Entry::new(
                Level::Four,
                "L4-dist",
                "pacchetto",
                "cargo xtask dist",
                Verdict::Failed("rotto".to_owned()),
            ),
        ];
        link_package_items(&mut entries);
        assert_eq!(entries[0].verdict, Verdict::Failed("rotto".to_owned()));
        assert_eq!(entries[1].verdict, Verdict::Failed("rotto".to_owned()));

        let mut skipped = vec![
            Entry::new(
                Level::Two,
                "T-D03",
                "dll",
                "cargo xtask dist",
                Verdict::Skipped("non costruito".to_owned()),
            ),
            Entry::new(
                Level::Four,
                "L4-dist",
                "pacchetto",
                "cargo xtask dist",
                Verdict::Skipped("--quick".to_owned()),
            ),
        ];
        link_package_items(&mut skipped);
        assert!(matches!(skipped[0].verdict, Verdict::Skipped(_)));
    }
}
