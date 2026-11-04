// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! "Apri" e "Mostra nella cartella" per un risultato: che cosa fare ([`open_plan`],
//! regole pure) e il lavoro vero ([`run`], fuori dal thread dell'interfaccia).
//!
//! Regole:
//! - un file sul disco si apre con il programma predefinito del sistema (Windows:
//!   l'associazione dei tipi di file; Linux: il portale XDG), mai con un programma cercato
//!   per nome;
//! - un file che il sistema eseguirebbe invece di aprire (programmi, script, collegamenti:
//!   [`is_executable_name`]) non si apre: i risultati vengono da cartelle qualsiasi, e un
//!   doppio clic su "pulisci.bat" non deve eseguirlo; resta "Mostra nella cartella";
//! - una voce dentro un archivio va prima estratta in una cartella temporanea privata
//!   ([`ArchiveEntryExtractor`]: [`OpenedEntries`](crate::extraction::OpenedEntries) per le
//!   voci di testo); quando non si può, l'utente riceve un messaggio chiaro;
//! - "Mostra nella cartella" di una voce d'archivio mostra l'archivio.

use std::io;
use std::path::{Path, PathBuf};

use lynkeus_engine::DocPath;
use lynkeus_extract::ExtractError;

use crate::platform;

/// Estensioni che il sistema eseguirebbe (programmi, script, collegamenti, installatori,
/// file di registro), in minuscolo. Elenco largo di proposito: un file in più mostrato
/// nella cartella invece che aperto è un fastidio piccolo.
const EXECUTABLE_EXTENSIONS: [&str; 40] = [
    "exe",
    "com",
    "bat",
    "cmd",
    "pif",
    "scr",
    "cpl",
    "msc",
    "msi",
    "msp",
    "mst",
    "ps1",
    "psm1",
    "psd1",
    "ps1xml",
    "psc1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "wsc",
    "hta",
    "lnk",
    "url",
    "reg",
    "inf",
    "scf",
    "jar",
    "application",
    "appref-ms",
    "settingcontent-ms",
    "library-ms",
    "desktop",
    "sh",
    "run",
    "appimage",
    "py",
    "pyw",
];

/// Che cosa fare quando l'utente chiede di aprire un risultato.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenPlan {
    /// Aprire questo file con il programma predefinito.
    File(PathBuf),
    /// Non aprire: il sistema eseguirebbe il file. Si propone "Mostra nella cartella".
    Executable,
    /// Voce dentro un archivio: va estratta prima.
    ArchiveEntry,
}

/// Il piano per aprire `doc`.
pub fn open_plan(doc: &DocPath) -> OpenPlan {
    let shown_name = doc.inner.last().map_or(doc.path.as_path(), Path::new);
    if is_executable_name(shown_name) {
        OpenPlan::Executable
    } else if doc.is_inside_archive() {
        OpenPlan::ArchiveEntry
    } else {
        OpenPlan::File(doc.path.clone())
    }
}

/// Vero se il nome di `path` ha un'estensione che il sistema eseguirebbe.
///
/// Punti e spazi finali non contano (Windows li toglie: "a.bat. " è "a.bat"), maiuscole e
/// minuscole nemmeno.
pub fn is_executable_name(path: &Path) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    let name = name.to_string_lossy();
    let trimmed = name.trim_end_matches(['.', ' ']);
    let Some((_, extension)) = trimmed.rsplit_once('.') else {
        return false;
    };
    let extension = extension.to_lowercase();
    EXECUTABLE_EXTENSIONS.contains(&extension.as_str())
}

/// Estrae una voce d'archivio in un file nuovo della cartella temporanea privata dell'app
/// (cancellata all'uscita), per aprirla.
pub trait ArchiveEntryExtractor: Send + Sync {
    /// Estrae la voce `doc` e restituisce il percorso del file estratto.
    ///
    /// Il nome del file sul disco lo sceglie l'estrattore (mai quello della voce, che è
    /// solo un'etichetta); l'estensione può essere quella della voce, se ammessa.
    /// `Ok(None)` se l'estrazione non è disponibile.
    fn extract_to_private_temp(&self, doc: &DocPath) -> Result<Option<PathBuf>, ExtractError>;
}

