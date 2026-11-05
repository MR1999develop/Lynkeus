// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Livello 3 del piano: ogni misura P-xx con il suo test `#[ignore]`, uno per processo e in
//! release; lettura della riga `MISURA` e confronto con le soglie del piano
//! ([`crate::plan_map::MEASURES`]).
//!
//! Formato della riga (scritta da `crates/lynkeus-engine/tests/misura`):
//! `MISURA <ID> chiave=valore …`, con almeno `samples`, `p50_ms`, `p95_ms`, `max_ms`,
//! `mem_mb` e `mem_kind`. Un test scritto prima di questo formato si legge dalle sue righe
//! con le durate ([`crate::plan_map::Output::Labeled`]).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use super::capture::{self, Echo};
use super::{Entry, Level, Verdict};
use crate::plan_map::{Length, MEASURES, Measure, Output, SharedState, TestRef, Threshold};
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

/// I valori di `measure` nell'uscita del suo test, nel formato dichiarato dalla mappa.
pub fn read_values(output: &str, measure: &Measure) -> Option<MeasureLine> {
    match measure.output {
        Output::Line => find_line(output, measure.id),
        Output::Labeled { main, baseline } => {
            let main = labeled(output, measure.id, main)?;
            let baseline = labeled(output, measure.id, baseline)?;
            let mut values = BTreeMap::new();
            let mut put = |key: &str, value: String| {
                values.insert(key.to_owned(), value);
            };
            if let Some(samples) = main.samples {
                put("samples", samples.to_string());
            }
            put("p50_ms", main.p50_ms.to_string());
            put("p95_ms", main.p95_ms.to_string());
            put("max_ms", main.max_ms.to_string());
            put("mem_mb", "not_measured".to_owned());
            put("mem_kind", "none".to_owned());
            put("baseline_p50_ms", baseline.p50_ms.to_string());
            Some(MeasureLine { values })
        }
    }
}

/// Una riga `<id> <etichetta>: p50 …, p95 …, massimo … (N ripetizioni)`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Labeled {
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
    samples: Option<u32>,
}

/// La riga di `id` con `label`, se c'è e si legge tutta.
fn labeled(output: &str, id: &str, label: &str) -> Option<Labeled> {
    let prefix = format!("{id} {label}: ");
    let rest = output
        .lines()
        .rev()
        .find_map(|line| from_word(line.trim_end(), &prefix)?.strip_prefix(prefix.as_str()))?;
    let (durations, samples) = match rest.split_once(" (") {
        Some((durations, tail)) => (
            durations,
            tail.split_whitespace().next().and_then(|n| n.parse().ok()),
        ),
        None => (rest, None),
    };
    let mut parts = durations.split(", ");
    let mut take = |word: &str| -> Option<f64> {
        let value = parts.next()?.trim().strip_prefix(word)?;
        debug_duration_ms(value)
    };
    Some(Labeled {
        p50_ms: take("p50 ")?,
        p95_ms: take("p95 ")?,
        max_ms: take("massimo ")?,
        samples,
    })
}

/// Le unità del formato `Debug` di `Duration` con il loro valore in millisecondi come
/// frazione (numeratore, denominatore); `s` per ultima, perché chiude anche le altre.
const DEBUG_UNITS: [(&str, f64, f64); 5] = [
    ("ms", 1.0, 1.0),
    ("µs", 1.0, 1e3),
    ("us", 1.0, 1e3),
    ("ns", 1.0, 1e6),
    ("s", 1e3, 1.0),
];

/// Una durata nel formato `Debug` di `Duration` (`1.5s`, `12.3ms`, `850µs`, `40ns`) in
/// millisecondi.
fn debug_duration_ms(text: &str) -> Option<f64> {
    let text = text.trim();
    let (number, numerator, denominator) = DEBUG_UNITS
        .iter()
        .find_map(|&(unit, num, den)| Some((text.strip_suffix(unit)?, num, den)))?;
    number
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(|v| v * numerator / denominator)
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
            let line = read_values(&captured.output, measure);
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
    fn debug_durations_partitions() {
        assert_eq!(debug_duration_ms("1.5s"), Some(1500.0));
        assert_eq!(debug_duration_ms("12.25ms"), Some(12.25));
        assert_eq!(debug_duration_ms("850µs"), Some(0.85));
        assert_eq!(debug_duration_ms("40ns"), Some(0.000_04));
        assert_eq!(debug_duration_ms("3 min"), None);
        assert_eq!(debug_duration_ms("-1ms"), None);
        assert_eq!(debug_duration_ms("ms"), None);
    }

    /// P-04 dal test di libreria scritto prima della riga `MISURA`.
    #[test]
    fn labeled_lines_become_measure_values() {
        let output = "\
test filter::tests::prepared_filter_timing ... P-04 confronto con to_lowercase: p50 9.51ms, p95 10.2ms, massimo 11ms (15 ripetizioni)
P-04 filtro preparato: p50 850µs, p95 1.1ms, massimo 1.5ms (15 ripetizioni)
test filter::tests::prepared_filter_timing ... ok
";
        let p04 = MEASURES.iter().find(|m| m.id == "P-04").expect("P-04");
        let line = read_values(output, p04).expect("valori");
        assert_eq!(line.number("p50_ms"), Some(0.85));
        assert_eq!(line.number("max_ms"), Some(1.5));
        assert_eq!(line.number("baseline_p50_ms"), Some(9.51));
        assert_eq!(line.number("samples"), Some(15.0));
        assert_eq!(line.number("mem_mb"), None);
        assert_eq!(judge(&line, p04.thresholds), Judgement::Within);
        assert!(
            read_values("P-04 filtro preparato: p50 1ms", p04).is_none(),
            "incompleta"
        );
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
