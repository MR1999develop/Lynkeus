// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Differenze tra sistemi operativi nella scansione, nell'avvio del processo estrattore e
//! nei file della cache; nessun altro modulo usa `cfg`.
//!
//! Gli attributi si leggono dai metadati della scansione: su Windows vengono dall'elenco
//! della cartella (`FindNextFileW`), senza aprire il file, quindi senza scaricare i file
//! solo online.

use std::ffi::OsStr;
use std::fs::{File, Metadata};
use std::io;
use std::path::{Component, Path, PathBuf};

/// Variabili d'ambiente che il processo estrattore riceve (tutte le altre si tolgono): su
/// Windows `SystemRoot`, senza la quale alcune DLL di sistema non si inizializzano; su Linux
/// nessuna.
pub const KEPT_ENV_VARS: &[&str] = imp::KEPT_ENV_VARS;

/// Crea `dir` (e le cartelle sopra) come cartella privata: su Linux con permessi 0700. Su
/// Windows eredita i permessi della cartella dell'utente; il descrittore esplicito arriva con
/// la fase 7.
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    imp::create_private_dir(dir)
}

/// Crea un file nuovo in scrittura; errore se esiste già (anche come collegamento). Su Linux
/// con permessi 0600.
pub fn create_new_private_file(path: &Path) -> io::Result<File> {
    imp::create_new_private_file(path)
}

/// Vero se il file o la cartella è nascosto o di sistema: su Windows attributi `HIDDEN` o
/// `SYSTEM`; altrove nome che inizia con un punto.
pub fn is_hidden(name: &OsStr, metadata: Option<&Metadata>) -> bool {
    imp::is_hidden(name, metadata)
}

/// Vero se il file è solo online: leggerlo lo scaricherebbe (Windows, attributi
/// `RECALL_ON_DATA_ACCESS`, `RECALL_ON_OPEN` o `OFFLINE`). Altrove sempre falso.
pub fn is_online_only(metadata: &Metadata) -> bool {
    imp::is_online_only(metadata)
}

/// Confronto tra un nome di cartella e un nome da saltare: senza maiuscole su Windows,
/// dove il file system non le distingue; esatto altrove.
pub fn dir_name_matches(name: &OsStr, skip: &str) -> bool {
    let name = name.to_string_lossy();
    if imp::CASE_INSENSITIVE {
        name.to_lowercase() == skip.to_lowercase()
    } else {
        name == skip
    }
}

/// Chiave di confronto di un percorso assoluto già normalizzato: in minuscolo su Windows,
/// identica altrove. Due radici con la stessa chiave sono la stessa radice.
pub fn path_key(normalized: &Path) -> PathBuf {
    if imp::CASE_INSENSITIVE {
        PathBuf::from(normalized.as_os_str().to_string_lossy().to_lowercase())
    } else {
        normalized.to_path_buf()
    }
}

/// Forma assoluta e lessicale di `path`: relativo → unito alla cartella corrente, `.`
/// tolti, `..` applicati sul testo (senza risolvere i link). `None` se la cartella corrente
/// non si può leggere.
pub fn normalize(path: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(path).ok()?;
    Some(lexical_normalize(&absolute))
}

/// Toglie `.` e applica `..` sul testo del percorso; `..` oltre la radice si ignora.
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if out.file_name().is_some() {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::fs::{File, Metadata, OpenOptions};
    use std::io;
    use std::os::windows::fs::MetadataExt;
    use std::path::Path;

    pub const CASE_INSENSITIVE: bool = true;

    pub const KEPT_ENV_VARS: &[&str] = &["SystemRoot"];

    pub fn create_private_dir(dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)
    }

    pub fn create_new_private_file(path: &Path) -> io::Result<File> {
        OpenOptions::new().write(true).create_new(true).open(path)
    }

    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
    const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x4_0000;
    const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

    pub fn is_hidden(_name: &OsStr, metadata: Option<&Metadata>) -> bool {
        metadata.is_some_and(|m| {
            m.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0
        })
    }

    pub fn is_online_only(metadata: &Metadata) -> bool {
        metadata.file_attributes()
            & (FILE_ATTRIBUTE_RECALL_ON_OPEN
                | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS
                | FILE_ATTRIBUTE_OFFLINE)
            != 0
    }
}

#[cfg(not(windows))]
mod imp {
    use std::ffi::OsStr;
    use std::fs::{DirBuilder, File, Metadata, OpenOptions};
    use std::io;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::path::Path;

    pub const CASE_INSENSITIVE: bool = false;

    pub const KEPT_ENV_VARS: &[&str] = &[];

    pub fn create_private_dir(dir: &Path) -> io::Result<()> {
        DirBuilder::new().recursive(true).mode(0o700).create(dir)
    }

    pub fn create_new_private_file(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }

    pub fn is_hidden(name: &OsStr, _metadata: Option<&Metadata>) -> bool {
        name.as_encoded_bytes().first() == Some(&b'.')
    }

    pub fn is_online_only(_metadata: &Metadata) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_names_are_hidden_only_outside_windows() {
        assert_eq!(is_hidden(OsStr::new(".git"), None), !cfg!(windows));
        assert!(!is_hidden(OsStr::new("git"), None));
    }

    #[test]
    fn skip_names() {
        assert!(dir_name_matches(OsStr::new("target"), "target"));
        assert!(!dir_name_matches(OsStr::new("targets"), "target"));
        assert_eq!(
            dir_name_matches(OsStr::new("$Recycle.Bin"), "$RECYCLE.BIN"),
            cfg!(windows)
        );
    }

    #[test]
    fn lexical_normalization() {
        let base = std::path::absolute("base").expect("cartella corrente");
        assert_eq!(
            lexical_normalize(&base.join("a").join(".").join("b").join("..").join("c")),
            base.join("a").join("c")
        );
        assert_eq!(
            normalize(Path::new("x/./y/..")),
            Some(base.with_file_name("x"))
        );
    }

    #[test]
    fn path_keys_ignore_case_only_on_windows() {
        let a = path_key(Path::new("/Dati/Note"));
        let b = path_key(Path::new("/dati/note"));
        assert_eq!(a == b, cfg!(windows));
    }
}
