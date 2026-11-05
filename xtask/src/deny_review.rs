// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! `cargo xtask deny-review`: le eccezioni di `deny.toml` scadono.
//!
//! Ogni eccezione (advisory ignorato, crate saltato da `bans`) ha nel motivo la frase
//! "rivedere entro AAAA-MM-GG". Il controllo fallisce se un'eccezione non ha motivo o data,
//! se la data non è valida o se è passata: un'eccezione scaduta si rivede (si toglie, o si
//! rinnova con un motivo aggiornato) invece di restare per sempre. Gira dentro
//! `cargo xtask check` e nel controllo giornaliero della CI.
//!
//! Le eccezioni di licenza (`licenses.exceptions`) non hanno un campo `reason` in
//! cargo-deny e non scadono: una licenza compatibile resta compatibile.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use crate::process::workspace_root;

/// Il file controllato, nella radice del workspace.
const DENY_FILE: &str = "deny.toml";
/// La frase che precede la data di revisione nel motivo.
const REVIEW_MARKER: &str = "rivedere entro ";
/// Le liste di eccezioni: (tabella, chiave).
const EXCEPTION_LISTS: [(&str, &str); 3] = [
    ("advisories", "ignore"),
    ("bans", "skip"),
    ("bans", "skip-tree"),
];

/// Una data del calendario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    year: i32,
    month: u32,
    day: u32,
}

impl Date {
    /// Una data valida `AAAA-MM-GG` (anni dal 2000 al 9999).
    pub fn parse(text: &str) -> Option<Date> {
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
            return None;
        }
        let number = |range: std::ops::Range<usize>| -> Option<u32> {
            let part = text.get(range)?;
            part.bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| part.parse().ok())?
        };
        let date = Date {
            year: i32::try_from(number(0..4)?).ok()?,
            month: number(5..7)?,
            day: number(8..10)?,
        };
        let valid = date.year >= 2000
            && (1..=12).contains(&date.month)
            && date.day >= 1
            && date.day <= days_in_month(date.year, date.month);
        valid.then_some(date)
    }

    /// La data di oggi (UTC) dall'orologio di sistema.
    pub fn today() -> Result<Date> {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("orologio di sistema prima del 1970")?
            .as_secs();
        let days = i64::try_from(seconds / 86_400).context("data fuori intervallo")?;
        Ok(civil_from_days(days))
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// La data di `days` giorni dopo il 1970-01-01 (algoritmo "civil from days" di
/// H. Hinnant, calendario gregoriano prolettico).
fn civil_from_days(days: i64) -> Date {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    Date {
        year: i32::try_from(year).unwrap_or(i32::MAX),
        month: u32::try_from(month).unwrap_or(1),
        day: u32::try_from(day).unwrap_or(1),
    }
}

/// Un'eccezione con la sua data di revisione.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exception {
    /// Dove sta e che cosa esclude (`advisories.ignore RUSTSEC-…`).
    pub what: String,
    /// La data entro cui rivederla.
    pub review_by: Date,
}

/// Le eccezioni di `text` (contenuto di `deny.toml`) e i problemi trovati rispetto a
/// `today`: senza motivo, senza data, data non valida o passata. La data stessa vale
/// ancora; il giorno dopo no.
pub fn review(text: &str, today: Date) -> Result<(Vec<Exception>, Vec<String>)> {
    let table: toml::Table = toml::from_str(text).context("deny.toml non è TOML valido")?;
    let mut exceptions = Vec::new();
    let mut problems = Vec::new();
    for (section, key) in EXCEPTION_LISTS {
        let Some(list) = table.get(section).and_then(|s| s.get(key)) else {
            continue;
        };
        let Some(items) = list.as_array() else {
            problems.push(format!("{section}.{key} non è un elenco"));
            continue;
        };
        for item in items {
            let what = format!("{section}.{key} {}", describe(item));
            match review_date(item) {
                Ok(review_by) if review_by < today => problems.push(format!(
                    "{what}: revisione scaduta il {review_by} (oggi {today})"
                )),
                Ok(review_by) => exceptions.push(Exception { what, review_by }),
                Err(reason) => problems.push(format!("{what}: {reason}")),
            }
        }
    }
    Ok((exceptions, problems))
}

/// Il nome dell'eccezione nei messaggi: id dell'advisory, crate o il testo.
fn describe(item: &toml::Value) -> String {
    ["id", "crate", "name"]
        .iter()
        .find_map(|key| item.get(key).and_then(toml::Value::as_str))
        .or_else(|| item.as_str())
        .unwrap_or("(senza nome)")
        .to_owned()
}

/// La data di revisione nel motivo di `item`.
fn review_date(item: &toml::Value) -> Result<Date, &'static str> {
    let reason = item
        .get("reason")
        .and_then(toml::Value::as_str)
        .ok_or("eccezione senza motivo (`reason`)")?;
    let (_, after) = reason
        .split_once(REVIEW_MARKER)
        .ok_or("motivo senza «rivedere entro AAAA-MM-GG»")?;
    after
        .get(..10)
        .and_then(Date::parse)
        .ok_or("data di revisione non valida (AAAA-MM-GG)")
}