/// Nessuna estrazione (processo estrattore non collegato): sempre `Ok(None)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoArchiveExtraction;

impl ArchiveEntryExtractor for NoArchiveExtraction {
    fn extract_to_private_temp(&self, _doc: &DocPath) -> Result<Option<PathBuf>, ExtractError> {
        Ok(None)
    }
}

/// Un'azione da eseguire fuori dal thread dell'interfaccia.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchTask {
    /// Aprire un file con il programma predefinito.
    Open(PathBuf),
    /// Estrarre una voce d'archivio e aprirla.
    OpenArchiveEntry(DocPath),
    /// Mostrare un file nella sua cartella.
    Reveal(PathBuf),
}

/// Com'è andata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchOutcome {
    /// Il sistema ha accettato la richiesta.
    Done,
    /// L'apertura delle voci d'archivio non è ancora disponibile.
    ArchiveUnavailable,
    /// Il sistema ha rifiutato (file sparito, nessun programma associato, …).
    Failed,
}

/// Esegue `task` (blocca: da chiamare in un thread di lavoro).
pub fn run(task: &LaunchTask, extractor: &dyn ArchiveEntryExtractor) -> LaunchOutcome {
    let result = match task {
        LaunchTask::Open(path) => {
            absolute(path).and_then(|p| platform::open_with_default_program(&p))
        }
        LaunchTask::Reveal(path) => absolute(path).and_then(|p| platform::reveal_in_folder(&p)),
        LaunchTask::OpenArchiveEntry(doc) => match extractor.extract_to_private_temp(doc) {
            Ok(Some(path)) => platform::open_with_default_program(&path),
            Ok(None) => return LaunchOutcome::ArchiveUnavailable,
            Err(_) => return LaunchOutcome::Failed,
        },
    };
    match result {
        Ok(()) => LaunchOutcome::Done,
        Err(_) => LaunchOutcome::Failed,
    }
}

/// Il percorso assoluto, senza accedere al disco: un percorso relativo non deve mai
/// arrivare al sistema, che lo risolverebbe rispetto a una cartella qualsiasi.
fn absolute(path: &Path) -> io::Result<PathBuf> {
    std::path::absolute(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_name_partitions() {
        for name in [
            "pulisci.bat",
            "SETUP.EXE",
            "x.ps1",
            "collegamento.lnk",
            "a.bat. ",
            "dir/b.Js",
            "avvia.sh",
        ] {
            assert!(is_executable_name(Path::new(name)), "{name}");
        }
        for name in [
            "note.txt",
            "bat",
            ".bashrc",
            "lettera.docx",
            "archivio.zip",
            "",
        ] {
            assert!(!is_executable_name(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn plan_partitions() {
        let file = DocPath::file("ricette/mare.txt");
        assert_eq!(
            open_plan(&file),
            OpenPlan::File(PathBuf::from("ricette/mare.txt"))
        );
        assert_eq!(open_plan(&DocPath::file("x.cmd")), OpenPlan::Executable);
        let entry = DocPath {
            path: "musica/archivio.zip".into(),
            inner: vec!["elenco.pdf".to_owned()],
        };
        assert_eq!(open_plan(&entry), OpenPlan::ArchiveEntry);
        // Si guarda il nome della voce, non quello dell'archivio.
        let script = DocPath {
            path: "musica/archivio.zip".into(),
            inner: vec!["installa.vbs".to_owned()],
        };
        assert_eq!(open_plan(&script), OpenPlan::Executable);
    }

    #[test]
    fn archive_entries_wait_for_the_extractor() {
        let entry = DocPath {
            path: "a.zip".into(),
            inner: vec!["b.txt".to_owned()],
        };
        assert_eq!(
            run(&LaunchTask::OpenArchiveEntry(entry), &NoArchiveExtraction),
            LaunchOutcome::ArchiveUnavailable
        );
    }
}
