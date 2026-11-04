// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Cosa cercare e dove: radici, query, filtri e limiti.

use std::path::{Path, PathBuf};
use std::time::Duration;

use lynkeus_core::{ByteSize, CompiledQuery};
use lynkeus_extract::ArchiveLimits;
use serde::{Deserialize, Serialize};

/// Una ricerca da avviare con [`start_search`](crate::start_search).
#[derive(Debug, Clone)]
pub struct SearchRequest {
    /// Cartelle (ricorsive) e file, nell'ordine scelto dall'utente; l'indice di una radice
    /// è [`Hit::root`](crate::Hit::root).
    pub roots: Vec<PathBuf>,
    /// La query già compilata: gli errori dei termini si mostrano prima di avviare.
    pub query: CompiledQuery,
    /// Quali file leggere.
    pub filters: Filters,
    /// Limiti sui risultati e sulle risorse.
    pub limits: Limits,
}

/// Quali file e cartelle si leggono.
///
/// Si applicano dentro le cartelle radice; una radice scelta dall'utente (cartella o file)
/// non è mai esclusa per nome, estensione o attributi, solo per dimensione.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filters {
    /// Estensioni ammesse o escluse.
    pub extensions: ExtensionFilter,
    /// Nomi di cartelle da non visitare (confronto senza maiuscole su Windows).
    pub skip_dirs: Vec<String>,
    /// File e cartelle nascosti o di sistema.
    pub hidden: HiddenPolicy,
    /// Dimensione massima di un file; `None` = nessun limite.
    pub max_file_size: Option<ByteSize>,
    /// File solo online (es. OneDrive), che leggere scaricherebbe.
    pub online_only: OnlineFilesPolicy,
}

impl Filters {
    /// Cartelle saltate per impostazione predefinita.
    pub const DEFAULT_SKIP_DIRS: [&'static str; 5] = [
        ".git",
        "node_modules",
        "target",
        "$RECYCLE.BIN",
        "System Volume Information",
    ];

    /// Dimensione massima predefinita di un file.
    pub const DEFAULT_MAX_FILE_SIZE: ByteSize = ByteSize::from_mib(200);
}

impl Default for Filters {
    fn default() -> Self {
        Filters {
            extensions: ExtensionFilter::All,
            skip_dirs: Self::DEFAULT_SKIP_DIRS.map(String::from).to_vec(),
            hidden: HiddenPolicy::default(),
            max_file_size: Some(Self::DEFAULT_MAX_FILE_SIZE),
            online_only: OnlineFilesPolicy::default(),
        }
    }
}

/// File nascosti e di sistema (attributi di Windows o nome che inizia con un punto).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum HiddenPolicy {
    /// Saltati (predefinito).
    #[default]
    Skip,
    /// Letti come gli altri.
    Include,
}

/// File solo online (attributo `RECALL_ON_DATA_ACCESS` su Windows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum OnlineFilesPolicy {
    /// Saltati (predefinito): leggerli li scaricherebbe.
    #[default]
    Skip,
    /// Letti, quindi scaricati.
    Include,
}

/// Filtro sulle estensioni dei file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ExtensionFilter {
    /// Tutti i file (predefinito).
    #[default]
    All,
    /// Solo i file con una di queste estensioni.
    Include(Vec<Extension>),
    /// Tutti i file tranne quelli con una di queste estensioni.
    Exclude(Vec<Extension>),
}

impl ExtensionFilter {
    /// Vero se il file `path` passa il filtro.
    pub fn accepts(&self, path: &Path) -> bool {
        match self {
            ExtensionFilter::All => true,
            ExtensionFilter::Include(list) => list.iter().any(|e| e.matches(path)),
            ExtensionFilter::Exclude(list) => !list.iter().any(|e| e.matches(path)),
        }
    }
}

/// Perché un'estensione scritta dall'utente non è valida.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExtensionError {
    /// Vuota (anche solo ".").
    #[error("estensione vuota")]
    Empty,
    /// Più lunga di `max_chars` caratteri.
    #[error("estensione più lunga di {max_chars} caratteri")]
    TooLong {
        /// Il limite superato, [`Extension::MAX_CHARS`].
        max_chars: usize,
    },
    /// Contiene spazi, punti interni, separatori di percorso o jolly.
    #[error("carattere non ammesso nell'estensione")]
    InvalidChar,
}

