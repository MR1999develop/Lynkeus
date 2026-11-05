// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Il rapporto del piano di test: Markdown in `target/test-plan/rapporto-<data e ora>.md`
//! (fuori dal controllo di versione, gli ultimi [`KEPT_REPORTS`] conservati) e il riepilogo
//! su stderr.
//!
//! Nel rapporto non finiscono nome utente, nome del computer né percorsi locali: l'ambiente
//! ha solo dati tecnici; esiti, note ed estratti delle uscite passano da [`Redaction`].

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use super::environment::Environment;
use super::measures::{self, MeasureLine};
use super::{Entry, Level, Verdict};
use crate::plan_map::MEASURES;

/// Rapporti conservati nella cartella; i più vecchi si cancellano.
pub const KEPT_REPORTS: usize = 20;
/// Prefisso dei file dei rapporti.
const REPORT_PREFIX: &str = "rapporto-";

/// Un istante in UTC, letto una volta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtcStamp {
    seconds: u64,
}

impl UtcStamp {
    /// Adesso, dall'orologio di sistema.
    pub fn now() -> Result<UtcStamp> {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("orologio di sistema prima del 1970")?
            .as_secs();
        Ok(UtcStamp { seconds })
    }

    /// (anno, mese, giorno, ore, minuti, secondi).
    fn parts(self) -> (i64, u64, u64, u64, u64, u64) {
        let days = i64::try_from(self.seconds / 86_400).unwrap_or(i64::MAX);
        let (year, month, day) = civil_from_days(days);
        let rest = self.seconds % 86_400;
        (year, month, day, rest / 3600, rest % 3600 / 60, rest % 60)
    }

    /// `2026-11-05 14:30:12 UTC`.
    pub fn human(self) -> String {
        let (y, mo, d, h, mi, s) = self.parts();
        format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC")
    }

    /// `2026-11-05T143012Z`, per il nome del file (ordinabile).
    pub fn file_part(self) -> String {
        let (y, mo, d, h, mi, s) = self.parts();
        format!("{y:04}-{mo:02}-{d:02}T{h:02}{mi:02}{s:02}Z")
    }
}

/// La data civile (anno, mese, giorno) di `days` giorni dal 1970-01-01 (algoritmo di
/// Howard Hinnant, calendario gregoriano prolettico).
fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let z = days.saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        year,
        u64::try_from(month).unwrap_or(0),
        u64::try_from(day).unwrap_or(0),
    )
}

/// Ciò che serve per scrivere il rapporto.
pub struct Report<'a> {
    pub stamp: UtcStamp,
    /// Il comando come l'ha scritto l'utente (`cargo xtask test-plan --quick`).
    pub invocation: &'a str,
    pub environment: &'a Environment,
    pub entries: &'a [Entry],
    pub total: Duration,
    /// Applicata a esiti, note ed estratti delle uscite.
    pub redaction: &'a Redaction,
}

