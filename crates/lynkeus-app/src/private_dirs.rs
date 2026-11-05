// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Preparazione delle cartelle private all'avvio (docs/threat-model.md, 4.8 C).
//!
//! Ogni cartella si crea privata o, se esiste, si controlla: una cartella di un altro
//! proprietario o aperta ad altri non si usa per questa sessione (impostazioni solo in
//! memoria, cache, log, rapporti o apertura delle voci d'archivio spenti) e l'utente lo
//! vede in un messaggio. Nessuna cartella viene "riparata" in silenzio.

use std::path::{Path, PathBuf};

use lynkeus_engine::{NotPrivate, PrivateFs};

use crate::platform::AppDirs;

/// Le cartelle utilizzabili in questa sessione: `None` = non disponibile.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreparedDirs {
    /// Configurazione (`settings.toml`).
    pub config: Option<PathBuf>,
    /// Dati locali (istanza unica).
    pub data: Option<PathBuf>,
    /// Cache dei documenti.
    pub cache: Option<PathBuf>,
    /// Log.
    pub logs: Option<PathBuf>,
    /// Rapporti di crash.
    pub crash_reports: Option<PathBuf>,
    /// File estratti dagli archivi per aprirli.
    pub opened_entries: Option<PathBuf>,
    /// Le cartelle rifiutate perché non private (da mostrare all'utente).
    pub refused: Vec<PathBuf>,
}

/// Crea o controlla ogni cartella di `dirs` con `private_fs`.
pub fn prepare(dirs: &AppDirs, private_fs: &dyn PrivateFs) -> PreparedDirs {
    let mut prepared = PreparedDirs::default();
    let mut ready = |dir: &Path| -> Option<PathBuf> {
        match private_fs.create_dir(dir) {
            Ok(()) => Some(dir.to_owned()),
            Err(error) if NotPrivate::is(&error) => {
                prepared.refused.push(dir.to_owned());
                None
            }
            Err(error) => {
                tracing::warn!(%error, dir = %dir.display(), "cartella privata non creata");
                None
            }
        }
    };
    let config = ready(&dirs.config);
    let data = ready(&dirs.data);
    // Le sottocartelle di una cartella dati rifiutata non si usano.
    let inside_data = |dir: &Path, ready: &mut dyn FnMut(&Path) -> Option<PathBuf>| {
        data.as_ref().and_then(|_| ready(dir))
    };
    let logs = inside_data(&dirs.logs, &mut ready);
    let crash_reports = inside_data(&dirs.crash_reports, &mut ready);
    let opened_entries = inside_data(&dirs.opened_entries, &mut ready);
    let cache = ready(&dirs.cache);
    PreparedDirs {
        config,
        data,
        cache,
        logs,
        crash_reports,
        opened_entries,
        refused: prepared.refused,
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io;
    use std::sync::Mutex;

    use super::*;

    /// Cartelle finte: rifiuta quelle che finiscono con "aperta".
    #[derive(Debug, Default)]
    struct FakeFs {
        created: Mutex<Vec<PathBuf>>,
    }

    impl PrivateFs for FakeFs {
        fn create_dir(&self, dir: &Path) -> io::Result<()> {
            if dir.ends_with("aperta") {
                return Err(NotPrivate::error());
            }
            if dir.ends_with("guasta") {
                return Err(io::Error::other("disco"));
            }
            if let Ok(mut created) = self.created.lock() {
                created.push(dir.to_owned());
            }
            Ok(())
        }

        fn create_new_file(&self, _path: &Path) -> io::Result<File> {
            Err(io::Error::other("non usato"))
        }
    }

    fn dirs(data: &str, cache: &str) -> AppDirs {
        AppDirs {
            config: PathBuf::from("/c/config"),
            data: PathBuf::from(data),
            cache: PathBuf::from(cache),
            logs: Path::new(data).join("logs"),
            crash_reports: Path::new(data).join("crash-reports"),
            opened_entries: Path::new(data).join("temp"),
        }
    }

    #[test]
    fn all_private() {
        let prepared = prepare(&dirs("/d/data", "/d/cache"), &FakeFs::default());
        assert!(prepared.refused.is_empty());
        assert_eq!(prepared.cache, Some(PathBuf::from("/d/cache")));
        assert_eq!(prepared.logs, Some(PathBuf::from("/d/data/logs")));
    }

    #[test]
    fn a_refused_cache_turns_off_only_the_cache() {
        let prepared = prepare(&dirs("/d/data", "/d/aperta"), &FakeFs::default());
        assert_eq!(prepared.cache, None);
        assert_eq!(prepared.refused, [PathBuf::from("/d/aperta")]);
        assert!(prepared.logs.is_some() && prepared.config.is_some());
    }

    #[test]
    fn a_refused_data_dir_turns_off_what_is_inside() {
        let fs = FakeFs::default();
        let prepared = prepare(&dirs("/d/aperta", "/d/cache"), &fs);
        assert_eq!(prepared.data, None);
        assert_eq!(prepared.logs, None);
        assert_eq!(prepared.crash_reports, None);
        assert_eq!(prepared.opened_entries, None);
        assert_eq!(prepared.refused, [PathBuf::from("/d/aperta")]);
        let created = fs.created.lock().map(|c| c.clone()).unwrap_or_default();
        assert!(!created.iter().any(|d| d.starts_with("/d/aperta")));
    }

    #[test]
    fn other_errors_turn_off_without_a_refusal() {
        let prepared = prepare(&dirs("/d/data", "/d/guasta"), &FakeFs::default());
        assert_eq!(prepared.cache, None);
        assert!(prepared.refused.is_empty());
    }
}