/// Controlla `deny.toml` nella radice del workspace rispetto alla data di oggi.
pub fn run() -> Result<()> {
    let root = workspace_root()?;
    let path = root.join(DENY_FILE);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("lettura di {}", path.display()))?;
    let today = Date::today()?;
    let (exceptions, problems) = review(&text, today)?;
    if !problems.is_empty() {
        bail!(
            "eccezioni di {DENY_FILE} da rivedere: {}",
            problems.join("; ")
        );
    }
    let next = exceptions.iter().map(|e| e.review_by).min();
    match next {
        Some(next) => eprintln!(
            "xtask: {} eccezioni di {DENY_FILE} valide; prossima revisione entro il {next}",
            exceptions.len()
        ),
        None => eprintln!("xtask: nessuna eccezione in {DENY_FILE}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> Date {
        Date::parse(text).expect("data valida")
    }

    const ONE: &str = r#"
[advisories]
ignore = [
    { id = "RUSTSEC-2026-0001", reason = "non mantenuto; rivedere entro 2027-05-01" },
]
"#;

    /// T-D12: un'eccezione vale fino alla sua data compresa e fallisce il giorno dopo.
    #[test]
    fn deny_exceptions_expire() {
        for (today, expired) in [
            ("2026-11-05", false),
            ("2027-04-30", false),
            ("2027-05-01", false),
            ("2027-05-02", true),
            ("2030-01-01", true),
        ] {
            let (valid, problems) = review(ONE, date(today)).expect("letto");
            assert_eq!(
                problems.len(),
                usize::from(expired),
                "{today}: {problems:?}"
            );
            assert_eq!(valid.len(), usize::from(!expired), "{today}");
        }
        let (_, problems) = review(ONE, date("2027-05-02")).expect("letto");
        assert!(problems[0].contains("RUSTSEC-2026-0001"), "{problems:?}");
    }

    /// Eccezioni senza motivo, senza data o con una data impossibile: tutte segnalate, in
    /// ogni elenco.
    #[test]
    fn exceptions_without_a_valid_date_are_problems() {
        let text = r#"
[advisories]
ignore = [
    "RUSTSEC-2026-0002",
    { id = "RUSTSEC-2026-0003" },
    { id = "RUSTSEC-2026-0004", reason = "senza data" },
    { id = "RUSTSEC-2026-0005", reason = "rivedere entro 2027-02-30" },
    { id = "RUSTSEC-2026-0006", reason = "rivedere entro 27-5-1" },
]
[bans]
skip = [{ crate = "a", reason = "doppia versione" }]
skip-tree = [
    { crate = "b", reason = "rivedere entro 2027-05-01" },
    { crate = "c", reason = "rivedere entro 2026-01-01" },
]
"#;
        let (valid, problems) = review(text, date("2026-11-05")).expect("letto");
        assert_eq!(valid.len(), 1, "{valid:?}");
        assert_eq!(problems.len(), 7, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("bans.skip a")));
        assert!(problems.iter().any(|p| p.contains("scaduta il 2026-01-01")));
    }

    #[test]
    fn no_exceptions_is_fine() {
        let (valid, problems) =
            review("[advisories]\nignore = []\n", date("2026-11-05")).expect("letto");
        assert!(valid.is_empty() && problems.is_empty());
        assert!(review("non [ toml", date("2026-11-05")).is_err());
    }

    /// Il `deny.toml` del progetto: ogni eccezione ha una data di revisione valida.
    #[test]
    fn the_project_deny_toml_has_review_dates() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(DENY_FILE);
        let text = std::fs::read_to_string(path).expect("deny.toml");
        let (valid, problems) = review(&text, date("2026-11-05")).expect("letto");
        assert!(problems.is_empty(), "{problems:?}");
        assert!(!valid.is_empty());
    }

    #[test]
    fn date_partitions() {
        assert_eq!(date("2028-02-29").to_string(), "2028-02-29");
        for bad in [
            "2027-02-29",
            "2100-02-29",
            "2027-00-10",
            "2027-04-31",
            "1999-12-31",
            "2027-5-01",
            "2027/05/01",
            "+027-05-01",
            "",
        ] {
            assert_eq!(Date::parse(bad), None, "{bad}");
        }
        assert!(date("2000-02-29") < date("2000-03-01"));
    }

    /// Il calcolo della data dai giorni: confini noti e andata e ritorno su ogni giorno di
    /// un intervallo che comprende anni bisestili e secolari.
    #[test]
    fn civil_dates_from_days() {
        assert_eq!(civil_from_days(0).to_string(), "1970-01-01");
        assert_eq!(civil_from_days(20_397).to_string(), "2025-11-05");
        assert_eq!(civil_from_days(11_016).to_string(), "2000-02-29");
        // Dal 2000-01-01 (giorno 10.957) al 2134.
        let mut previous = civil_from_days(10_957);
        for days in 10_958..60_000 {
            let current = civil_from_days(days);
            assert!(current > previous, "{days}");
            assert_eq!(Date::parse(&current.to_string()), Some(current), "{days}");
            previous = current;
        }
    }
}
