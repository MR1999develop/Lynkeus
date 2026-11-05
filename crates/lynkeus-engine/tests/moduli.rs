// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Struttura del crate: nessun ciclo tra i moduli di `src/` (regola delle dipendenze in una
//! sola direzione, dentro il crate come tra i crate).
//!
//! Le dipendenze si leggono dal codice: ogni `crate::<modulo>` fuori dai commenti e prima dei
//! test del file (`#[cfg(test)]` a inizio riga) è un arco verso quel modulo.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// I moduli di `src/` (un file ciascuno) con i moduli che usano.
fn module_graph() -> BTreeMap<String, BTreeSet<String>> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let files: Vec<(String, String)> = std::fs::read_dir(&src)
        .expect("cartella src")
        .flatten()
        .filter_map(|item| {
            let path = item.path();
            let name = path.file_stem()?.to_str()?.to_owned();
            let is_rust = path.extension().is_some_and(|e| e == "rs");
            (is_rust && name != "lib").then(|| {
                let text = std::fs::read_to_string(&path).expect("sorgente leggibile");
                (name, text)
            })
        })
        .collect();
    let modules: BTreeSet<String> = files.iter().map(|(name, _)| name.clone()).collect();
    files
        .iter()
        .map(|(name, text)| {
            let used = code_lines(text)
                .flat_map(|line| crate_paths(line, &modules))
                .filter(|used| used != name)
                .collect();
            (name.clone(), used)
        })
        .collect()
}

/// Le righe di codice prima dei test, senza i commenti di riga.
fn code_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .take_while(|line| !line.starts_with("#[cfg(test)]"))
        .map(|line| line.split("//").next().unwrap_or(""))
}

/// I moduli nominati in `line` come `crate::<modulo>`, anche dentro `use crate::{a, b}`.
fn crate_paths(line: &str, modules: &BTreeSet<String>) -> Vec<String> {
    let mut found = Vec::new();
    for (at, _) in line.match_indices("crate::") {
        let rest = &line[at + "crate::".len()..];
        let names: Vec<&str> = match rest.strip_prefix('{') {
            Some(group) => group
                .split(['}', ','])
                .map(|part| part.trim().split("::").next().unwrap_or(""))
                .collect(),
            None => vec![
                rest.split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or(""),
            ],
        };
        found.extend(
            names
                .into_iter()
                .filter(|name| modules.contains(*name))
                .map(str::to_owned),
        );
    }
    found
}

/// Un ciclo del grafo, se c'è (visita in profondità con la pila dei moduli aperti).
fn find_cycle(graph: &BTreeMap<String, BTreeSet<String>>) -> Option<Vec<String>> {
    fn visit(
        node: &str,
        graph: &BTreeMap<String, BTreeSet<String>>,
        open: &mut Vec<String>,
        done: &mut BTreeSet<String>,
    ) -> Option<Vec<String>> {
        if let Some(at) = open.iter().position(|n| n == node) {
            let mut cycle = open[at..].to_vec();
            cycle.push(node.to_owned());
            return Some(cycle);
        }
        if done.contains(node) {
            return None;
        }
        open.push(node.to_owned());
        for next in graph.get(node).into_iter().flatten() {
            if let Some(cycle) = visit(next, graph, open, done) {
                return Some(cycle);
            }
        }
        open.pop();
        done.insert(node.to_owned());
        None
    }
    let mut done = BTreeSet::new();
    graph
        .keys()
        .find_map(|node| visit(node, graph, &mut Vec::new(), &mut done))
}

#[test]
fn modules_have_no_cycles() {
    let graph = module_graph();
    assert!(graph.len() > 5, "moduli letti: {graph:?}");
    assert_eq!(find_cycle(&graph), None, "grafo: {graph:#?}");
}

#[test]
fn the_cycle_finder_finds_cycles() {
    let graph: BTreeMap<String, BTreeSet<String>> = [
        ("a", vec!["b"]),
        ("b", vec!["c"]),
        ("c", vec!["a"]),
        ("d", vec![]),
    ]
    .into_iter()
    .map(|(n, e)| (n.to_owned(), e.into_iter().map(str::to_owned).collect()))
    .collect();
    assert_eq!(
        find_cycle(&graph),
        Some(vec!["a".into(), "b".into(), "c".into(), "a".into()])
    );
    let modules: BTreeSet<String> = ["worker", "event"].map(str::to_owned).into();
    assert_eq!(
        crate_paths("use crate::{worker::Stop, event::Hit, Notify};", &modules),
        ["worker", "event"]
    );
    assert_eq!(crate_paths("crate::worker::run(&x)", &modules), ["worker"]);
}
