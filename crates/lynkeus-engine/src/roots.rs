// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Radici della ricerca: esistenza, duplicati e radici annidate.
//!
//! Ogni file si legge una volta sola, anche se sta sotto più radici:
//!
//! - due radici con lo stesso percorso (anche scritto in modo diverso: relativo, con `.` o
//!   `..`, con altre maiuscole su Windows) sono una sola, quella che compare per prima;
//! - una radice dentro un'altra (cartella dentro cartella, file dentro cartella) resta una
//!   radice a sé: la radice esterna salta quel ramo e la radice interna lo visita con le
//!   sue regole (mai esclusa per nome, estensione o attributi).
//!
//! Il confronto è sul testo dei percorsi, senza risolvere i link: due percorsi diversi che
//! portano alla stessa cartella attraverso un link non si riconoscono.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::platform;

/// Una radice da visitare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedRoot {
    /// Indice in [`SearchRequest::roots`](crate::SearchRequest::roots).
    pub index: usize,
    /// Il percorso come l'ha dato l'utente: la scansione parte da qui.
    pub path: PathBuf,
    /// Forma assoluta e normalizzata di `path`.
    normalized: PathBuf,
    /// Vero se un'altra radice sta dentro questa.
    has_nested: bool,
}

/// Una radice che non si può visitare (inesistente, accesso negato).
#[derive(Debug)]
pub(crate) struct MissingRoot {
    /// Il percorso come l'ha dato l'utente.
    pub path: PathBuf,
    /// L'errore del sistema.
    pub error: io::Error,
}

/// Le radici da visitare, senza duplicati, e quelle che non si possono visitare.
#[derive(Debug)]
pub(crate) struct RootPlan {
    /// Radici esistenti, nell'ordine dato.
    pub index: RootIndex,
    /// Radici che non si possono visitare, nell'ordine dato.
    pub missing: Vec<MissingRoot>,
}

/// Prepara le radici: controlla che esistano (seguendo un eventuale link della radice
/// stessa, come fa la scansione) e toglie i duplicati.
pub(crate) fn plan(roots: &[PathBuf]) -> RootPlan {
    let mut candidates = Vec::with_capacity(roots.len());
    let mut missing = Vec::new();
    for (index, path) in roots.iter().enumerate() {
        match fs::metadata(path) {
            Ok(_) => {
                let normalized = platform::normalize(path).unwrap_or_else(|| path.clone());
                candidates.push((index, path.clone(), normalized));
            }
            Err(error) => missing.push(MissingRoot {
                path: path.clone(),
                error,
            }),
        }
    }
    RootPlan {
        index: RootIndex::new(candidates),
        missing,
    }
}

/// Radici da visitare, con le ricerche che servono durante la scansione.
#[derive(Debug, Default)]
pub(crate) struct RootIndex {
    roots: Vec<PlannedRoot>,
    /// Percorso dato → posizione in `roots`.
    by_path: HashMap<PathBuf, usize>,
    /// Chiavi di confronto di tutte le radici.
    keys: HashSet<PathBuf>,
}

impl RootIndex {
    /// Indice da radici esistenti: (indice nella richiesta, percorso dato, forma normalizzata).
    ///
    /// Costo O(n²) nel numero di radici per trovare quelle annidate: le radici le sceglie
    /// l'utente, quindi sono poche.
    fn new(candidates: Vec<(usize, PathBuf, PathBuf)>) -> RootIndex {
        let mut index = RootIndex::default();
        let mut key_list = Vec::new();
        for (request_index, path, normalized) in candidates {
            let key = platform::path_key(&normalized);
            if !index.keys.insert(key.clone()) {
                continue;
            }
            index.by_path.insert(path.clone(), index.roots.len());
            index.roots.push(PlannedRoot {
                index: request_index,
                path,
                normalized,
                has_nested: false,
            });
            key_list.push(key);
        }
        for (root, key) in index.roots.iter_mut().zip(&key_list) {
            root.has_nested = key_list
                .iter()
                .any(|other| other != key && other.starts_with(key));
        }
        index
    }

    /// Le radici da visitare, nell'ordine dato.
    pub fn roots(&self) -> &[PlannedRoot] {
        &self.roots
    }

    /// La radice da cui la scansione è arrivata a `path`, a profondità `depth`.
    ///
    /// La scansione costruisce i percorsi unendo i nomi alla radice, quindi la radice è
    /// `path` senza gli ultimi `depth` componenti.
    pub fn owner(&self, path: &Path, depth: usize) -> Option<&PlannedRoot> {
        let root_path = path.ancestors().nth(depth)?;
        self.by_path
            .get(root_path)
            .and_then(|&position| self.roots.get(position))
    }

    /// Vero se `path`, trovato sotto `owner`, è a sua volta una radice: la visita lei.
    pub fn is_other_root(&self, path: &Path, owner: &PlannedRoot) -> bool {
        if !owner.has_nested {
            return false;
        }
        match path.strip_prefix(&owner.path) {
            Ok(relative) if !relative.as_os_str().is_empty() => {
                let key = platform::path_key(&owner.normalized.join(relative));
                self.keys.contains(&key)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(paths: &[&str]) -> RootIndex {
        let base = std::path::absolute("radici").expect("cartella corrente");
        RootIndex::new(
            paths
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let path = base.join(p);
                    let normalized = platform::normalize(&path).expect("normalizzabile");
                    (i, path, normalized)
                })
                .collect(),
        )
    }

    fn indices(index: &RootIndex) -> Vec<usize> {
        index.roots().iter().map(|r| r.index).collect()
    }

    #[test]
    fn duplicates_keep_the_first() {
        let roots = index(&["a", "b", "a/.", "x/../a"]);
        assert_eq!(indices(&roots), [0, 1]);
        let case = index(&["Dati", "dati"]);
        assert_eq!(case.roots().len(), if cfg!(windows) { 1 } else { 2 });
    }

    #[test]
    fn nested_roots_are_visited_by_themselves() {
        let roots = index(&["a", "a/b", "c"]);
        assert_eq!(indices(&roots), [0, 1, 2]);
        let outer = &roots.roots()[0];
        let inner_path = outer.path.join("b");
        assert!(roots.is_other_root(&inner_path, outer));
        assert!(!roots.is_other_root(&outer.path.join("bb"), outer));
        assert!(!roots.is_other_root(&outer.path.join("b").join("x"), outer));
        let unrelated = &roots.roots()[2];
        assert!(!roots.is_other_root(&unrelated.path.join("b"), unrelated));
    }

    #[test]
    fn owner_is_found_from_the_depth() {
        let roots = index(&["a", "a/b"]);
        let outer = roots.roots()[0].path.clone();
        let file = outer.join("x").join("y.txt");
        assert_eq!(roots.owner(&file, 2).map(|r| r.index), Some(0));
        let inner_file = outer.join("b").join("z.txt");
        assert_eq!(roots.owner(&inner_file, 1).map(|r| r.index), Some(1));
        assert_eq!(roots.owner(&file, 9), None);
    }

    #[test]
    fn missing_roots_are_reported() {
        let plan = plan(&[PathBuf::from("non-esiste-davvero-12345")]);
        assert!(plan.index.roots().is_empty());
        assert_eq!(plan.missing.len(), 1);
        assert_eq!(plan.missing[0].error.kind(), io::ErrorKind::NotFound);
    }
}
