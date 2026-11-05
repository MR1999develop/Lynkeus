// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Livelli 1 e 2 del piano: `cargo xtask check` e il controllo di compilazione per Linux
//! (saltati con `--quick`), l'elenco dei test (`cargo test -- --list`, che compila tutto in
//! release), l'esecuzione delle suite in release; poi gli esiti delle voci T-xx secondo la
//! mappa ([`crate::plan_map::TEST_ITEMS`]) e il controllo della mappa stessa.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use super::capture::{self, Captured, Echo};
use super::libtest::{self, Status, Suite};
use super::{Entry, Level, Verdict};
use crate::check;
use crate::plan_map::{Automation, Coverage, MANUAL_CASES, MEASURES, TEST_ITEMS, TestRef};
use crate::process::cargo;

/// Tempo massimo della compilazione con l'elenco e dell'esecuzione delle suite.
const SUITE_TIMEOUT: Duration = Duration::from_secs(120 * 60);

/// Argomenti comuni delle due chiamate a `cargo test`.
const CARGO_TEST: [&str; 5] = [
    "test",
    "--release",
    "--workspace",
    "--locked",
    "--no-fail-fast",
];

/// Esegue i livelli 1 e 2 richiesti (`level_1`, `level_2`) e aggiunge le voci.
pub fn run(root: &Path, quick: bool, level_1: bool, level_2: bool, entries: &mut Vec<Entry>) {
    if level_1 {
        entries.push(in_process(
            "L1-check",
            "fmt, clippy, test in debug, cargo deny",
            "cargo xtask check",
            quick,
            check::run_all,
        ));
        entries.push(if cfg!(windows) {
            in_process(
                "L1-linux",
                "compilazione per il target Linux",
                "cargo xtask check-linux",
                quick,
                check::run_linux,
            )
        } else {
            Entry::new(
                Level::One,
                "L1-linux",
                "compilazione per il target Linux",
                "cargo xtask check-linux",
                Verdict::NotApplicable("sistema Linux: la compilano le suite".to_owned()),
            )
        });
    }

    let list_command = format!("cargo {} -- --list", CARGO_TEST.join(" "));
    eprintln!("xtask: ── elenco dei test ── {list_command}");
    let listing = cargo_test(root, &["--list"], Echo::WithoutTestList);
    let mut list_entry = Entry::new(
        Level::One,
        "L1-elenco",
        "compilazione in release ed elenco dei test",
        &list_command,
        Verdict::Passed,
    );
    let listed = settle(&mut list_entry, &listing);
    // Senza il livello 1 la voce compare solo se la compilazione fallisce: altrimenti le
    // voci T-xx direbbero "non trovato" senza la causa.
    if level_1 || matches!(list_entry.verdict, Verdict::Failed(_)) {
        entries.push(list_entry);
    }

    let run_command = format!("cargo {}", CARGO_TEST.join(" "));
    eprintln!("xtask: ── suite in release ── {run_command}");
    let execution = cargo_test(root, &[], Echo::All);
    let mut run_entry = Entry::new(
        Level::One,
        "L1-suite",
        "tutte le suite del workspace in release",
        &run_command,
        Verdict::Passed,
    );
    let suites = settle(&mut run_entry, &execution);
    let failed = execution
        .as_ref()
        .map(|c| failed_targets(&c.output))
        .unwrap_or_default();
    if level_1 {
        entries.push(run_entry);
        entries.extend(suites.iter().map(|suite| suite_entry(suite, &failed)));
    }
    if level_2 {
        entries.push(map_entry(&listed));
        entries.extend(
            TEST_ITEMS
                .iter()
                .map(|item| test_item_entry(item.id, item.title, item.coverage, &suites, &listed)),
        );
    }
}