/// Il rapporto in Markdown.
pub fn render(report: &Report<'_>) -> String {
    let mut text = String::new();
    push_line(&mut text, "# Rapporto del piano di test\n");
    push_line(&mut text, &format!("- Data: {}", report.stamp.human()));
    push_line(
        &mut text,
        &format!(
            "- Commit: {}",
            report.environment.get("Commit").unwrap_or("non rilevato")
        ),
    );
    push_line(&mut text, &format!("- Comando: `{}`", report.invocation));
    push_line(
        &mut text,
        &format!("- Durata totale: {}", duration(report.total)),
    );
    push_line(
        &mut text,
        "- Piano: `docs/piano-di-test.md`; mappa degli ID: `xtask/src/plan_map.rs`\n",
    );

    push_line(&mut text, "## Ambiente\n\n| Voce | Valore |\n|---|---|");
    for (name, value) in &report.environment.fields {
        push_line(&mut text, &format!("| {name} | {} |", cell(value)));
    }

    push_line(&mut text, "\n## Riepilogo\n");
    push_line(
        &mut text,
        "| Livello | Superate | Fallite | Sopra soglia | Non eseguite | Non applicabili | Altrove | A mano |",
    );
    push_line(&mut text, "|---|---:|---:|---:|---:|---:|---:|---:|");
    for level in Level::ALL {
        let counts = Counts::of(report.entries.iter().filter(|e| e.level == level));
        if counts.total() == 0 {
            continue;
        }
        push_line(
            &mut text,
            &format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} |",
                level.number(),
                counts.passed,
                counts.failed,
                counts.above,
                counts.skipped,
                counts.not_applicable,
                counts.elsewhere,
                counts.manual
            ),
        );
    }

    for level in Level::ALL {
        let entries: Vec<&Entry> = report.entries.iter().filter(|e| e.level == level).collect();
        if entries.is_empty() {
            continue;
        }
        push_line(&mut text, &format!("\n## {}\n", level.title()));
        if level == Level::Three {
            measure_table(&mut text, &entries, report.redaction);
        } else {
            plain_table(&mut text, &entries, report.redaction);
        }
    }

    let failures: Vec<&Entry> = report
        .entries
        .iter()
        .filter(|e| e.excerpt.is_some())
        .collect();
    if !failures.is_empty() {
        push_line(&mut text, "\n## Dettagli dei fallimenti\n");
        for entry in failures {
            let excerpt = report
                .redaction
                .apply(entry.excerpt.as_deref().unwrap_or(""));
            push_line(
                &mut text,
                &format!(
                    "### {}\n\n```text\n{}\n```\n",
                    entry.id,
                    excerpt.replace("```", "'''")
                ),
            );
        }
    }
    text
}

fn plain_table(text: &mut String, entries: &[&Entry], redaction: &Redaction) {
    push_line(text, "| ID | Voce | Comando | Esito | Durata | Note |");
    push_line(text, "|---|---|---|---|---:|---|");
    for entry in entries {
        push_line(
            text,
            &format!(
                "| {} | {} | {} | {} | {} | {} |",
                entry.id,
                cell(&entry.title),
                command_cell(&entry.command),
                cell(&redaction.apply(&entry.verdict.text())),
                entry.duration.map(duration).unwrap_or_default(),
                cell(&redaction.apply(&entry.note))
            ),
        );
    }
}

