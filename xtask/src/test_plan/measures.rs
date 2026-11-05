// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Livello 3 del piano: ogni misura P-xx con il suo test `#[ignore]`, uno per processo e in
//! release; lettura della riga `MISURA` e confronto con le soglie del piano
//! ([`crate::plan_map::MEASURES`]).
//!
//! Formato della riga (scritta da `crates/lynkeus-engine/tests/misura/formato.rs`, anche
//! per P-04, test di libreria del motore): `MISURA <ID> chiave=valore …`, con almeno
//! `samples`, `p50_ms`, `p95_ms`, `max_ms`, `mem_mb` e `mem_kind`.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use super::capture::{self, Echo};
use super::{Entry, Level, Verdict};
use crate::plan_map::{Length, MEASURES, Measure, SharedState, TestRef, Threshold};
use crate::process::cargo;

/// Tempo massimo di una misura (P-01 crea e cancella 100.000 file).
const MEASURE_TIMEOUT: Duration = Duration::from_secs(45 * 60);

/// Prefisso della riga di una misura.
const LINE_PREFIX: &str = "MISURA";

/// La riga `MISURA` di una misura.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MeasureLine {
    pub values: BTreeMap<String, String>,
}

impl MeasureLine {
    /// Il valore numerico di `key`, se c'è ed è un numero finito.
    pub fn number(&self, key: &str) -> Option<f64> {
        self.values
            .get(key)?
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
    }

    /// Il valore testuale di `key`.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
}

/// Il testo da `prefix` alla fine della riga, se `prefix` comincia una parola della riga:
/// con `--nocapture` e un solo thread libtest scrive `test nome ... ` prima dell'uscita
/// del test, sulla stessa riga.
fn from_word<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    line.match_indices(prefix)
        .find(|(start, _)| {
            line[..*start]
                .chars()
                .next_back()
                .is_none_or(char::is_whitespace)
        })
        .map(|(start, _)| &line[start..])
}

/// La riga `MISURA <id>` in `output`, se c'è (l'ultima, se ripetuta).
pub fn find_line(output: &str, id: &str) -> Option<MeasureLine> {
    let prefix = format!("{LINE_PREFIX} {id} ");
    output.lines().rev().find_map(|line| {
        let rest = from_word(line, &prefix)?;
        let values = rest
            .split_whitespace()
            .skip(2)
            .filter_map(|pair| pair.split_once('='))
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect();
        Some(MeasureLine { values })
    })
}

/// Esito del confronto con le soglie.
#[derive(Debug, Clone, PartialEq)]
pub enum Judgement {
    /// Tutte rispettate.
    Within,
    /// Qualche soglia superata (in parole).
    Above(Vec<String>),
    /// Valori mancanti per confrontare (in parole).
    Missing(Vec<String>),
}

/// Confronta `line` con `thresholds`.
pub fn judge(line: &MeasureLine, thresholds: &[Threshold]) -> Judgement {
    let mut above = Vec::new();
    let mut missing = Vec::new();
    for threshold in thresholds {
        let (key, limit, inclusive) = match *threshold {
            Threshold::AtMost { key, limit } => (key, Some(limit), true),
            Threshold::Below { key, limit } => (key, Some(limit), false),
            Threshold::BelowKey { key, other } => (key, line.number(other), false),
        };
        let (Some(value), Some(limit)) = (line.number(key), limit) else {
            missing.push(describe(threshold));
            continue;
        };
        let within = if inclusive {
            value <= limit
        } else {
            value < limit
        };
        if !within {
            above.push(format!(
                "{key} = {} ({})",
                decimal(value),
                describe(threshold)
            ));
        }
    }
    if !missing.is_empty() {
        Judgement::Missing(missing)
    } else if above.is_empty() {
        Judgement::Within
    } else {
        Judgement::Above(above)
    }
}

/// La soglia in parole.
pub fn describe(threshold: &Threshold) -> String {
    match *threshold {
        Threshold::AtMost { key, limit } => format!("{key} ≤ {}", decimal(limit)),
        Threshold::Below { key, limit } => format!("{key} < {}", decimal(limit)),
        Threshold::BelowKey { key, other } => format!("{key} < {other}"),
    }
}

