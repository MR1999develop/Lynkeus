// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Quali cartelle visitare e quali file leggere, secondo i [`Filters`].
//!
//! [`FileFilter`] prepara i filtri una volta per ricerca (estensioni e nomi delle cartelle
//! già nella forma del confronto), così il ciclo sulle voci della scansione non alloca per i
//! nomi ASCII. Le decisioni sono pure, su fatti già letti ([`EntryFacts`]): la scansione
//! ricava i fatti dai metadati dell'elenco della cartella, senza aprire i file.

use std::ffi::OsStr;
use std::fs::Metadata;
use std::path::Path;

use lynkeus_core::ByteSize;

use crate::platform;
use crate::request::{Extension, ExtensionFilter, Filters, HiddenPolicy, OnlineFilesPolicy};

/// Ciò che serve sapere di un file o di una cartella per filtrarlo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct EntryFacts {
    /// Nascosto o di sistema.
    pub hidden: bool,
    /// Solo online (leggerlo, o elencarlo se è una cartella, lo scaricherebbe).
    pub online_only: bool,
    /// Dimensione in byte, se nota.
    pub len: Option<u64>,
}

impl EntryFacts {
    /// Fatti dal nome e dai metadati (non seguiti: un link è un link).
    pub fn read(name: &OsStr, metadata: Option<&Metadata>) -> EntryFacts {
        EntryFacts {
            hidden: platform::is_hidden(name, metadata),
            online_only: metadata.is_some_and(platform::is_online_only),
            len: metadata.map(Metadata::len),
        }
    }
}

/// Perché un file non si legge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkipReason {
    /// Nascosto o di sistema.
    Hidden,
    /// Escluso dal filtro sulle estensioni.
    Extension,
    /// Solo online.
    OnlineOnly,
    /// Più grande della dimensione massima.
    TooLarge,
}

/// Le estensioni del filtro, in minuscolo come [`Extension`](crate::Extension).
#[derive(Debug, Clone)]
enum Extensions {
    All,
    Include(Vec<String>),
    Exclude(Vec<String>),
}

/// I [`Filters`] di una ricerca, preparati per il confronto.
#[derive(Debug, Clone)]
pub(crate) struct FileFilter {
    extensions: Extensions,
    /// Nomi delle cartelle da saltare, nella forma di [`platform::dir_name_key`].
    skip_dirs: Vec<String>,
    hidden: HiddenPolicy,
    online_only: OnlineFilesPolicy,
    max_file_size: Option<ByteSize>,
}

impl FileFilter {
    /// Prepara `filters`.
    pub(crate) fn new(filters: &Filters) -> FileFilter {
        let lowercase = |list: &[Extension]| -> Vec<String> {
            list.iter().map(|e| e.as_str().to_owned()).collect()
        };
        FileFilter {
            extensions: match &filters.extensions {
                ExtensionFilter::All => Extensions::All,
                ExtensionFilter::Include(list) => Extensions::Include(lowercase(list)),
                ExtensionFilter::Exclude(list) => Extensions::Exclude(lowercase(list)),
            },
            skip_dirs: filters
                .skip_dirs
                .iter()
                .map(|skip| platform::dir_name_key(skip))
                .collect(),
            hidden: filters.hidden,
            online_only: filters.online_only,
            max_file_size: filters.max_file_size,
        }
    }

    /// Vero se la cartella `name` (sotto una radice, mai la radice stessa) va visitata: non
    /// è tra quelle da saltare, né nascosta, né solo online (elencarla scaricherebbe
    /// l'elenco), salvo che i filtri li ammettano.
    pub(crate) fn enter_dir(&self, name: &OsStr, facts: EntryFacts) -> bool {
        let skipped_by_name = self
            .skip_dirs
            .iter()
            .any(|key| platform::dir_name_is(name, key));
        let skipped_as_hidden = self.hidden == HiddenPolicy::Skip && facts.hidden;
        let skipped_as_online = self.online_only == OnlineFilesPolicy::Skip && facts.online_only;
        !(skipped_by_name || skipped_as_hidden || skipped_as_online)
    }