/// Un'estensione di file valida, in minuscolo e senza punto: "txt", "docx".
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Extension(String);

impl Extension {
    /// Lunghezza massima, in caratteri.
    pub const MAX_CHARS: usize = 32;

    /// Valida un'estensione scritta dall'utente; un punto iniziale si toglie (".TXT" → "txt").
    pub fn parse(raw: &str) -> Result<Extension, ExtensionError> {
        if raw.len() > Self::MAX_CHARS * 4 + 1 {
            return Err(ExtensionError::TooLong {
                max_chars: Self::MAX_CHARS,
            });
        }
        let trimmed = raw.trim();
        let bare = trimmed.strip_prefix('.').unwrap_or(trimmed);
        if bare.is_empty() {
            return Err(ExtensionError::Empty);
        }
        if bare.chars().count() > Self::MAX_CHARS {
            return Err(ExtensionError::TooLong {
                max_chars: Self::MAX_CHARS,
            });
        }
        let invalid = |c: char| {
            c.is_whitespace() || c.is_control() || matches!(c, '.' | '/' | '\\' | '*' | '?' | ':')
        };
        if bare.chars().any(invalid) {
            return Err(ExtensionError::InvalidChar);
        }
        Ok(Extension(bare.to_lowercase()))
    }

    /// L'estensione, in minuscolo e senza punto.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Vero se l'ultima estensione di `path` è questa, senza distinguere le maiuscole.
    pub fn matches(&self, path: &Path) -> bool {
        path.extension()
            .is_some_and(|ext| ext.to_string_lossy().to_lowercase() == self.0)
    }
}

impl TryFrom<String> for Extension {
    type Error = ExtensionError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Extension::parse(&value)
    }
}

impl From<Extension> for String {
    fn from(value: Extension) -> Self {
        value.0
    }
}

/// Limiti sui risultati, sul testo mostrato e sulle risorse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Risultati massimi: raggiunto il limite la ricerca si ferma con
    /// [`Outcome::LimitReached`](crate::Outcome::LimitReached).
    pub max_results: usize,
    /// Caratteri di contesto intorno alla parola.
    pub context_chars: usize,
    /// Caratteri massimi di una parola, oltre i quali è troncata.
    pub word_max_chars: usize,
    /// Tempo massimo di estrazione di un documento nel processo estrattore.
    pub extract_timeout: Duration,
    /// Limiti sugli archivi.
    pub archive: ArchiveLimits,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_results: 200_000,
            context_chars: 120,
            word_max_chars: 200,
            extract_timeout: Duration::from_secs(60),
            archive: ArchiveLimits::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_parse_partitions() {
        assert_eq!(Extension::parse(".TXT").map(|e| e.0), Ok("txt".to_owned()));
        assert_eq!(Extension::parse("docx").map(|e| e.0), Ok("docx".to_owned()));
        assert_eq!(Extension::parse(""), Err(ExtensionError::Empty));
        assert_eq!(Extension::parse(" . "), Err(ExtensionError::Empty));
        assert_eq!(Extension::parse("tar.gz"), Err(ExtensionError::InvalidChar));
        assert_eq!(Extension::parse("*"), Err(ExtensionError::InvalidChar));
        assert!(Extension::parse(&"a".repeat(Extension::MAX_CHARS)).is_ok());
        assert_eq!(
            Extension::parse(&"a".repeat(Extension::MAX_CHARS + 1)),
            Err(ExtensionError::TooLong {
                max_chars: Extension::MAX_CHARS
            })
        );
    }

    #[test]
    fn extension_filter() {
        let txt = Extension::parse("txt").expect("valida");
        let include = ExtensionFilter::Include(vec![txt.clone()]);
        let exclude = ExtensionFilter::Exclude(vec![txt]);
        assert!(include.accepts(Path::new("a/B.TXT")));
        assert!(!include.accepts(Path::new("a/b.md")));
        assert!(!include.accepts(Path::new("a/txt")));
        assert!(!exclude.accepts(Path::new("b.txt")));
        assert!(exclude.accepts(Path::new("b")));
    }

    #[test]
    fn defaults_match_the_specification() {
        let filters = Filters::default();
        assert_eq!(filters.skip_dirs.len(), 5);
        assert_eq!(filters.max_file_size, Some(ByteSize::from_mib(200)));
        assert_eq!(Limits::default().max_results, 200_000);
    }
}
