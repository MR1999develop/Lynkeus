// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Quali cartelle visitare e quali file leggere, secondo i [`Filters`].
//!
//! Decisioni pure su fatti già letti ([`EntryFacts`]): la scansione ricava i fatti dai
//! metadati dell'elenco della cartella, senza aprire i file.

use std::ffi::OsStr;
use std::fs::Metadata;
use std::path::Path;

use lynkeus_core::ByteSize;

use crate::platform;
use crate::request::{Filters, HiddenPolicy, OnlineFilesPolicy};

/// Ciò che serve sapere di un file o di una cartella per filtrarlo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct EntryFacts {
    /// Nascosto o di sistema.
    pub hidden: bool,
    /// Solo online (leggerlo lo scaricherebbe).
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

/// Vero se la cartella `name` (sotto una radice, mai la radice stessa) va visitata.
pub(crate) fn enter_dir(name: &OsStr, facts: EntryFacts, filters: &Filters) -> bool {
    let skipped_by_name = filters
        .skip_dirs
        .iter()
        .any(|skip| platform::dir_name_matches(name, skip));
    let skipped_as_hidden = filters.hidden == HiddenPolicy::Skip && facts.hidden;
    !(skipped_by_name || skipped_as_hidden)
}

/// Decide se leggere il file `path`.
///
/// Una radice (`is_root`) scelta dall'utente non si esclude per attributi nascosto o di
/// sistema né per estensione; dimensione massima e stato solo online valgono anche per
/// lei, perché leggerla costerebbe lo stesso.
pub(crate) fn check_file(
    path: &Path,
    facts: EntryFacts,
    is_root: bool,
    filters: &Filters,
) -> Result<(), SkipReason> {
    if !is_root {
        if filters.hidden == HiddenPolicy::Skip && facts.hidden {
            return Err(SkipReason::Hidden);
        }
        if !filters.extensions.accepts(path) {
            return Err(SkipReason::Extension);
        }
    }
    if filters.online_only == OnlineFilesPolicy::Skip && facts.online_only {
        return Err(SkipReason::OnlineOnly);
    }
    if let (Some(len), Some(max)) = (facts.len, filters.max_file_size)
        && len > max.bytes()
    {
        return Err(SkipReason::TooLarge);
    }
    Ok(())
}

/// Dimensione massima di lettura: quella dei filtri, o nessun limite.
pub(crate) fn max_read_size(filters: &Filters) -> ByteSize {
    filters
        .max_file_size
        .unwrap_or(ByteSize::from_bytes(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{Extension, ExtensionFilter};

    fn facts(hidden: bool, online_only: bool, len: u64) -> EntryFacts {
        EntryFacts {
            hidden,
            online_only,
            len: Some(len),
        }
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
}