    /// Decide se leggere il file `path`.
    ///
    /// Una radice (`is_root`) scelta dall'utente non si esclude per attributi nascosto o di
    /// sistema né per estensione; dimensione massima e stato solo online valgono anche per
    /// lei, perché leggerla costerebbe lo stesso.
    pub(crate) fn check_file(
        &self,
        path: &Path,
        facts: EntryFacts,
        is_root: bool,
    ) -> Result<(), SkipReason> {
        if !is_root {
            if self.hidden == HiddenPolicy::Skip && facts.hidden {
                return Err(SkipReason::Hidden);
            }
            if !self.accepts_extension(path) {
                return Err(SkipReason::Extension);
            }
        }
        if self.online_only == OnlineFilesPolicy::Skip && facts.online_only {
            return Err(SkipReason::OnlineOnly);
        }
        if let (Some(len), Some(max)) = (facts.len, self.max_file_size)
            && len > max.bytes()
        {
            return Err(SkipReason::TooLarge);
        }
        Ok(())
    }

    /// Dimensione massima di lettura: quella dei filtri, o nessun limite.
    pub(crate) fn max_read_size(&self) -> ByteSize {
        self.max_file_size.unwrap_or(ByteSize::from_bytes(u64::MAX))
    }

    fn accepts_extension(&self, path: &Path) -> bool {
        let listed = |list: &[String]| list.iter().any(|lower| platform::extension_is(path, lower));
        match &self.extensions {
            Extensions::All => true,
            Extensions::Include(list) => listed(list),
            Extensions::Exclude(list) => !listed(list),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(hidden: bool, online_only: bool, len: u64) -> EntryFacts {
        EntryFacts {
            hidden,
            online_only,
            len: Some(len),
        }
    }

    fn enter_dir(name: &OsStr, facts: EntryFacts, filters: &Filters) -> bool {
        FileFilter::new(filters).enter_dir(name, facts)
    }

    fn check_file(
        path: &Path,
        facts: EntryFacts,
        is_root: bool,
        filters: &Filters,
    ) -> Result<(), SkipReason> {
        FileFilter::new(filters).check_file(path, facts, is_root)
    }

    #[test]
    fn dirs_skipped_by_name_or_when_hidden() {
        let filters = Filters::default();
        let plain = EntryFacts::default();
        assert!(enter_dir(OsStr::new("src"), plain, &filters));
        assert!(!enter_dir(OsStr::new("node_modules"), plain, &filters));
        assert!(!enter_dir(
            OsStr::new("src"),
            facts(true, false, 0),
            &filters
        ));
        let include_hidden = Filters {
            hidden: HiddenPolicy::Include,
            ..Filters::default()
        };
        assert!(enter_dir(
            OsStr::new("src"),
            facts(true, false, 0),
            &include_hidden
        ));
        assert!(!enter_dir(OsStr::new(".git"), plain, &include_hidden));
    }

    /// Una cartella solo online (OneDrive, attributi `RECALL_ON_*`): elencarla scatenerebbe
    /// il download dell'elenco, quindi con "salta i file solo online" non si visita.
    #[test]
    fn online_only_dirs_are_entered_only_when_included() {
        let online = facts(false, true, 0);
        assert!(!enter_dir(
            OsStr::new("Documenti"),
            online,
            &Filters::default()
        ));
        let include_online = Filters {
            online_only: OnlineFilesPolicy::Include,
            ..Filters::default()
        };
        assert!(enter_dir(OsStr::new("Documenti"), online, &include_online));
    }

    #[test]
    fn file_partitions() {
        let txt = Extension::parse("txt").expect("valida");
        let filters = Filters {
            extensions: ExtensionFilter::Include(vec![txt]),
            max_file_size: Some(ByteSize::from_bytes(10)),
            ..Filters::default()
        };
        let path = Path::new("a.txt");
        let plain = facts(false, false, 10);
        assert_eq!(check_file(path, plain, false, &filters), Ok(()));
        assert_eq!(
            check_file(path, facts(false, false, 11), false, &filters),
            Err(SkipReason::TooLarge)
        );
        assert_eq!(
            check_file(Path::new("a.md"), plain, false, &filters),
            Err(SkipReason::Extension)
        );
        assert_eq!(
            check_file(path, facts(true, false, 1), false, &filters),
            Err(SkipReason::Hidden)
        );
        assert_eq!(
            check_file(path, facts(false, true, 1), false, &filters),
            Err(SkipReason::OnlineOnly)
        );
    }

    #[test]
    fn roots_ignore_name_filters_but_not_size_or_online_state() {
        let filters = Filters {
            extensions: ExtensionFilter::Exclude(vec![Extension::parse("md").expect("valida")]),
            max_file_size: Some(ByteSize::from_bytes(10)),
            ..Filters::default()
        };
        let md = Path::new(".nota.md");
        assert_eq!(
            check_file(md, facts(true, false, 1), true, &filters),
            Ok(())
        );
        assert_eq!(
            check_file(md, facts(false, false, 11), true, &filters),
            Err(SkipReason::TooLarge)
        );
        assert_eq!(
            check_file(md, facts(false, true, 1), true, &filters),
            Err(SkipReason::OnlineOnly)
        );
        let include_online = Filters {
            online_only: OnlineFilesPolicy::Include,
            max_file_size: None,
            ..Filters::default()
        };
        assert_eq!(
            check_file(md, facts(false, true, u64::MAX), true, &include_online),
            Ok(())
        );
    }

    /// L'estensione come la confrontava la versione senza filtro preparato: oracolo.
    fn naive_extension_is(path: &Path, lower: &str) -> bool {
        path.extension()
            .is_some_and(|ext| ext.to_string_lossy().to_lowercase() == lower)
    }

    proptest::proptest! {
        /// Il filtro preparato decide come il confronto con `to_lowercase` per ogni nome.
        #[test]
        fn prepared_extensions_match_the_naive_comparison(
            stem in "[a-zA-Z0-9]{0,6}",
            ext in "[a-zA-ZÀàΣσςİẞß]{0,5}",
            listed in proptest::collection::vec("[a-zA-ZÀàΣσςİẞß]{1,5}", 0..6),
        ) {
            let name = format!("{stem}.{ext}");
            let path = Path::new(&name);
            let list: Vec<Extension> = listed.iter().filter_map(|e| Extension::parse(e).ok()).collect();
            let naive = list.iter().any(|e| naive_extension_is(path, e.as_str()));
            let include = Filters {
                extensions: ExtensionFilter::Include(list.clone()),
                ..Filters::default()
            };
            proptest::prop_assert_eq!(FileFilter::new(&include).accepts_extension(path), naive);
            proptest::prop_assert_eq!(include.extensions.accepts(path), naive);
            let exclude = Filters {
                extensions: ExtensionFilter::Exclude(list),
                ..Filters::default()
            };
            proptest::prop_assert_eq!(FileFilter::new(&exclude).accepts_extension(path), !naive);
        }
    }

    /// Misura P-04 (piano di test): 32 estensioni su 100.000 nomi, filtro preparato contro
    /// il confronto con `to_lowercase` per voce. `cargo test --release -p lynkeus-engine
    /// --lib prepared_filter_timing -- --ignored --nocapture`.
    #[test]
    #[ignore = "misura di prestazioni (P-04), da lanciare a mano in release"]
    fn prepared_filter_timing() {
        use std::hint::black_box;
        use std::time::{Duration, Instant};

        let list: Vec<Extension> = (0..32)
            .map(|n| Extension::parse(&format!("e{n:02}x")).expect("valida"))
            .collect();
        let names: Vec<String> = (0..100_000)
            .map(|n| format!("file{n}.E{:02}X", n % 40))
            .collect();
        let filters = Filters {
            extensions: ExtensionFilter::Include(list.clone()),
            ..Filters::default()
        };
        let prepared = FileFilter::new(&filters);
        let measure = |run: &dyn Fn() -> usize| -> Vec<Duration> {
            let mut times: Vec<Duration> = (0..16)
                .map(|_| {
                    let started = Instant::now();
                    black_box(run());
                    started.elapsed()
                })
                .skip(1)
                .collect();
            times.sort_unstable();
            times
        };
        let naive = measure(&|| {
            names
                .iter()
                .filter(|n| {
                    let path = Path::new(black_box(n.as_str()));
                    list.iter().any(|e| naive_extension_is(path, e.as_str()))
                })
                .count()
        });
        let fast = measure(&|| {
            names
                .iter()
                .filter(|n| prepared.accepts_extension(Path::new(black_box(n.as_str()))))
                .count()
        });
        let report = |label: &str, t: &[Duration]| {
            println!(
                "P-04 {label}: p50 {:?}, p95 {:?}, massimo {:?} (15 ripetizioni)",
                t[t.len() / 2],
                t[(t.len() - 1) * 95 / 100],
                t[t.len() - 1]
            );
        };
        report("confronto con to_lowercase", &naive);
        report("filtro preparato", &fast);
    }
}
