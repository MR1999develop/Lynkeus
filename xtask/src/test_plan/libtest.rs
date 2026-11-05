// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Lettura dell'uscita di `cargo test` (stdout e stderr nello stesso canale, nell'ordine):
//! le suite (un eseguibile di test o i doc-test di un crate), i test elencati da
//! `-- --list` e gli esiti di un'esecuzione.
//!
//! Righe riconosciute:
//!
//! - `Running unittests src/lib.rs (target/release/deps/lynkeus_app-0123abcd.exe)` e
//!   `Running tests/accettazione.rs (…/accettazione-0123abcd)`: inizio di una suite, il cui
//!   nome è quello dell'eseguibile senza hash;
//! - `Doc-tests lynkeus_core`: inizio dei doc-test, suite `doc:lynkeus_core`;
//! - `nome: test` (elenco), `test nome ... ok|FAILED|ignored` (esito, anche dai test senza
//!   l'harness di libtest, che lo scrivono su stderr);
//! - `test result: ok. 3 passed; 0 failed; 1 ignored; …; finished in 0.52s`: fine della
//!   suite.

use std::collections::{BTreeMap, BTreeSet};

/// Esito di un test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Passed,
    Failed,
    Ignored,
}

/// Il riepilogo finale di una suite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub passed: u32,
    pub failed: u32,
    pub ignored: u32,
    pub finished_s: Option<f64>,
}

/// Una suite di test: un eseguibile o i doc-test di un crate.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Suite {
    /// Nome dell'eseguibile senza hash (`lynkeus_app`, `accettazione`) o `doc:<crate>`.
    pub key: String,
    /// Il sorgente stampato da cargo (`unittests src/lib.rs`, `tests/accettazione.rs`).
    pub source: String,
    /// I test nominati, dall'elenco o da un esito.
    pub listed: BTreeSet<String>,
    /// Gli esiti letti.
    pub outcomes: BTreeMap<String, Status>,
    /// Presente se la suite è arrivata alla fine.
    pub summary: Option<Summary>,
}

/// Le suite nell'ordine in cui compaiono in `output`.
pub fn parse(output: &str) -> Vec<Suite> {
    let mut suites: Vec<Suite> = Vec::new();
    for line in output.lines() {
        if let Some(suite) = suite_header(line) {
            suites.push(suite);
            continue;
        }
        let Some(current) = suites.last_mut() else {
            continue;
        };
        if let Some(summary) = summary_line(line) {
            current.summary = Some(summary);
        } else if let Some(name) = listed_name(line) {
            current.listed.insert(name.to_owned());
        } else {
            for (name, status) in outcomes(line) {
                current.listed.insert(name.to_owned());
                current.outcomes.insert(name.to_owned(), status);
            }
        }
    }
    suites
}

/// L'inizio di una suite, se la riga lo è.
fn suite_header(line: &str) -> Option<Suite> {
    let trimmed = line.trim();
    if let Some(krate) = trimmed.strip_prefix("Doc-tests ") {
        return Some(Suite {
            key: format!("doc:{}", krate.trim()),
            source: "doc-test".to_owned(),
            ..Suite::default()
        });
    }
    let rest = trimmed.strip_prefix("Running ")?;
    let (source, exe) = rest.strip_suffix(')')?.rsplit_once(" (")?;
    Some(Suite {
        key: executable_key(exe)?,
        source: source.replace('\\', "/"),
        ..Suite::default()
    })
}

/// `…/deps/accettazione-0123abcd.exe` → `accettazione`.
fn executable_key(exe: &str) -> Option<String> {
    let file = exe.rsplit(['/', '\\']).next()?;
    let stem = file.strip_suffix(".exe").unwrap_or(file);
    let (name, hash) = stem.rsplit_once('-')?;
    let is_hash = !hash.is_empty() && hash.bytes().all(|b| b.is_ascii_hexdigit());
    (is_hash && !name.is_empty()).then(|| name.to_owned())
}

/// `nome: test` dell'elenco di libtest (non i benchmark).
fn listed_name(line: &str) -> Option<&str> {
    let name = line.trim_end().strip_suffix(": test")?;
    (!name.is_empty() && !name.contains(char::is_whitespace)).then_some(name)
}

/// Gli esiti `test nome ... stato` presenti nella riga (di solito uno; più d'uno se le
/// uscite di due processi si sono mescolate).
fn outcomes(line: &str) -> Vec<(&str, Status)> {
    let mut found = Vec::new();
    for (start, _) in line.match_indices("test ") {
        let at_word_start = start == 0
            || line[..start]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        if !at_word_start {
            continue;
        }
        let rest = &line[start + "test ".len()..];
        let Some((name, after)) = rest.split_once(" ... ") else {
            continue;
        };
        if name.is_empty() || name.contains(char::is_whitespace) || name == "result:" {
            continue;
        }
        let word = after
            .split(|c: char| c.is_whitespace() || c == ',')
            .next()
            .unwrap_or("");
        let status = match word {
            "ok" => Status::Passed,
            "FAILED" => Status::Failed,
            "ignored" => Status::Ignored,
            _ => continue,
        };
        found.push((name, status));
    }
    found
}