/// Una voce eseguita nel processo di xtask (uscita sul terminale), saltata con `--quick`.
fn in_process(
    id: &str,
    title: &str,
    command: &str,
    quick: bool,
    step: fn() -> anyhow::Result<()>,
) -> Entry {
    let mut entry = Entry::new(Level::One, id, title, command, Verdict::Passed);
    if quick {
        entry.verdict = Verdict::Skipped("saltato con --quick".to_owned());
        return entry;
    }
    eprintln!("xtask: ── {id} ── {command}");
    let started = Instant::now();
    let result = step();
    entry.duration = Some(started.elapsed());
    if let Err(error) = result {
        entry.verdict = Verdict::Failed(format!("{error:#}"));
    }
    entry
}

/// `cargo test` in release su tutto il workspace, con `extra` dopo `--`.
fn cargo_test(root: &Path, extra: &[&str], echo: Echo) -> anyhow::Result<Captured> {
    let mut command = cargo();
    command
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .args(CARGO_TEST)
        .arg("--")
        .args(extra);
    capture::run(command, echo, SUITE_TIMEOUT)
}

/// Esito e durata di un `cargo test` nella voce; le suite lette dall'uscita.
fn settle(entry: &mut Entry, captured: &anyhow::Result<Captured>) -> Vec<Suite> {
    match captured {
        Err(error) => {
            entry.verdict = Verdict::Failed(format!("{error:#}"));
            Vec::new()
        }
        Ok(captured) => {
            entry.duration = Some(captured.elapsed);
            if !captured.success() {
                entry.verdict = Verdict::Failed(format!("cargo test: {}", captured.status_text()));
                entry.excerpt = Some(captured.tail(40));
            }
            if captured.truncated {
                entry.note = "uscita troppo lunga: conservata solo in parte".to_owned();
            }
            libtest::parse(&captured.output)
        }
    }
}

/// Le suite fallite secondo cargo: i bersagli tra apici inversi nei suoi messaggi
/// (`` `-p lynkeus-extract --test processo` ``), come nome della suite.
pub fn failed_targets(output: &str) -> BTreeSet<String> {
    let mut failed = BTreeSet::new();
    for line in output.lines() {
        for quoted in line.split('`').skip(1).step_by(2) {
            let words: Vec<&str> = quoted.split_whitespace().collect();
            let key = match words.as_slice() {
                ["-p", package, "--lib"] => package.replace('-', "_"),
                ["-p", package, "--doc"] => format!("doc:{}", package.replace('-', "_")),
                ["-p", _, "--test" | "--bin", name] => name.replace('-', "_"),
                _ => continue,
            };
            failed.insert(key);
        }
    }
    failed
}

/// La voce di una suite del livello 1.
fn suite_entry(suite: &Suite, failed: &BTreeSet<String>) -> Entry {
    let mut entry = Entry::new(
        Level::One,
        &format!("L1-{}", suite.key),
        &suite.source,
        "parte di L1-suite",
        Verdict::Passed,
    );
    entry.duration = suite
        .summary
        .and_then(|s| s.finished_s)
        .and_then(|s| Duration::try_from_secs_f64(s).ok());
    let failing: Vec<&str> = suite
        .outcomes
        .iter()
        .filter(|(_, status)| **status == Status::Failed)
        .map(|(name, _)| name.as_str())
        .collect();
    entry.note = match suite.summary {
        Some(summary) => format!(
            "{} superati, {} falliti, {} ignorati",
            summary.passed, summary.failed, summary.ignored
        ),
        None => format!("{} esiti letti, senza riepilogo", suite.outcomes.len()),
    };
    if !failing.is_empty() {
        entry.verdict = Verdict::Failed(format!("falliti: {}", failing.join(", ")));
    } else if failed.contains(&suite.key) || suite.summary.is_some_and(|s| s.failed > 0) {
        entry.verdict = Verdict::Failed("la suite non è finita con successo".to_owned());
    }
    entry
}