/// Un numero con la virgola decimale: al massimo tre decimali, sei sotto il millesimo (i
/// tempi per chiamata), senza zeri finali.
pub fn decimal(value: f64) -> String {
    let places = if (f64::MIN_POSITIVE..0.001).contains(&value.abs()) {
        6
    } else {
        3
    };
    let text = format!("{value:.places$}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.replace('.', ",")
}

/// Esegue le misure del piano e aggiunge una voce per ciascuna; una misura che modifica
/// stato condiviso della macchina gira solo se `allowed` lo comprende.
pub fn run_all(root: &Path, quick: bool, allowed: &[SharedState], entries: &mut Vec<Entry>) {
    for measure in MEASURES {
        let forbidden = measure.touches.filter(|state| !allowed.contains(state));
        let entry = if let Some(state) = forbidden {
            skipped(measure, &state.requirement())
        } else if quick && measure.length == Length::Long {
            skipped(measure, "misura lunga, saltata con --quick")
        } else if !measure.test.os.includes_host() {
            skipped(measure, "non prevista su questo sistema")
        } else {
            run_one(root, measure)
        };
        entries.push(entry);
    }
}

fn skipped(measure: &Measure, reason: &str) -> Entry {
    Entry {
        level: Level::Three,
        id: measure.id.to_owned(),
        title: measure.title.to_owned(),
        command: measure.test.command(true),
        verdict: Verdict::Skipped(reason.to_owned()),
        duration: None,
        measure: None,
        note: measure.note.to_owned(),
        excerpt: None,
    }
}

fn run_one(root: &Path, measure: &Measure) -> Entry {
    eprintln!("xtask: ── {} ── {}", measure.id, measure.test.command(true));
    let mut command = cargo();
    command
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .args(["test", "--release", "--locked", "--workspace"])
        .args(measure.test.cargo_args())
        .arg("--")
        .args(TestRef::harness_args(true))
        .args(["--test-threads", "1"]);
    let mut entry = skipped(measure, "");
    match capture::run(command, Echo::All, MEASURE_TIMEOUT) {
        Err(error) => entry.verdict = Verdict::Failed(format!("{error:#}")),
        Ok(captured) => {
            entry.duration = Some(captured.elapsed);
            let line = find_line(&captured.output, measure.id);
            entry.verdict = match (&line, captured.success()) {
                (_, false) => {
                    entry.excerpt = Some(captured.tail(40));
                    Verdict::Failed(format!("il test di misura: {}", captured.status_text()))
                }
                (None, true) => Verdict::Failed(format!(
                    "valori di {} non trovati nell'uscita (test non eseguito o formato cambiato)",
                    measure.id
                )),
                (Some(line), true) => match judge(line, measure.thresholds) {
                    Judgement::Within => Verdict::Passed,
                    Judgement::Above(list) => Verdict::AboveThreshold(list.join("; ")),
                    Judgement::Missing(list) => {
                        Verdict::Failed(format!("valori mancanti per: {}", list.join("; ")))
                    }
                },
            };
            entry.measure = line;
        }
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "\
running 1 test
MISURA P-04 samples=15 p50_ms=1.250 p95_ms=1.900 max_ms=2.000 mem_mb=40.5 mem_kind=private baseline_p50_ms=9.000
MISURA P-040 samples=1
test p04 ... ok
";

    #[test]
    fn the_line_of_the_right_id_is_read() {
        let line = find_line(OUTPUT, "P-04").expect("riga");
        assert_eq!(line.number("p95_ms"), Some(1.9));
        assert_eq!(line.text("mem_kind"), Some("private"));
        assert_eq!(line.number("samples"), Some(15.0));
        assert!(find_line(OUTPUT, "P-05").is_none());
        assert_eq!(
            find_line(OUTPUT, "P-040").and_then(|l| l.number("samples")),
            Some(1.0)
        );
    }

    /// Con `--nocapture` e un thread libtest scrive il nome del test sulla stessa riga.
    #[test]
    fn the_line_after_the_test_name_is_read() {
        let output = "test p03 ... MISURA P-03 samples=5 p50_ms=386.8 mem_mb=848.4\nok\n";
        let line = find_line(output, "P-03").expect("riga");
        assert_eq!(line.number("samples"), Some(5.0));
        assert_eq!(line.number("mem_mb"), Some(848.4));
        assert!(
            find_line("xMISURA P-03 samples=5", "P-03").is_none(),
            "dentro una parola"
        );
    }

    #[test]
    fn non_numbers_are_not_numbers() {
        let line = find_line(
            "MISURA P-01 mem_mb=not_measured p50_ms=NaN p95_ms=inf",
            "P-01",
        )
        .expect("riga");
        assert_eq!(line.number("mem_mb"), None);
        assert_eq!(line.number("p50_ms"), None);
        assert_eq!(line.number("p95_ms"), None);
        assert_eq!(line.text("mem_mb"), Some("not_measured"));
    }

    #[test]
    fn thresholds_on_and_off_points() {
        let line =
            find_line("MISURA P-01 p95_ms=1500 max_ms=1000 base=1000", "P-01").expect("riga");
        let at_most = |limit| Threshold::AtMost {
            key: "p95_ms",
            limit,
        };
        assert_eq!(judge(&line, &[at_most(1500.0)]), Judgement::Within);
        assert!(matches!(
            judge(&line, &[at_most(1499.0)]),
            Judgement::Above(_)
        ));
        let below = |limit| Threshold::Below {
            key: "max_ms",
            limit,
        };
        assert!(matches!(
            judge(&line, &[below(1000.0)]),
            Judgement::Above(_)
        ));
        assert_eq!(judge(&line, &[below(1000.5)]), Judgement::Within);
        let below_key = Threshold::BelowKey {
            key: "max_ms",
            other: "base",
        };
        assert!(matches!(judge(&line, &[below_key]), Judgement::Above(_)));
        let missing = Threshold::BelowKey {
            key: "max_ms",
            other: "assente",
        };
        assert!(matches!(judge(&line, &[missing]), Judgement::Missing(_)));
    }

    #[test]
    fn decimals_use_the_comma() {
        assert_eq!(decimal(1500.0), "1500");
        assert_eq!(decimal(0.25), "0,25");
        assert_eq!(decimal(12.3456), "12,346");
        assert_eq!(decimal(0.000_25), "0,00025");
        assert_eq!(decimal(0.0), "0");
    }
}
