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
//! - una voce dentro un archivio va prima estratta, con i suoi byte originali, in un file
//!   con nome generato dal programma nella cartella temporanea privata
//!   ([`ArchiveEntryExtractor`]: [`OpenedEntries`](crate::extraction::OpenedEntries)); l'estensione
//!   del file è quella della voce resa innocua ([`opened_extension`]); quando non si può,
//!   l'utente riceve un messaggio chiaro;
//! - un capitolo di un EPUB non è un file: "Apri" apre il libro (o lo estrae, se il libro
//!   sta in un archivio);
//! - "Mostra nella cartella" di una voce d'archivio mostra l'archivio.

use std::io;
use std::path::{Path, PathBuf};

use lynkeus_engine::DocPath;
use lynkeus_extract::{ExtractError, Format};

use crate::platform;

/// Estensioni che il sistema eseguirebbe (programmi, script, collegamenti, installatori,
/// file di registro), in minuscolo. Elenco largo di proposito: un file in più mostrato
/// nella cartella invece che aperto è un fastidio piccolo.
const EXECUTABLE_EXTENSIONS: &[&str] = &[
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
    // Guida compilata, componente aggiuntivo di Excel, pacchetti e installatori delle app.
    "chm",
    "xll",
    "msix",
    "msixbundle",
    "appx",
    "appxbundle",
    "appinstaller",
    // Risoluzione dei problemi, script, connessioni remote, sandbox.
    "diagcab",
    "ws",
    "rdp",
    "wsb",
    // Ricerche e connettori salvati, siti aggiunti, gadget.
    "search-ms",
    "searchconnector-ms",
    "website",
    "gadget",
];

/// Estensione di un file estratto che non ha un'estensione ammessa ma è testo.
const TEXT_EXTENSION: &str = "txt";
/// Estensione di un file estratto che il sistema non deve poter eseguire né riconoscere.
const NEUTRAL_EXTENSION: &str = "bin";
/// Lunghezza massima di un'estensione ammessa.
const MAX_EXTENSION_CHARS: usize = 16;

/// Che cosa fare quando l'utente chiede di aprire un risultato.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenPlan {
    /// Aprire questo file con il programma predefinito.
    File(PathBuf),
    /// Non aprire: il sistema eseguirebbe il file. Si propone "Mostra nella cartella".
    Executable,
    /// Voce dentro un archivio: va estratta prima.
    ArchiveEntry(DocPath),
}

/// Il piano per aprire `doc`.
pub fn open_plan(doc: &DocPath) -> OpenPlan {
    let doc = openable(doc);
    let shown_name = doc.inner.last().map_or(doc.path.as_path(), Path::new);
    if is_executable_name(shown_name) {
        OpenPlan::Executable
    } else if doc.is_inside_archive() {
        OpenPlan::ArchiveEntry(doc)
    } else {
        OpenPlan::File(doc.path.clone())
    }
}

/// Il documento che "Apri" apre per `doc`: per un capitolo di un EPUB il libro, che è il
/// file che l'utente conosce; per gli altri documenti `doc` stesso.
fn openable(doc: &DocPath) -> DocPath {
    let Some((_, parents)) = doc.inner.split_last() else {
        return doc.clone();
    };
    let parent_is_epub = match parents.last() {
        Some(name) => Format::for_name(name) == Some(Format::Epub),
        None => Format::for_path(&doc.path) == Some(Format::Epub),
    };
    if parent_is_epub {
        DocPath {
            path: doc.path.clone(),
            inner: parents.to_vec(),
        }
    } else {
        doc.clone()
    }
}

/// L'estensione del file in cui si estrae la voce chiamata `entry_name`, senza punto.
///
/// Quella della voce, in minuscolo, se fatta solo di lettere e cifre ASCII (al più 16) e se
/// il sistema non la eseguirebbe; altrimenti `txt` per una voce senza formato riconosciuto
/// (che la ricerca legge come testo) e `bin` per le altre. Un programma o uno script dentro
/// un archivio non diventa mai un file che il sistema eseguirebbe.
pub fn opened_extension(entry_name: &str) -> String {
    if is_executable_name(Path::new(entry_name)) {
        return NEUTRAL_EXTENSION.to_owned();
    }
    let file = entry_name.rsplit(['/', '\\']).next().unwrap_or(entry_name);
    let trimmed = file.trim_end_matches(['.', ' ']);
    let extension = trimmed.rsplit_once('.').map(|(_, extension)| extension);
    match extension {
        Some(extension)
            if !extension.is_empty()
                && extension.len() <= MAX_EXTENSION_CHARS
                && extension.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            extension.to_ascii_lowercase()
        }
        _ if Format::for_name(entry_name).is_none() => TEXT_EXTENSION.to_owned(),
        _ => NEUTRAL_EXTENSION.to_owned(),
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
    /// L'apertura delle voci d'archivio non è disponibile (processo estrattore non
    /// collegato).
    ArchiveUnavailable,
    /// La voce d'archivio non si è potuta estrarre, per questo motivo.
    EntryFailed(EntryFailure),
    /// Il sistema ha rifiutato (file sparito, nessun programma associato, …).
    Failed,
}