/// Tutti i test della mappa che valgono su questo sistema.
fn mapped_tests() -> Vec<TestRef> {
    let mut refs = Vec::new();
    for item in TEST_ITEMS {
        if let Coverage::Tests(list) | Coverage::TestsAndDist(list) = item.coverage {
            refs.extend_from_slice(list);
        }
    }
    refs.extend(MEASURES.iter().map(|m| m.test));
    refs.extend(MANUAL_CASES.iter().filter_map(|c| match c.automation {
        Automation::IgnoredTest(test) => Some(test),
        _ => None,
    }));
    refs.retain(|r| r.os.includes_host());
    refs
}

/// Vero se `test` è tra i test nominati nella sua suite.
fn is_listed(test: &TestRef, suites: &[Suite]) -> bool {
    suites
        .iter()
        .filter(|suite| suite.key == test.suite())
        .any(|suite| suite.listed.iter().any(|name| test.matches(name)))
}

/// Il controllo della mappa: ogni test citato esiste nell'elenco di `cargo test -- --list`.
fn map_entry(listed: &[Suite]) -> Entry {
    let mut entry = Entry::new(
        Level::Two,
        "MAPPA",
        "ogni test della mappa del piano esiste (xtask/src/plan_map.rs)",
        "cargo test --release --workspace -- --list",
        Verdict::Passed,
    );
    if listed.is_empty() {
        entry.verdict = Verdict::Failed("elenco dei test non disponibile".to_owned());
        return entry;
    }
    let missing: Vec<String> = mapped_tests()
        .iter()
        .filter(|test| !is_listed(test, listed))
        .map(|test| format!("{} {}", test.suite(), test.name))
        .collect();
    if !missing.is_empty() {
        entry.verdict = Verdict::Failed(format!("non trovati: {}", missing.join(", ")));
    }
    entry
}