fn measure_table(text: &mut String, entries: &[&Entry], redaction: &Redaction) {
    push_line(
        text,
        "| ID | Misura | Esito | Campioni | p50 (ms) | p95 (ms) | Massimo (ms) | Memoria di picco | Soglia | Durata | Note | Comando |",
    );
    push_line(
        text,
        "|---|---|---|---:|---:|---:|---:|---:|---|---:|---|---|",
    );
    for entry in entries {
        let plan = MEASURES.iter().find(|m| m.id == entry.id);
        let thresholds = plan
            .map(|m| {
                m.thresholds
                    .iter()
                    .map(measures::describe)
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        let line = entry.measure.clone().unwrap_or_default();
        let number = |key: &str| line.number(key).map(measures::decimal).unwrap_or_default();
        let mut note = entry.note.clone();
        let extras = extra_values(&line);
        if !extras.is_empty() {
            note = format!("{note}; {extras}");
        }
        push_line(
            text,
            &format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                entry.id,
                cell(&entry.title),
                cell(&redaction.apply(&entry.verdict.text())),
                number("samples"),
                number("p50_ms"),
                number("p95_ms"),
                number("max_ms"),
                memory(&line),
                cell(&thresholds),
                entry.duration.map(duration).unwrap_or_default(),
                cell(&note),
                command_cell(&entry.command)
            ),
        );
    }
}

/// Aggiunge `line` e un a capo a `text`.
fn push_line(text: &mut String, line: &str) {
    text.push_str(line);
    text.push('\n');
}

/// Aggiunge `fragment` a `text`.
fn push_text(text: &mut String, fragment: &str) {
    text.push_str(fragment);
}

/// Le chiavi della riga di misura già nelle colonne.
const STANDARD_KEYS: [&str; 6] = [
    "samples", "p50_ms", "p95_ms", "max_ms", "mem_mb", "mem_kind",
];

/// I valori in più della riga di misura (`chiave = valore`), con la virgola decimale.
fn extra_values(line: &MeasureLine) -> String {
    line.values
        .iter()
        .filter(|(key, _)| !STANDARD_KEYS.contains(&key.as_str()))
        .map(|(key, value)| {
            let shown = line
                .number(key)
                .map_or_else(|| value.clone(), measures::decimal);
            format!("{key} = {shown}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Il picco di memoria con il suo tipo.
fn memory(line: &MeasureLine) -> String {
    match (line.number("mem_mb"), line.text("mem_kind")) {
        (Some(mib), Some("private")) => format!("{} MiB (privata)", measures::decimal(mib)),
        (Some(mib), Some("rss")) => format!("{} MiB (RSS)", measures::decimal(mib)),
        (Some(mib), _) => format!("{} MiB", measures::decimal(mib)),
        (None, _) if line.values.is_empty() => String::new(),
        (None, _) => "non misurata".to_owned(),
    }
}

/// Testo di una cella: niente `|` né a capo.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn command_cell(command: &str) -> String {
    if command.is_empty() || !command.starts_with("cargo") {
        cell(command)
    } else {
        format!("`{}`", cell(command))
    }
}

/// Una durata leggibile: `850 ms`, `12,3 s`, `4 min 05 s`.
pub fn duration(value: Duration) -> String {
    let ms = value.as_millis();
    if ms < 1_000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", value.as_secs_f64()).replace('.', ",")
    } else {
        let seconds = value.as_secs();
        format!("{} min {:02} s", seconds / 60, seconds % 60)
    }
}

/// Conteggi degli esiti.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub passed: usize,
    pub failed: usize,
    pub above: usize,
    pub skipped: usize,
    pub not_applicable: usize,
    pub elsewhere: usize,
    pub manual: usize,
}

impl Counts {
    pub fn of<'a>(entries: impl Iterator<Item = &'a Entry>) -> Counts {
        let mut counts = Counts::default();
        for entry in entries {
            match entry.verdict {
                Verdict::Passed => counts.passed += 1,
                Verdict::Failed(_) => counts.failed += 1,
                Verdict::AboveThreshold(_) => counts.above += 1,
                Verdict::Skipped(_) => counts.skipped += 1,
                Verdict::NotApplicable(_) => counts.not_applicable += 1,
                Verdict::Elsewhere(_) => counts.elsewhere += 1,
                Verdict::Manual(_) => counts.manual += 1,
            }
        }
        counts
    }

    fn total(&self) -> usize {
        self.passed
            + self.failed
            + self.above
            + self.skipped
            + self.not_applicable
            + self.elsewhere
            + self.manual
    }
}

/// Il riepilogo per stderr: conteggi, voci fallite e sopra soglia, il percorso del rapporto.
pub fn summary(entries: &[Entry], report: &str, redaction: &Redaction) -> String {
    let counts = Counts::of(entries.iter());
    let mut text = format!(
        "xtask: piano di test: {} superate, {} fallite, {} sopra soglia, {} non eseguite, \
         {} non applicabili, {} altrove, {} a mano\n",
        counts.passed,
        counts.failed,
        counts.above,
        counts.skipped,
        counts.not_applicable,
        counts.elsewhere,
        counts.manual
    );
    for entry in entries {
        match &entry.verdict {
            Verdict::Failed(detail) => {
                push_line(
                    &mut text,
                    &format!("xtask: FALLITA {}: {}", entry.id, redaction.apply(detail)),
                );
            }
            Verdict::AboveThreshold(detail) => {
                push_line(
                    &mut text,
                    &format!("xtask: sopra soglia {}: {detail}", entry.id),
                );
            }
            _ => {}
        }
    }
    push_text(&mut text, &format!("xtask: rapporto: {report}"));
    text
}

/// Scrive `text` in `dir/rapporto-<stamp>.md` (file temporaneo, `sync_all`, rinomina) e
/// cancella i rapporti oltre gli ultimi [`KEPT_REPORTS`].
pub fn write(dir: &Path, stamp: UtcStamp, text: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("creazione di {}", dir.display()))?;
    let path = dir.join(format!("{REPORT_PREFIX}{}.md", stamp.file_part()));
    let temporary = path.with_extension("md.tmp");
    {
        let mut file = std::fs::File::create(&temporary)
            .with_context(|| format!("creazione di {}", temporary.display()))?;
        file.write_all(text.as_bytes())
            .and_then(|()| file.sync_all())
            .with_context(|| format!("scrittura di {}", temporary.display()))?;
    }
    std::fs::rename(&temporary, &path)
        .with_context(|| format!("rinomina in {}", path.display()))?;
    prune(dir);
    Ok(path)
}

