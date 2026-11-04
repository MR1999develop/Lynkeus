// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Differenze tra sistemi operativi nella scansione; nessun altro modulo usa `cfg`.

use std::ffi::OsStr;
use std::fs::Metadata;

/// Vero se il file o la cartella è nascosto o di sistema: nome che inizia con un punto,
/// oppure (Windows) attributi `HIDDEN` o `SYSTEM`.
pub fn is_hidden(name: &OsStr, metadata: Option<&Metadata>) -> bool {
    name.as_encoded_bytes().first() == Some(&b'.') || metadata.is_some_and(attributes::hidden)
}

/// Vero se il file è solo online: leggerlo lo scaricherebbe (Windows,
/// `RECALL_ON_DATA_ACCESS` o `RECALL_ON_OPEN`). Su Linux sempre falso.
pub fn is_online_only(metadata: &Metadata) -> bool {
    attributes::online_only(metadata)
}

/// Confronto tra un nome di cartella e un nome da saltare: senza maiuscole (ASCII) su
/// Windows, dove il file system non le distingue; esatto altrove.
pub fn dir_name_matches(name: &OsStr, skip: &str) -> bool {
    let name = name.to_string_lossy();
    if cfg!(windows) {
        name.eq_ignore_ascii_case(skip)
    } else {
        name == skip
    }
}

#[cfg(windows)]
mod attributes {
    use std::fs::Metadata;
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x4_0000;
    const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

    pub fn hidden(metadata: &Metadata) -> bool {
        metadata.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0
    }

    pub fn online_only(metadata: &Metadata) -> bool {
        metadata.file_attributes()
            & (FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
            != 0
    }
}

#[cfg(not(windows))]
mod attributes {
    use std::fs::Metadata;

    pub fn hidden(_metadata: &Metadata) -> bool {
        false
    }

    pub fn online_only(_metadata: &Metadata) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_names_are_hidden() {
        assert!(is_hidden(OsStr::new(".git"), None));
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
}