/// La voce T-xx dagli esiti delle suite.
pub fn test_item_entry(
    id: &str,
    title: &str,
    coverage: Coverage,
    suites: &[Suite],
    listed: &[Suite],
) -> Entry {
    let (tests, also_dist) = match coverage {
        Coverage::Tests(tests) => (tests, false),
        Coverage::TestsAndDist(tests) => (tests, true),
        Coverage::Dist => {
            return Entry::new(
                Level::Two,
                id,
                title,
                "cargo xtask dist",
                Verdict::Skipped("pacchetto non costruito (livello 4, senza --quick)".to_owned()),
            );
        }
        Coverage::Elsewhere(reason) => {
            return Entry::new(
                Level::Two,
                id,
                title,
                "",
                Verdict::Elsewhere(reason.to_owned()),
            );
        }
    };
    let here: Vec<&TestRef> = tests.iter().filter(|t| t.os.includes_host()).collect();
    let command = here
        .iter()
        .map(|t| t.command(false))
        .collect::<Vec<_>>()
        .join(" ; ");
    let mut entry = Entry::new(Level::Two, id, title, &command, Verdict::Passed);
    if also_dist {
        entry.note = "verificata anche sul pacchetto da cargo xtask dist (livello 4)".to_owned();
    }
    if here.is_empty() {
        let mut systems: Vec<&str> = tests.iter().map(|t| t.os.label()).collect();
        systems.dedup();
        entry.verdict = Verdict::NotApplicable(format!("solo su {}", systems.join(", ")));
        return entry;
    }
    let mut problems = Vec::new();
    let mut seconds = 0.0;
    for test in here {
        let suite = suites.iter().find(|s| s.key == test.suite());
        if let Some(finished) = suite.and_then(|s| s.summary).and_then(|s| s.finished_s) {
            seconds += finished;
        }
        let statuses: Vec<Status> = suite
            .map(|s| {
                s.outcomes
                    .iter()
                    .filter(|(name, _)| test.matches(name))
                    .map(|(_, status)| *status)
                    .collect()
            })
            .unwrap_or_default();
        if statuses.is_empty() {
            problems.push(if is_listed(test, listed) {
                format!("{}: non eseguito (suite interrotta)", test.name)
            } else {
                format!("{}: test non trovato", test.name)
            });
        } else if statuses.contains(&Status::Failed) {
            problems.push(format!("{}: fallito", test.name));
        } else if statuses.contains(&Status::Ignored) {
            problems.push(format!("{}: ignorato, quindi non eseguito", test.name));
        }
    }
    if let Ok(duration) = Duration::try_from_secs_f64(seconds)
        && !duration.is_zero()
    {
        entry.duration = Some(duration);
        if entry.note.is_empty() {
            entry.note = "durata della suite che lo contiene".to_owned();
        }
    }
    if !problems.is_empty() {
        entry.verdict = Verdict::Failed(problems.join("; "));
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_map::{Os, Target};

    fn suites(output: &str) -> Vec<Suite> {
        libtest::parse(output)
    }

    const RUN: &str = "\
     Running unittests src/lib.rs (deps/lynkeus_app-00aa)
test theme::tests::selection_contrast ... ok
test theme::tests::focus_ring_contrast ... FAILED
test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.00s
     Running tests/interfaccia.rs (deps/interfaccia-00bb)
";

    const LIST: &str = "\
     Running unittests src/lib.rs (deps/lynkeus_app-00aa)
theme::tests::selection_contrast: test
theme::tests::focus_ring_contrast: test
     Running tests/interfaccia.rs (deps/interfaccia-00bb)
row_selection_is_exposed: test
";

    const fn app_lib(name: &'static str) -> TestRef {
        TestRef {
            package: "lynkeus-app",
            target: Target::Lib,
            name,
            os: Os::Any,
        }
    }

    const PASSED: &[TestRef] = &[app_lib("selection_contrast")];
    const FAILED: &[TestRef] = &[app_lib("focus_ring_contrast")];
    const MISSING: &[TestRef] = &[app_lib("non_esiste")];
    const INTERRUPTED: &[TestRef] = &[TestRef {
        package: "lynkeus-app",
        target: Target::Test("interfaccia"),
        name: "row_selection_is_exposed",
        os: Os::Any,
    }];
    const OTHER_SYSTEM: &[TestRef] = &[TestRef {
        os: if cfg!(windows) {
            Os::Linux
        } else {
            Os::Windows
        },
        ..app_lib("selection_contrast")
    }];

    #[test]
    fn test_item_partitions() {
        let (run, list) = (suites(RUN), suites(LIST));
        let verdict = |tests: &'static [TestRef]| {
            test_item_entry("T-X", "x", Coverage::Tests(tests), &run, &list).verdict
        };
        let failed_with = |tests, text: &str| matches!(verdict(tests), Verdict::Failed(message) if message.contains(text));
        assert_eq!(verdict(PASSED), Verdict::Passed);
        assert!(failed_with(FAILED, "fallito"));
        assert!(failed_with(INTERRUPTED, "interrotta"));
        assert!(failed_with(MISSING, "non trovato"));
        assert!(matches!(verdict(OTHER_SYSTEM), Verdict::NotApplicable(_)));
    }

    #[test]
    fn the_map_check_names_the_missing_tests() {
        assert!(matches!(map_entry(&[]).verdict, Verdict::Failed(_)));
        assert!(is_listed(&PASSED[0], &suites(LIST)));
        assert!(!is_listed(&MISSING[0], &suites(LIST)));
    }

    #[test]
    fn failed_targets_from_cargo_messages() {
        let output = "\
error: test failed, to rerun pass `-p lynkeus-extract --test processo`
error: 3 targets failed:
    `-p lynkeus-app --lib`
    `-p lynkeus-core --doc`
    `-p lynkeus-app --bin lynkeus-searcher`
nota `codice` qualsiasi
";
        let failed: Vec<String> = failed_targets(output).into_iter().collect();
        assert_eq!(
            failed,
            [
                "doc:lynkeus_core",
                "lynkeus_app",
                "lynkeus_searcher",
                "processo"
            ]
        );
    }
}
