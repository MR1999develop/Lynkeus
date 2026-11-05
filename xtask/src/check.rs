// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! `cargo xtask check` e `cargo xtask check-linux`.

use anyhow::{Context, Result};

use crate::nospawn;
use crate::process::{cargo, run_inherited, workspace_root};

/// Un passo del controllo: nome mostrato e argomenti di cargo.
struct Step {
    name: &'static str,
    args: &'static [&'static str],
}

/// I controlli, nell'ordine: il primo che fallisce ferma tutto.
const CHECK_STEPS: &[Step] = &[
    Step {
        name: "fmt",
        args: &["fmt", "--all", "--", "--check"],
    },
    Step {
        name: "clippy",
        args: &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    },
    Step {
        name: "test",
        args: &["test", "--workspace", "--locked"],
    },
    Step {
        name: "deny",
        args: &["deny", "check"],
    },
];

/// Controllo di compilazione per Linux da Windows: solo librerie e binari. Test e
/// benchmark compilerebbero anche le dev-dependency, e `criterion` → `alloca` richiede
/// un compilatore C per Linux; la CI su Ubuntu compila tutto con `--all-targets`.
const LINUX_STEP: Step = Step {
    name: "check-linux",
    args: &[
        "check",
        "--workspace",
        "--lib",
        "--bins",
        "--locked",
        "--target",
        "x86_64-unknown-linux-gnu",
    ],
};

/// Esegue il controllo "nessun avvio di processi nei decoder", poi fmt, clippy, test e
/// cargo-deny; si ferma al primo passo che fallisce.
pub fn run_all() -> Result<()> {
    let root = workspace_root()?;
    eprintln!("xtask: ── nospawn ── {}", nospawn::CHECKED_DIR);
    nospawn::run(&root).context("passo «nospawn» fallito")?;
    for step in CHECK_STEPS {
        run_step(step, &root).with_context(|| format!("passo «{}» fallito", step.name))?;
    }
    eprintln!("xtask: tutti i controlli sono passati");
    Ok(())
}

/// Controllo di compilazione dell'intero workspace per Linux x86_64.
pub fn run_linux() -> Result<()> {
    let root = workspace_root()?;
    run_step(&LINUX_STEP, &root).with_context(|| format!("passo «{}» fallito", LINUX_STEP.name))
}

fn run_step(step: &Step, root: &std::path::Path) -> Result<()> {
    eprintln!("xtask: ── {} ── cargo {}", step.name, step.args.join(" "));
    run_inherited(
        cargo().args(step.args).current_dir(root),
        &format!("cargo {}", step.name),
    )
}
