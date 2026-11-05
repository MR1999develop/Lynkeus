// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Argomenti della finestra (dopo `--extract-worker`, che il `main` riconosce prima).
//!
//! - `--add <percorso>`, ripetibile: il valore successivo è sempre un percorso, anche se
//!   inizia con `-` (un file chiamato `-r` o `--add` resta un file): così lo passa Explorer
//!   (`"<exe>" --add "%1"`).
//! - `--add -- <percorsi…>`: tutto ciò che segue è un percorso (script di Nautilus,
//!   `--add -- "$@"`).
//!
//! I percorsi qui sono solo raccolti; li valida il guardiano unico
//! [`crate::instance::accept_path`] dopo averli resi assoluti.

use std::ffi::OsString;
use std::path::PathBuf;

/// L'opzione che aggiunge percorsi.
pub const ADD_FLAG: &str = "--add";
/// Fine delle opzioni.
const END_OF_OPTIONS: &str = "--";

/// Uso scorretto della riga di comando (codice d'uscita 2).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UsageError {
    /// `--add` senza valore.
    #[error("--add senza percorso")]
    MissingValue,
    /// Un argomento sconosciuto (non ripetuto: può essere un percorso dell'utente).
    #[error("argomento non riconosciuto")]
    Unknown,
}

/// I percorsi passati con `--add`, nell'ordine; vuoto = avvio normale.
pub fn parse_added_paths(
    args: impl IntoIterator<Item = OsString>,
) -> Result<Vec<PathBuf>, UsageError> {
    let mut args = args.into_iter();
    let mut paths = Vec::new();
    while let Some(arg) = args.next() {
        if arg != ADD_FLAG {
            return Err(UsageError::Unknown);
        }
        let value = args.next().ok_or(UsageError::MissingValue)?;
        if value == END_OF_OPTIONS {
            paths.extend(args.by_ref().map(PathBuf::from));
            if paths.is_empty() {
                return Err(UsageError::MissingValue);
            }
        } else {
            paths.push(PathBuf::from(value));
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Vec<PathBuf>, UsageError> {
        parse_added_paths(args.iter().map(OsString::from))
    }

    fn paths(items: &[&str]) -> Vec<PathBuf> {
        items.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn no_arguments_is_a_normal_start() {
        assert_eq!(parse(&[]), Ok(vec![]));
    }

    /// Il valore dopo `--add` è sempre un percorso, qualunque cosa contenga.
    #[test]
    fn values_are_data() {
        let long = "x".repeat(32_000);
        for value in [
            "-r",
            "--add",
            "--",
            "a b",
            "x & calc",
            "100%",
            "^caret",
            "punto;virgola",
            "città è",
            "",
            long.as_str(),
        ] {
            let expected = if value == "--" {
                Err(UsageError::MissingValue)
            } else {
                Ok(paths(&[value]))
            };
            assert_eq!(parse(&["--add", value]), expected, "{value}");
        }
    }

    #[test]
    fn several_adds_and_end_of_options() {
        assert_eq!(
            parse(&["--add", "a", "--add", "-b"]),
            Ok(paths(&["a", "-b"]))
        );
        assert_eq!(
            parse(&["--add", "--", "a'b", "-r", "--add", "$(x)"]),
            Ok(paths(&["a'b", "-r", "--add", "$(x)"]))
        );
    }

    #[test]
    fn usage_errors() {
        assert_eq!(parse(&["--add"]), Err(UsageError::MissingValue));
        assert_eq!(parse(&["--add", "--"]), Err(UsageError::MissingValue));
        assert_eq!(parse(&["cartella"]), Err(UsageError::Unknown));
        assert_eq!(
            parse(&["--add", "a", "--verbose"]),
            Err(UsageError::Unknown)
        );
    }
}