/// Perché una voce d'archivio non si è estratta: ognuno ha il suo messaggio e il suo
/// rimedio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryFailure {
    /// La voce o l'archivio è protetto da password.
    Encrypted,
    /// La voce supera i limiti degli archivi (dimensione, rapporto di compressione,
    /// profondità) o il tempo massimo.
    LimitExceeded,
    /// L'archivio è danneggiato o la voce non c'è più.
    Corrupt,
    /// L'archivio non si legge (accesso negato, file in uso o sparito) o la voce non si è
    /// potuta scrivere nella cartella temporanea.
    Unreadable,
}

impl EntryFailure {
    /// Il motivo per l'utente dall'errore dell'estrazione.
    pub fn from_error(error: &ExtractError) -> EntryFailure {
        match error {
            ExtractError::Encrypted => EntryFailure::Encrypted,
            ExtractError::LimitExceeded(_)
            | ExtractError::TooLarge { .. }
            | ExtractError::Timeout => EntryFailure::LimitExceeded,
            ExtractError::Corrupt { .. }
            | ExtractError::ContentMismatch
            | ExtractError::Binary
            | ExtractError::NoText => EntryFailure::Corrupt,
            _ => EntryFailure::Unreadable,
        }
    }
}

/// Esegue `task` (blocca: da chiamare in un thread di lavoro).
pub fn run(task: &LaunchTask, extractor: &dyn ArchiveEntryExtractor) -> LaunchOutcome {
    let result = match task {
        LaunchTask::Open(path) => {
            absolute(path).and_then(|p| platform::open_with_default_program(&p))
        }
        LaunchTask::Reveal(path) => absolute(path).and_then(|p| platform::reveal_in_folder(&p)),
        LaunchTask::OpenArchiveEntry(doc) => match extractor.extract_to_private_temp(doc) {
            // Stessa regola dei file sul disco: un file che il sistema eseguirebbe non si
            // apre (l'estrattore non dovrebbe mai crearlo).
            Ok(Some(path)) if is_executable_name(&path) => return LaunchOutcome::Failed,
            Ok(Some(path)) => platform::open_with_default_program(&path),
            Ok(None) => return LaunchOutcome::ArchiveUnavailable,
            Err(error) => return LaunchOutcome::EntryFailed(EntryFailure::from_error(&error)),
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

    /// Ogni estensione che Windows esegue o interpreta invece di aprire come documento
    /// (guida, componenti aggiuntivi, pacchetti, script, collegamenti, ricerche salvate,
    /// connessioni remote, sandbox, pannello di controllo, registro).
    #[test]
    fn windows_launchers_are_executable() {
        for extension in [
            "chm",
            "xll",
            "msix",
            "msixbundle",
            "appx",
            "appxbundle",
            "appinstaller",
            "diagcab",
            "ws",
            "wsf",
            "rdp",
            "wsb",
            "search-ms",
            "searchconnector-ms",
            "library-ms",
            "settingcontent-ms",
            "url",
            "website",
            "cpl",
            "inf",
            "reg",
            "scf",
            "hta",
            "jse",
            "vbe",
            "vbs",
            "msc",
            "gadget",
            "application",
            "appref-ms",
        ] {
            let name = format!("file.{extension}");
            assert!(is_executable_name(Path::new(&name)), "{name}");
            let upper = format!("FILE.{}", extension.to_uppercase());
            assert!(is_executable_name(Path::new(&upper)), "{upper}");
        }
        // Estensioni vicine ma innocue.
        for name in [
            "guida.chmx",
            "foglio.xlsx",
            "x.wsx",
            "a.rdpx",
            "nota.url.txt",
        ] {
            assert!(!is_executable_name(Path::new(name)), "{name}");
        }
    }

    /// Un capitolo di un EPUB non è un file: si apre il libro.
    #[test]
    fn epub_chapters_open_the_book() {
        let chapter = DocPath {
            path: "libri/romanzo.epub".into(),
            inner: vec!["OEBPS/cap1.xhtml".to_owned()],
        };
        assert_eq!(
            open_plan(&chapter),
            OpenPlan::File(PathBuf::from("libri/romanzo.epub"))
        );
        // Un EPUB dentro uno zip: si estrae il libro.
        let nested = DocPath {
            path: "a.zip".into(),
            inner: vec!["Libro.EPUB".to_owned(), "testo/cap2.xhtml".to_owned()],
        };
        assert_eq!(
            open_plan(&nested),
            OpenPlan::ArchiveEntry(DocPath {
                path: "a.zip".into(),
                inner: vec!["Libro.EPUB".to_owned()],
            })
        );
    }

    #[test]
    fn opened_extension_partitions() {
        assert_eq!(opened_extension("cartella/Elenco.PDF"), "pdf");
        assert_eq!(opened_extension("interno.docx"), "docx");
        assert_eq!(opened_extension("note (2).txt"), "txt");
        // Senza estensione, o con caratteri non ammessi: testo se non è un formato noto.
        assert_eq!(opened_extension("LEGGIMI"), "txt");
        assert_eq!(opened_extension("strano.t\u{FFFD}t"), "txt");
        assert_eq!(opened_extension("dati.a b"), "txt");
        assert_eq!(opened_extension(&format!("x.{}", "a".repeat(40))), "txt");
        // Un programma o uno script non diventa mai un file eseguibile.
        assert_eq!(opened_extension("installa.exe"), "bin");
        assert_eq!(opened_extension("pulisci.bat. "), "bin");
        assert_eq!(opened_extension("x.search-ms"), "bin");
    }

    /// Un estrattore che restituisce un file eseguibile: non si apre.
    struct ExecutableExtractor;

    impl ArchiveEntryExtractor for ExecutableExtractor {
        fn extract_to_private_temp(&self, _doc: &DocPath) -> Result<Option<PathBuf>, ExtractError> {
            Ok(Some(PathBuf::from("voce-0123.exe")))
        }
    }

    #[test]
    fn an_extracted_executable_is_never_opened() {
        let entry = DocPath {
            path: "a.zip".into(),
            inner: vec!["b.pdf".to_owned()],
        };
        assert_eq!(
            run(&LaunchTask::OpenArchiveEntry(entry), &ExecutableExtractor),
            LaunchOutcome::Failed
        );
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
        assert_eq!(open_plan(&entry), OpenPlan::ArchiveEntry(entry.clone()));
        // Si guarda il nome della voce, non quello dell'archivio.
        let script = DocPath {
            path: "musica/archivio.zip".into(),
            inner: vec!["installa.vbs".to_owned()],
        };
        assert_eq!(open_plan(&script), OpenPlan::Executable);
    }

    /// Un estrattore che fallisce con un errore dato.
    struct FailingExtractor(fn() -> ExtractError);

    impl ArchiveEntryExtractor for FailingExtractor {
        fn extract_to_private_temp(&self, _doc: &DocPath) -> Result<Option<PathBuf>, ExtractError> {
            Err((self.0)())
        }
    }

    /// Ogni motivo di fallimento dell'estrazione ha il suo esito, non il generico "non si è
    /// aperto".
    #[test]
    fn entry_failures_say_why() {
        let entry = DocPath {
            path: "a.zip".into(),
            inner: vec!["b.pdf".to_owned()],
        };
        let cases: [(fn() -> ExtractError, EntryFailure); 5] = [
            (|| ExtractError::Encrypted, EntryFailure::Encrypted),
            (
                || ExtractError::LimitExceeded(lynkeus_extract::LimitKind::CompressionRatio),
                EntryFailure::LimitExceeded,
            ),
            (|| ExtractError::Timeout, EntryFailure::LimitExceeded),
            (
                || ExtractError::Corrupt {
                    reason: "x".to_owned(),
                },
                EntryFailure::Corrupt,
            ),
            (|| ExtractError::AccessDenied, EntryFailure::Unreadable),
        ];
        for (error, expected) in cases {
            let task = LaunchTask::OpenArchiveEntry(entry.clone());
            assert_eq!(
                run(&task, &FailingExtractor(error)),
                LaunchOutcome::EntryFailed(expected)
            );
        }
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