/// Cancella i rapporti più vecchi oltre [`KEPT_REPORTS`]; un errore qui non ferma il piano.
fn prune(dir: &Path) {
    let Ok(items) = std::fs::read_dir(dir) else {
        return;
    };
    let mut reports: Vec<PathBuf> = items
        .flatten()
        .map(|item| item.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(REPORT_PREFIX) && n.ends_with(".md"))
        })
        .collect();
    reports.sort();
    let excess = reports.len().saturating_sub(KEPT_REPORTS);
    for old in reports.into_iter().take(excess) {
        if std::fs::remove_file(&old).is_err() {
            eprintln!("xtask: rapporto vecchio non cancellato: {}", old.display());
        }
    }
}

/// Le sostituzioni dei dati locali nei testi del rapporto: cartella del progetto, cartella
/// utente e nome utente nei percorsi, con `\` o `/`, senza distinguere le maiuscole ASCII.
#[derive(Debug, Clone, Default)]
pub struct Redaction {
    /// (testo locale, segnaposto), nell'ordine in cui si applicano.
    replacements: Vec<(String, String)>,
}

impl Redaction {
    /// Le sostituzioni per questa macchina, lette una volta; `root` è la cartella del
    /// progetto.
    pub fn for_machine(root: &Path) -> Redaction {
        let env = |name: &str| std::env::var(name).ok().filter(|v| v.len() >= 2);
        let mut pairs = vec![(root.display().to_string(), "<repo>".to_owned())];
        if let Some(home) = env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
            pairs.push((home, "~".to_owned()));
        }
        if let Some(user) = env("USERNAME").or_else(|| env("USER")) {
            pairs.push((format!("\\{user}\\"), "\\<utente>\\".to_owned()));
        }
        let mut replacements = Vec::new();
        for (needle, replacement) in pairs {
            let forward = (needle.replace('\\', "/"), replacement.replace('\\', "/"));
            if forward.0 != needle {
                replacements.push(forward);
            }
            replacements.push((needle, replacement));
        }
        Redaction { replacements }
    }

    /// `text` con i dati locali sostituiti dai segnaposti.
    pub fn apply(&self, text: &str) -> String {
        self.replacements
            .iter()
            .fold(text.to_owned(), |out, (needle, replacement)| {
                replace_ignoring_ascii_case(&out, needle, replacement)
            })
    }
}

