// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Identità di un documento: file sul disco ed eventuale percorso dentro archivi annidati.

use std::fmt;
use std::path::{Path, PathBuf};

use lynkeus_core::neutralize_char;

/// Separatore mostrato tra un archivio e le sue voci.
pub const INNER_SEPARATOR: &str = " › ";

/// Un documento: un file sul disco, o una voce dentro uno o più archivi annidati.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocPath {
    /// Il file sul disco (per una voce d'archivio, l'archivio più esterno).
    pub path: PathBuf,
    /// Nomi delle voci, dall'archivio più esterno al più interno; vuoto per un file normale.
    ///
    /// Vengono dal contenuto degli archivi: solo etichette, mai usate come percorsi.
    pub inner: Vec<String>,
}

impl DocPath {
    /// Un file sul disco.
    pub fn file(path: impl Into<PathBuf>) -> DocPath {
        DocPath {
            path: path.into(),
            inner: Vec::new(),
        }
    }

    /// Vero se il documento è dentro un archivio.
    pub fn is_inside_archive(&self) -> bool {
        !self.inner.is_empty()
    }

    /// Percorso da mostrare relativo a `root`: "sotto/archivio.zip › doc.docx", con i
    /// caratteri neutralizzati come in [`Display`](fmt::Display).
    ///
    /// Se `path` non sta sotto `root` (o coincide con la radice-file), il nome del file.
    pub fn display_relative(&self, root: &Path) -> String {
        let relative = match self.path.strip_prefix(root) {
            Ok(rest) if !rest.as_os_str().is_empty() => rest.to_path_buf(),
            _ => self
                .path
                .file_name()
                .map_or_else(|| self.path.clone(), PathBuf::from),
        };
        join_display(&relative, &self.inner)
    }
}

impl fmt::Display for DocPath {
    /// Percorso completo: "C:\dati\archivio.zip › doc.docx". I caratteri che mostrati
    /// potrebbero ingannare (controllo, direzione del testo, separatori di riga, invisibili:
    /// [`neutralize_char`]), nel percorso come nei nomi delle voci, diventano U+FFFD: è un
    /// testo da mostrare, mai un percorso da riaprire.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&join_display(&self.path, &self.inner))
    }
}

/// Il percorso e le voci uniti da [`INNER_SEPARATOR`], con i caratteri che potrebbero
/// ingannare sostituiti da U+FFFD ([`neutralize_char`]).
fn join_display(path: &Path, inner: &[String]) -> String {
    let mut out: String = path
        .display()
        .to_string()
        .chars()
        .map(neutralize_char)
        .collect();
    for name in inner {
        out.push_str(INNER_SEPARATOR);
        out.extend(name.chars().map(neutralize_char));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_with_archive_entries() {
        let doc = DocPath {
            path: PathBuf::from("musica").join("archivio.zip"),
            inner: vec!["elenco\u{7}.pdf".to_owned()],
        };
        let expected = format!(
            "{}{INNER_SEPARATOR}elenco\u{FFFD}.pdf",
            Path::new("musica").join("archivio.zip").display()
        );
        assert_eq!(doc.to_string(), expected);
        assert_eq!(
            doc.display_relative(Path::new("musica")),
            "archivio.zip › elenco\u{FFFD}.pdf"
        );
    }

    #[test]
    fn control_and_bidi_chars_are_neutralized_in_the_path_too() {
        let root = PathBuf::from("dati\u{202E}txt.exe");
        let doc = DocPath {
            path: root.join("a\u{7}b\u{2066}.zip"),
            inner: vec![
                "voce\u{2069}\u{202A}.txt".to_owned(),
                "a\u{2028}b\u{200B}.txt".to_owned(),
            ],
        };
        let shown = doc.to_string();
        for bad in [
            '\u{202E}', '\u{7}', '\u{2066}', '\u{2069}', '\u{202A}', '\u{2028}', '\u{200B}',
        ] {
            assert!(!shown.contains(bad), "{bad:?} in {shown:?}");
        }
        assert_eq!(
            doc.display_relative(&root),
            "a\u{FFFD}b\u{FFFD}.zip › voce\u{FFFD}\u{FFFD}.txt › a\u{FFFD}b\u{FFFD}.txt"
        );
        // Gli altri caratteri restano: lettere accentate, spazi, separatori.
        let plain = Path::new("città nuove").join("perché.txt");
        assert_eq!(
            DocPath::file(&plain).to_string(),
            plain.display().to_string()
        );
    }

    #[test]
    fn relative_to_a_file_root_is_the_file_name() {
        let doc = DocPath::file(Path::new("viaggi").join("citta.txt"));
        let root = Path::new("viaggi").join("citta.txt");
        assert_eq!(doc.display_relative(&root), "citta.txt");
        assert_eq!(doc.display_relative(Path::new("altro")), "citta.txt");
    }
}
