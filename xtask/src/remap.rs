// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! `--remap-path-prefix` per togliere i percorsi locali dai binari di rilascio.
//!
//! `trim-paths` non è stabile in cargo 1.99.1, quindi xtask passa a cargo
//! `--config target.<triple>.rustflags=[...]` con i percorsi calcolati al momento:
//! niente viene scritto su file né in `RUSTFLAGS`. Cargo unisce gli array della
//! configurazione, quindi i rustflags di `.cargo/config.toml` restano attivi.

use std::path::PathBuf;

/// Un percorso locale da riscrivere e il nome neutro che lo sostituisce nei binari.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remap {
    pub from: PathBuf,
    pub to: &'static str,
}

/// Le riscritture in ordine di applicazione per rustc: quando più prefissi
/// corrispondono vince l'ultimo, quindi i percorsi più generali vanno prima.
///
/// `home` è la cartella utente; `project`, `cargo_home` e `rustup_home` ci stanno di solito dentro.
pub fn remaps(
    home: Option<PathBuf>,
    cargo_home: PathBuf,
    rustup_home: PathBuf,
    project: PathBuf,
) -> Vec<Remap> {
    let mut list = Vec::with_capacity(4);
    if let Some(home) = home {
        list.push(Remap {
            from: home,
            to: "home",
        });
    }
    list.push(Remap {
        from: rustup_home,
        to: "rustup",
    });
    list.push(Remap {
        from: cargo_home,
        to: "cargo",
    });
    list.push(Remap {
        from: project,
        to: "lynkeus",
    });
    list
}

/// Il valore per `cargo --config`: `target.<triple>.rustflags=[...]` con un
/// `--remap-path-prefix=<da>=<a>` per ogni riscrittura.
pub fn config_arg(triple: &str, remaps: &[Remap]) -> String {
    let flags: Vec<String> = remaps
        .iter()
        .map(|remap| {
            toml_basic_string(&format!(
                "--remap-path-prefix={}={}",
                remap.from.display(),
                remap.to
            ))
        })
        .collect();
    format!("target.{triple}.rustflags=[{}]", flags.join(", "))
}

/// Stringa TOML di base (tra virgolette) con i caratteri speciali escapati.
fn toml_basic_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_string_escapes_backslash_quote_and_controls() {
        assert_eq!(toml_basic_string(r#"C:\a "b""#), r#""C:\\a \"b\"""#);
        assert_eq!(toml_basic_string("a\tb"), r#""a\u0009b""#);
    }

    #[test]
    fn remaps_put_general_prefixes_first_and_project_last() {
        let list = remaps(
            Some(PathBuf::from("/h")),
            PathBuf::from("/h/.cargo"),
            PathBuf::from("/h/.rustup"),
            PathBuf::from("/h/src/p"),
        );
        let targets: Vec<&str> = list.iter().map(|r| r.to).collect();
        assert_eq!(targets, ["home", "rustup", "cargo", "lynkeus"]);
    }

    #[test]
    fn remaps_without_home_keep_the_others() {
        let list = remaps(
            None,
            PathBuf::from("/c"),
            PathBuf::from("/r"),
            PathBuf::from("/p"),
        );
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn config_arg_is_a_toml_array_of_remap_flags() {
        let list = [Remap {
            from: PathBuf::from("/p"),
            to: "lynkeus",
        }];
        assert_eq!(
            config_arg("x86_64-unknown-linux-gnu", &list),
            r#"target.x86_64-unknown-linux-gnu.rustflags=["--remap-path-prefix=/p=lynkeus"]"#
        );
    }
}