/// Sostituisce ogni `needle` in `text` senza distinguere le maiuscole ASCII.
fn replace_ignoring_ascii_case(text: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return text.to_owned();
    }
    // `to_ascii_lowercase` non cambia la lunghezza in byte: gli indici valgono per `text`.
    let lower_text = text.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, _) in lower_text.match_indices(&lower_needle) {
        if start < last {
            continue;
        }
        out.push_str(&text[last..start]);
        out.push_str(replacement);
        last = start + needle.len();
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_from_unix_seconds() {
        let stamp = |seconds| UtcStamp { seconds };
        assert_eq!(stamp(0).human(), "1970-01-01 00:00:00 UTC");
        assert_eq!(stamp(951_782_400).human(), "2000-02-29 00:00:00 UTC");
        assert_eq!(stamp(1_793_889_012).file_part(), "2026-11-05T143012Z");
        assert_eq!(stamp(4_107_542_399).human(), "2100-02-28 23:59:59 UTC");
    }

    #[test]
    fn durations_partitions() {
        assert_eq!(duration(Duration::from_millis(850)), "850 ms");
        assert_eq!(duration(Duration::from_millis(12_340)), "12,3 s");
        assert_eq!(duration(Duration::from_secs(245)), "4 min 05 s");
    }

    #[test]
    fn cells_have_no_pipes_or_newlines() {
        assert_eq!(cell("a|b\nc"), "a\\|b c");
    }

    #[test]
    fn case_insensitive_replacement() {
        assert_eq!(
            replace_ignoring_ascii_case(
                r"C:\Users\Nome\x e c:\users\nome\y",
                r"c:\users\nome",
                "~"
            ),
            r"~\x e ~\y"
        );
        assert_eq!(replace_ignoring_ascii_case("città", "TT", "-"), "ci-à");
        assert_eq!(replace_ignoring_ascii_case("abc", "", "-"), "abc");
    }

    #[test]
    fn counts_and_summary() {
        let entries = [
            Entry::new(Level::Two, "T-A01", "a", "", Verdict::Passed),
            Entry::new(
                Level::Two,
                "T-A02",
                "b",
                "",
                Verdict::Failed("rotto".to_owned()),
            ),
            Entry::new(
                Level::Three,
                "P-01",
                "c",
                "",
                Verdict::AboveThreshold("p95_ms".to_owned()),
            ),
        ];
        let counts = Counts::of(entries.iter());
        assert_eq!((counts.passed, counts.failed, counts.above), (1, 1, 1));
        let text = summary(&entries, "target/test-plan/r.md", &Redaction::default());
        assert!(text.contains("FALLITA T-A02: rotto"), "{text}");
        assert!(text.contains("sopra soglia P-01"), "{text}");
        assert!(text.ends_with("target/test-plan/r.md"), "{text}");
    }

    #[test]
    fn render_has_a_table_per_level_and_the_measure_values() {
        let environment = Environment {
            fields: vec![("Commit", "abc".to_owned())],
        };
        let mut measured = Entry::new(
            Level::Three,
            "P-06",
            "ordinamento",
            "cargo test",
            Verdict::Passed,
        );
        measured.measure = Some(MeasureLine {
            values: [
                ("samples", "75"),
                ("p50_ms", "12.5"),
                ("p95_ms", "20"),
                ("max_ms", "30"),
                ("mem_mb", "100.0"),
                ("mem_kind", "private"),
                ("batch_max_ms", "4.25"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
        });
        let entries = [
            Entry::new(
                Level::One,
                "L1-suite",
                "suite",
                "cargo test",
                Verdict::Passed,
            ),
            measured,
            Entry::new(
                Level::Four,
                "M-01",
                "demo",
                "",
                Verdict::Manual("a mano".to_owned()),
            ),
        ];
        let text = render(&Report {
            stamp: UtcStamp { seconds: 0 },
            invocation: "cargo xtask test-plan",
            environment: &environment,
            entries: &entries,
            total: Duration::from_secs(61),
            redaction: &Redaction::default(),
        });
        assert!(text.contains("- Commit: abc"), "{text}");
        assert!(text.contains("## Livello 1"), "{text}");
        assert!(text.contains("## Livello 3"), "{text}");
        assert!(text.contains("## Livello 4"), "{text}");
        assert!(!text.contains("## Livello 2"), "{text}");
        assert!(
            text.contains("| 75 | 12,5 | 20 | 30 | 100 MiB (privata) |"),
            "{text}"
        );
        assert!(text.contains("batch_max_ms = 4,25"), "{text}");
        assert!(text.contains("p95_ms < 50"), "soglia del piano: {text}");
    }
}