/// `test result: …` con i conteggi e la durata.
fn summary_line(line: &str) -> Option<Summary> {
    let rest = line.trim().strip_prefix("test result: ")?;
    let (_, counts) = rest.split_once(". ")?;
    let mut summary = Summary {
        passed: 0,
        failed: 0,
        ignored: 0,
        finished_s: None,
    };
    for part in counts.split(';') {
        let part = part.trim();
        if let Some(seconds) = part.strip_prefix("finished in ") {
            summary.finished_s = seconds.trim_end_matches('s').parse().ok();
            continue;
        }
        let Some((number, label)) = part.split_once(' ') else {
            continue;
        };
        let Ok(number) = number.parse::<u32>() else {
            continue;
        };
        match label {
            "passed" => summary.passed = number,
            "failed" => summary.failed = number,
            "ignored" => summary.ignored = number,
            _ => {}
        }
    }
    Some(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUN: &str = "\
   Compiling lynkeus-app v0.1.0
     Running unittests src\\lib.rs (target\\release\\deps\\lynkeus_app-0a1b2c3d4e5f6789.exe)

running 3 tests
test theme::tests::selection_contrast ... ok
test app::tests::copy_200k ... ignored, misura di prestazioni
test form::tests::rows ... FAILED

test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.52s

     Running tests/processo.rs (target/release/deps/processo-0123abcd)
test sparse_xlsx_does_not_crash_the_worker ...
test sparse_xlsx_does_not_crash_the_worker ... ok
     Running tests\\accettazione.rs (target\\release\\deps\\accettazione-ffee00.exe)
   Doc-tests lynkeus_core

running 1 test
test src\\lib.rs - (line 10) ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.5s
";

    #[test]
    fn suites_outcomes_and_summaries() {
        let suites = parse(RUN);
        let keys: Vec<&str> = suites.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "lynkeus_app",
                "processo",
                "accettazione",
                "doc:lynkeus_core"
            ]
        );
        let app = &suites[0];
        assert_eq!(app.source, "unittests src/lib.rs");
        assert_eq!(
            app.outcomes.get("theme::tests::selection_contrast"),
            Some(&Status::Passed)
        );
        assert_eq!(
            app.outcomes.get("app::tests::copy_200k"),
            Some(&Status::Ignored)
        );
        assert_eq!(app.outcomes.get("form::tests::rows"), Some(&Status::Failed));
        assert_eq!(
            app.summary,
            Some(Summary {
                passed: 1,
                failed: 1,
                ignored: 1,
                finished_s: Some(0.52)
            })
        );
        // Senza harness: esito su stderr, nessun riepilogo.
        assert_eq!(
            suites[1]
                .outcomes
                .get("sparse_xlsx_does_not_crash_the_worker"),
            Some(&Status::Passed)
        );
        assert_eq!(suites[1].summary, None);
        // Interrotta: nessun esito, nessun riepilogo.
        assert!(suites[2].outcomes.is_empty() && suites[2].summary.is_none());
        // Doc-test con spazi nel nome: contano solo nel riepilogo.
        assert!(suites[3].outcomes.is_empty());
        assert_eq!(suites[3].summary.map(|s| s.passed), Some(1));
    }

    #[test]
    fn list_output_names_every_test() {
        let list = "\
     Running tests/interfaccia.rs (target/debug/deps/interfaccia-abc123)
row_selection_is_exposed: test
f6_moves_between_panes: test
bench_x: bench

2 tests, 1 benchmark
";
        let suites = parse(list);
        assert_eq!(suites.len(), 1);
        let names: Vec<&str> = suites[0].listed.iter().map(String::as_str).collect();
        assert_eq!(
            names,
            ["f6_moves_between_panes", "row_selection_is_exposed"]
        );
    }

    #[test]
    fn mixed_lines_and_noise() {
        let suites = parse(
            "     Running tests/a.rs (deps/a-00ff)\n\
             rumore test a ... ok test b ... FAILED\n\
             latest x ... ok\n\
             test spaced name ... ok\n\
             test c ... boh\n",
        );
        let outcomes = &suites[0].outcomes;
        assert_eq!(outcomes.get("a"), Some(&Status::Passed));
        assert_eq!(outcomes.get("b"), Some(&Status::Failed));
        assert_eq!(outcomes.len(), 2, "{outcomes:?}");
    }

    #[test]
    fn executable_key_partitions() {
        assert_eq!(
            executable_key("target/release/deps/lynkeus_engine-9f8e.exe").as_deref(),
            Some("lynkeus_engine")
        );
        assert_eq!(
            executable_key("deps/prestazioni_interfaccia-01ab").as_deref(),
            Some("prestazioni_interfaccia")
        );
        assert_eq!(executable_key("deps/senza_hash"), None);
        assert_eq!(executable_key("deps/nome-nonhex"), None);
        assert_eq!(executable_key("deps/-0a"), None);
    }

    #[test]
    fn lines_before_any_suite_are_ignored() {
        assert!(parse("test a ... ok\ntest result: ok. 1 passed;\n").is_empty());
    }
}
