// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Voce "Cerca con Lynkeus" nel clic destro di Explorer (Windows) e di Nautilus (Linux),
//! accesa e spenta dall'utente (docs/threat-model.md, 4.8 B).
//!
//! Qui le regole, senza I/O: la riga di comando e lo script ([`windows_command`],
//! [`nautilus_script`]), come riconoscere una voce propria ([`classify_windows`],
//! [`classify_scripts`]) e i casi d'uso ([`enable`], [`disable`], [`refresh`]) sopra un
//! [`ContextMenuStore`]; il registro e la cartella degli script sono in `platform`.
//!
//! - Windows: `HKCU\Software\Classes\{*,Directory,Directory\Background}\shell\LynkeusSearcher`
//!   con testo, icona = eseguibile e comando `"<exe>" --add "%1"` (`"%V\."` per lo sfondo
//!   di una cartella: `"%V"` sulla radice di un'unità darebbe `"C:\"`, dove `\"` è una
//!   virgoletta letterale per `CommandLineToArgvW`; con `\.` l'argomento resta `C:\\.`, che
//!   il guardiano dei percorsi normalizza in `C:\`). Nessun diritto di amministratore.
//! - Linux: script `~/.local/share/nautilus/scripts/<testo>` con `exec '<exe>' --add -- "$@"`;
//!   senza argomenti (clic sullo sfondo di una cartella) passa la cartella corrente `$PWD`.
//! - L'eseguibile viene da `current_exe()`; un percorso con `"`, NUL o a capo, o una riga
//!   oltre 260 caratteri, non si registra. Un eseguibile o una cartella dell'eseguibile
//!   modificabili da altri nemmeno: la voce eseguirebbe codice altrui con l'identità
//!   dell'utente.
//! - Una voce con lo stesso nome che non è nostra non si sovrascrive né si cancella.

use std::io;
use std::path::{Path, PathBuf};

/// Nome fisso della chiave sotto `shell` (Windows).
pub const VERB_KEY: &str = "LynkeusSearcher";
/// Caratteri massimi della riga di comando registrata.
pub const MAX_COMMAND_CHARS: usize = 260;
/// Seconda riga degli script generati: li riconosce come propri.
pub const SCRIPT_MARKER: &str = "# lynkeus-searcher: voce del clic destro generata dal programma";

/// Dove compare la voce su Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuTarget {
    /// Clic destro su un file qualsiasi (`*`).
    Files,
    /// Clic destro su una cartella (`Directory`).
    Folders,
    /// Clic destro sullo sfondo di una cartella aperta (`Directory\Background`).
    FolderBackground,
}

impl MenuTarget {
    /// Tutte, nell'ordine in cui si scrivono.
    pub const ALL: [MenuTarget; 3] = [
        MenuTarget::Files,
        MenuTarget::Folders,
        MenuTarget::FolderBackground,
    ];

    /// La chiave sotto `Software\Classes`.
    pub fn key_path(self) -> String {
        let class = match self {
            MenuTarget::Files => "*",
            MenuTarget::Folders => "Directory",
            MenuTarget::FolderBackground => "Directory\\Background",
        };
        format!("{class}\\shell\\{VERB_KEY}")
    }

    /// Il segnaposto di Explorer per l'elemento cliccato.
    fn placeholder(self) -> &'static str {
        match self {
            MenuTarget::Files | MenuTarget::Folders => "%1",
            MenuTarget::FolderBackground => r"%V\.",
        }
    }

    /// I segnaposti delle versioni precedenti: voci da riconoscere come nostre e riscrivere.
    fn legacy_placeholders(self) -> &'static [&'static str] {
        match self {
            MenuTarget::Files | MenuTarget::Folders => &[],
            MenuTarget::FolderBackground => &["%V"],
        }
    }
}

/// Perché la voce non si è accesa (o spenta).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MenuError {
    /// Eseguibile senza percorso assoluto.
    #[error("eseguibile senza percorso assoluto")]
    ExeNotAbsolute,
    /// Percorso dell'eseguibile con `"`, NUL, a capo o non UTF-8.
    #[error("percorso dell'eseguibile con caratteri non ammessi")]
    ExeUnsafeChars,
    /// Riga di comando oltre [`MAX_COMMAND_CHARS`].
    #[error("riga di comando troppo lunga")]
    CommandTooLong,
    /// L'eseguibile o la sua cartella sono modificabili da altri utenti.
    #[error("eseguibile o cartella dell'eseguibile modificabili da altri utenti")]
    ExeDirWritable,
    /// Una voce con lo stesso nome appartiene a un altro programma.
    #[error("voce di un altro programma")]
    Foreign,
    /// Errore di registro o di file.
    #[error("errore di scrittura: {0:?}")]
    Io(io::ErrorKind),
}

impl From<io::Error> for MenuError {
    fn from(error: io::Error) -> MenuError {
        MenuError::Io(error.kind())
    }
}

/// La voce da registrare, valida per costruzione.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    exe: PathBuf,
    exe_text: String,
    label: String,
    other_labels: Vec<String>,
}

impl MenuEntry {
    /// La voce per l'eseguibile `exe` con il testo `label` (nella lingua attuale);
    /// `other_labels` sono i testi delle altre lingue (script da riconoscere e togliere su
    /// Linux).
    pub fn new(exe: &Path, label: &str, other_labels: &[String]) -> Result<MenuEntry, MenuError> {
        if !exe.is_absolute() {
            return Err(MenuError::ExeNotAbsolute);
        }
        let exe_text = exe.to_str().ok_or(MenuError::ExeUnsafeChars)?.to_owned();
        if exe_text.contains(['"', '\0', '\n', '\r']) {
            return Err(MenuError::ExeUnsafeChars);
        }
        let entry = MenuEntry {
            exe: exe.to_owned(),
            exe_text,
            label: label.to_owned(),
            other_labels: other_labels
                .iter()
                .filter(|other| *other != label)
                .cloned()
                .collect(),
        };
        let longest = MenuTarget::ALL
            .iter()
            .map(|target| entry.windows_command(*target).chars().count())
            .max()
            .unwrap_or(0);
        if longest > MAX_COMMAND_CHARS {
            return Err(MenuError::CommandTooLong);
        }
        Ok(entry)
    }

    /// L'eseguibile.
    pub fn exe(&self) -> &Path {
        &self.exe
    }

    /// Il testo della voce.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// I testi delle altre lingue.
    pub fn other_labels(&self) -> &[String] {
        &self.other_labels
    }

    /// Il valore di `command` per `target`: `"<exe>" --add "%1"` (`"%V\."` per lo sfondo).
    pub fn windows_command(&self, target: MenuTarget) -> String {
        windows_command(&self.exe_text, target)
    }

    /// Lo script di Nautilus.
    pub fn nautilus_script(&self) -> String {
        nautilus_script(&self.exe_text)
    }
}

/// La riga di comando registrata per `exe` (percorso già controllato).
pub fn windows_command(exe: &str, target: MenuTarget) -> String {
    format!("\"{exe}\" --add \"{}\"", target.placeholder())
}

/// Vero se `command` è una riga di questo programma per `target`, anche con l'eseguibile
/// in un'altra cartella (spostato) o con il segnaposto di una versione precedente:
/// `"<…\nome dell'exe>" --add "%1"`.
pub fn is_own_command(command: &str, target: MenuTarget, exe_file_name: &str) -> bool {
    let Some(rest) = command.strip_prefix('"') else {
        return false;
    };
    let Some(quoted) = std::iter::once(target.placeholder())
        .chain(target.legacy_placeholders().iter().copied())
        .find_map(|placeholder| rest.strip_suffix(format!("\" --add \"{placeholder}\"").as_str()))
    else {
        return false;
    };
    if quoted.contains('"') {
        return false;
    }
    let name = quoted.rsplit(['\\', '/']).next().unwrap_or(quoted);
    name.eq_ignore_ascii_case(exe_file_name)
}

/// Il testo tra apici singoli per `sh`: ogni `'` diventa `'\''`.
pub fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Lo script di Nautilus per `exe` (percorso già controllato: niente NUL né a capo). Nessun
/// `eval`, nessuna espansione senza virgolette; `--` chiude le opzioni. Senza argomenti
/// (clic sullo sfondo di una cartella aperta) passa la cartella corrente, `$PWD`.
pub fn nautilus_script(exe: &str) -> String {
    format!(
        "#!/bin/sh\n{SCRIPT_MARKER}\n{NO_ARGUMENTS_MEANS_PWD}\nexec {} --add -- \"$@\"\n",
        sh_quote(exe)
    )
}

/// Riga dello script: senza argomenti, la cartella corrente.
const NO_ARGUMENTS_MEANS_PWD: &str = r#"if [ "$#" -eq 0 ]; then set -- "$PWD"; fi"#;

/// Come sta la voce rispetto a quella attesa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuState {
    /// Nessuna voce.
    Missing,
    /// La voce attesa, identica.
    Current,
    /// Una voce nostra ma diversa (eseguibile spostato, testo di un'altra lingua, voce
    /// incompleta): si riscrive.
    Outdated,
    /// Una voce con lo stesso nome che non è nostra: non si tocca.
    Foreign,
}

/// Una voce trovata nel registro: testo e comando (se ci sono).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FoundItem {
    /// Il valore predefinito della chiave (testo della voce).
    pub label: Option<String>,
    /// Il valore predefinito della sottochiave `command`.
    pub command: Option<String>,
}

/// Lo stato delle tre voci di Windows (`found` nell'ordine di [`MenuTarget::ALL`]; `None` =
/// chiave assente).
pub fn classify_windows(found: &[Option<FoundItem>; 3], entry: &MenuEntry) -> MenuState {
    if found.iter().all(Option::is_none) {
        return MenuState::Missing;
    }
    let exe_name = entry
        .exe
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut all_current = true;
    for (target, item) in MenuTarget::ALL.iter().zip(found) {
        let Some(item) = item else {
            all_current = false;
            continue;
        };
        match item.command.as_deref() {
            Some(command) if is_own_command(command, *target, &exe_name) => {
                let current = command == entry.windows_command(*target)
                    && item.label.as_deref() == Some(entry.label());
                all_current &= current;
            }
            // Una chiave con il nostro nome ma senza comando: avanzo di una scrittura
            // interrotta, si riscrive.
            None => all_current = false,
            Some(_) => return MenuState::Foreign,
        }
    }
    if all_current {
        MenuState::Current
    } else {
        MenuState::Outdated
    }
}

/// Lo stato degli script di Nautilus: `found` sono i file trovati con i nomi della voce
/// (testo della lingua attuale e delle altre) e il loro contenuto.
pub fn classify_scripts(found: &[(String, String)], entry: &MenuEntry) -> MenuState {
    let is_own = |content: &str| content.lines().nth(1) == Some(SCRIPT_MARKER);
    let expected = entry.nautilus_script();
    let mut current = false;
    let mut outdated = false;
    for (name, content) in found {
        let own = is_own(content);
        if name == entry.label() {
            if !own {
                return MenuState::Foreign;
            }
            if *content == expected {
                current = true;
            } else {
                outdated = true;
            }
        } else if own {
            // Lo script di un'altra lingua: si toglie.
            outdated = true;
        }
    }
    match (current, outdated) {
        (true, false) => MenuState::Current,
        (_, true) => MenuState::Outdated,
        (false, false) => MenuState::Missing,
    }
}

/// Dove sta la voce: il registro dell'utente o la cartella degli script.
pub trait ContextMenuStore: Send + Sync {
    /// Lo stato attuale rispetto a `entry`.
    fn state(&self, entry: &MenuEntry) -> io::Result<MenuState>;
    /// Scrive la voce (sostituendo una voce nostra diversa).
    fn install(&self, entry: &MenuEntry) -> io::Result<()>;
    /// Toglie le voci nostre (solo quelle).
    fn remove(&self, entry: &MenuEntry) -> io::Result<()>;
    /// Vero se l'eseguibile `exe` o la sua cartella sono modificabili da altri utenti
    /// (proprietario estraneo o scrittura concessa ad altri).
    fn exe_writable_by_others(&self, exe: &Path) -> io::Result<bool>;
}

/// Accende la voce: controlla l'eseguibile e la sua cartella, non tocca una voce altrui,
/// scrive se manca o è diversa.
pub fn enable(store: &dyn ContextMenuStore, entry: &MenuEntry) -> Result<(), MenuError> {
    refresh(store, entry).map(|_| ())
}

/// Come [`enable`] (all'avvio, con l'opzione accesa); vero se ha riscritto la voce
/// (mancante, eseguibile spostato, testo cambiato).
pub fn refresh(store: &dyn ContextMenuStore, entry: &MenuEntry) -> Result<bool, MenuError> {
    if store.exe_writable_by_others(entry.exe())? {
        return Err(MenuError::ExeDirWritable);
    }
    match store.state(entry)? {
        MenuState::Current => Ok(false),
        MenuState::Foreign => Err(MenuError::Foreign),
        MenuState::Missing | MenuState::Outdated => {
            store.install(entry)?;
            Ok(true)
        }
    }
}

/// Spegne la voce: toglie solo le voci nostre; una voce altrui resta.
pub fn disable(store: &dyn ContextMenuStore, entry: &MenuEntry) -> Result<(), MenuError> {
    match store.state(entry)? {
        MenuState::Missing | MenuState::Foreign => Ok(()),
        MenuState::Current | MenuState::Outdated => Ok(store.remove(entry)?),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn exe() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Programmi portatili\Lynkeus città\lynkeus-searcher.exe")
        } else {
            PathBuf::from("/opt/lynkeus città/lynkeus-searcher")
        }
    }

    fn entry() -> MenuEntry {
        MenuEntry::new(
            &exe(),
            "Cerca con Lynkeus",
            &["Search with Lynkeus".to_owned()],
        )
        .expect("voce valida")
    }

    #[test]
    fn commands_are_quoted() {
        let entry = entry();
        let exe = exe().display().to_string();
        assert_eq!(
            entry.windows_command(MenuTarget::Folders),
            format!("\"{exe}\" --add \"%1\"")
        );
        assert_eq!(
            entry.windows_command(MenuTarget::FolderBackground),
            format!(r#""{exe}" --add "%V\.""#)
        );
        assert_eq!(MenuTarget::Files.key_path(), r"*\shell\LynkeusSearcher");
        assert_eq!(
            MenuTarget::FolderBackground.key_path(),
            r"Directory\Background\shell\LynkeusSearcher"
        );
    }

    #[test]
    fn entry_partitions() {
        let refused = |path: &str| MenuEntry::new(Path::new(path), "x", &[]).err();
        assert_eq!(refused("relativo.exe"), Some(MenuError::ExeNotAbsolute));
        let base = if cfg!(windows) { r"C:\a" } else { "/a" };
        assert_eq!(
            refused(&format!("{base}\"b.exe")),
            Some(MenuError::ExeUnsafeChars)
        );
        assert_eq!(
            refused(&format!("{base}\nb.exe")),
            Some(MenuError::ExeUnsafeChars)
        );
        assert_eq!(
            refused(&format!("{base}\0b.exe")),
            Some(MenuError::ExeUnsafeChars)
        );
        // Al limite: la riga più lunga (quella con `"%V\."`) di 260 caratteri passa, 261 no.
        let overhead = MenuTarget::ALL
            .iter()
            .map(|target| windows_command("", *target).chars().count())
            .max()
            .unwrap_or(0);
        let fit = MAX_COMMAND_CHARS - overhead;
        let path_of = |len: usize| format!("{base}{}", "x".repeat(len - base.len()));
        assert!(MenuEntry::new(Path::new(&path_of(fit)), "x", &[]).is_ok());
        assert_eq!(
            MenuEntry::new(Path::new(&path_of(fit + 1)), "x", &[]).err(),
            Some(MenuError::CommandTooLong)
        );
    }

    #[test]
    fn own_commands_are_recognized_even_when_moved() {
        let name = "lynkeus-searcher.exe";
        let target = MenuTarget::Folders;
        assert!(is_own_command(
            r#""D:\altrove\Lynkeus-Searcher.EXE" --add "%1""#,
            target,
            name
        ));
        for foreign in [
            r#""C:\x\altro.exe" --add "%1""#,
            r#""C:\x\lynkeus-searcher.exe" --add "%V""#,
            r#""C:\x\lynkeus-searcher.exe" --add "%1" & calc"#,
            r#"C:\x\lynkeus-searcher.exe --add "%1""#,
            r#""C:\x" "\lynkeus-searcher.exe" --add "%1""#,
            "",
        ] {
            assert!(!is_own_command(foreign, target, name), "{foreign}");
        }
    }

    /// La voce dello sfondo delle versioni precedenti (`"%V"`) è nostra, da riscrivere; su
    /// un'altra voce `"%V"` resta estraneo.
    #[test]
    fn legacy_background_entries_are_ours_and_outdated() {
        let entry = entry();
        let exe = exe().display().to_string();
        let legacy = format!("\"{exe}\" --add \"%V\"");
        assert!(is_own_command(
            &legacy,
            MenuTarget::FolderBackground,
            "lynkeus-searcher.exe"
        ));
        assert!(!is_own_command(
            &legacy,
            MenuTarget::Folders,
            "lynkeus-searcher.exe"
        ));
        let mut found = found(entry.label(), |t| entry.windows_command(t));
        found[2] = Some(FoundItem {
            label: Some(entry.label().to_owned()),
            command: Some(legacy),
        });
        assert_eq!(classify_windows(&found, &entry), MenuState::Outdated);
    }

    /// Argomento che segna il figlio avviato dal test della riga di comando.
    #[cfg(windows)]
    const CHILD_MARKER: &str = "lynkeus-figlio-riga-di-comando-41b9";

    /// Prefisso della riga con gli argomenti stampata dal figlio.
    #[cfg(windows)]
    const ARGS_LINE: &str = "ARGOMENTI:";

    #[cfg(windows)]
    #[test]
    #[ignore = "figlio del test della riga di comando"]
    fn child_prints_its_arguments() {
        let args: Vec<String> = std::env::args().collect();
        if let Some(at) = args.iter().position(|a| a == CHILD_MARKER) {
            let rest = args.get(at + 1..).unwrap_or_default();
            println!("{ARGS_LINE}{}", rest.join("\u{1f}"));
        }
    }

    /// Il comando registrato per lo sfondo, con `%V` sostituito come fa Explorer, diviso dal
    /// vero `CommandLineToArgvW` di un processo figlio: la radice di un'unità e una cartella
    /// con spazi arrivano intere, e il guardiano dei percorsi le normalizza.
    #[cfg(windows)]
    #[test]
    fn the_background_command_survives_the_real_command_line_split() {
        use std::os::windows::process::CommandExt;

        let entry = entry();
        let command = entry.windows_command(MenuTarget::FolderBackground);
        let quoted_exe = format!("\"{}\" ", exe().display());
        let arguments = command
            .strip_prefix(&quoted_exe)
            .expect("comando dell'eseguibile");
        let folder = tempfile::tempdir().expect("cartella");
        let with_spaces = std::path::absolute(folder.path().join("Dati utente")).expect("assoluto");
        std::fs::create_dir(&with_spaces).expect("cartella con spazi");
        let drive_root = PathBuf::from(r"C:\");
        let test_exe = std::env::current_exe().expect("eseguibile del test");
        for clicked in [drive_root, with_spaces] {
            let substituted = arguments.replace("%V", &clicked.display().to_string());
            let output = std::process::Command::new(&test_exe)
                .args([
                    "context_menu::tests::child_prints_its_arguments",
                    "--exact",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                    "--",
                    CHILD_MARKER,
                ])
                .raw_arg(&substituted)
                .output()
                .expect("figlio avviato");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let line = stdout
                .lines()
                .find_map(|l| l.split_once(ARGS_LINE).map(|(_, rest)| rest))
                .unwrap_or_else(|| panic!("nessun argomento stampato: {stdout}"));
            let received: Vec<std::ffi::OsString> =
                line.split('\u{1f}').map(std::ffi::OsString::from).collect();
            let paths = crate::cli::parse_added_paths(received).expect("argomenti validi");
            assert_eq!(paths.len(), 1, "{line}");
            assert_eq!(
                crate::instance::accept_path(&paths[0]),
                Ok(clicked.clone()),
                "{line}"
            );
        }
    }

    /// Senza argomenti (sfondo di una cartella) lo script passa `$PWD`.
    #[test]
    fn the_script_passes_the_current_folder_without_arguments() {
        let script = nautilus_script("/opt/lynkeus-searcher");
        let lines: Vec<&str> = script.lines().collect();
        assert_eq!(
            lines,
            [
                "#!/bin/sh",
                SCRIPT_MARKER,
                r#"if [ "$#" -eq 0 ]; then set -- "$PWD"; fi"#,
                r#"exec '/opt/lynkeus-searcher' --add -- "$@""#,
            ]
        );
    }

    /// Le voci trovate: testo e comando per ogni posto.
    fn found(label: &str, command: impl Fn(MenuTarget) -> String) -> [Option<FoundItem>; 3] {
        MenuTarget::ALL.map(|target| {
            Some(FoundItem {
                label: Some(label.to_owned()),
                command: Some(command(target)),
            })
        })
    }

    #[test]
    fn windows_states() {
        let entry = entry();
        let current = found(entry.label(), |t| entry.windows_command(t));
        assert_eq!(classify_windows(&current, &entry), MenuState::Current);
        assert_eq!(
            classify_windows(&[None, None, None], &entry),
            MenuState::Missing
        );
        let moved_exe = if cfg!(windows) {
            r"E:\nuova\lynkeus-searcher.exe"
        } else {
            "/nuova/lynkeus-searcher.exe"
        };
        let moved = found(entry.label(), |t| windows_command(moved_exe, t));
        assert_eq!(classify_windows(&moved, &entry), MenuState::Outdated);
        let english = found("Search with Lynkeus", |t| entry.windows_command(t));
        assert_eq!(classify_windows(&english, &entry), MenuState::Outdated);
        let partial = [current[0].clone(), None, None];
        assert_eq!(classify_windows(&partial, &entry), MenuState::Outdated);
        let mut foreign = current.clone();
        foreign[1] = found("Altro", |_| r#""C:\altro.exe" "%1""#.to_owned())[1].clone();
        assert_eq!(classify_windows(&foreign, &entry), MenuState::Foreign);
    }

    #[test]
    fn sh_quoting() {
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("a'b"), r"'a'\''b'");
        assert_eq!(sh_quote("$(x) `y` \\"), "'$(x) `y` \\'");
        let script = nautilus_script("/opt/l'k/lynkeus-searcher");
        assert_eq!(
            script,
            format!(
                "#!/bin/sh\n{SCRIPT_MARKER}\n{NO_ARGUMENTS_MEANS_PWD}\nexec '/opt/l'\\''k/lynkeus-searcher' --add -- \"$@\"\n"
            )
        );
    }

    #[test]
    fn script_states() {
        let entry = entry();
        let mine = (entry.label().to_owned(), entry.nautilus_script());
        assert_eq!(
            classify_scripts(std::slice::from_ref(&mine), &entry),
            MenuState::Current
        );
        assert_eq!(classify_scripts(&[], &entry), MenuState::Missing);
        let old = (
            entry.label().to_owned(),
            nautilus_script("/vecchio/lynkeus-searcher"),
        );
        assert_eq!(classify_scripts(&[old], &entry), MenuState::Outdated);
        let english = ("Search with Lynkeus".to_owned(), entry.nautilus_script());
        assert_eq!(
            classify_scripts(&[mine.clone(), english], &entry),
            MenuState::Outdated
        );
        let foreign = (
            entry.label().to_owned(),
            "#!/bin/sh\necho altro\n".to_owned(),
        );
        assert_eq!(classify_scripts(&[foreign], &entry), MenuState::Foreign);
    }

    /// Un registro finto: lo stato e le operazioni fatte.
    struct FakeStore {
        state: Mutex<MenuState>,
        writable: bool,
        log: Mutex<Vec<&'static str>>,
    }

    impl FakeStore {
        fn new(state: MenuState, writable: bool) -> FakeStore {
            FakeStore {
                state: Mutex::new(state),
                writable,
                log: Mutex::new(Vec::new()),
            }
        }

        fn log(&self) -> Vec<&'static str> {
            self.log.lock().map(|l| l.clone()).unwrap_or_default()
        }
    }

    impl ContextMenuStore for FakeStore {
        fn state(&self, _entry: &MenuEntry) -> io::Result<MenuState> {
            Ok(*self.state.lock().map_err(|_| io::Error::other("lock"))?)
        }

        fn install(&self, _entry: &MenuEntry) -> io::Result<()> {
            self.log
                .lock()
                .map_err(|_| io::Error::other("lock"))?
                .push("install");
            *self.state.lock().map_err(|_| io::Error::other("lock"))? = MenuState::Current;
            Ok(())
        }

        fn remove(&self, _entry: &MenuEntry) -> io::Result<()> {
            self.log
                .lock()
                .map_err(|_| io::Error::other("lock"))?
                .push("remove");
            *self.state.lock().map_err(|_| io::Error::other("lock"))? = MenuState::Missing;
            Ok(())
        }

        fn exe_writable_by_others(&self, _exe: &Path) -> io::Result<bool> {
            Ok(self.writable)
        }
    }

    #[test]
    fn use_cases_respect_foreign_entries_and_writable_folders() {
        let entry = entry();
        let missing = FakeStore::new(MenuState::Missing, false);
        assert_eq!(refresh(&missing, &entry), Ok(true));
        assert_eq!(refresh(&missing, &entry), Ok(false));
        assert_eq!(disable(&missing, &entry), Ok(()));
        assert_eq!(missing.log(), ["install", "remove"]);

        let moved = FakeStore::new(MenuState::Outdated, false);
        assert_eq!(refresh(&moved, &entry), Ok(true));

        let foreign = FakeStore::new(MenuState::Foreign, false);
        assert_eq!(enable(&foreign, &entry), Err(MenuError::Foreign));
        assert_eq!(disable(&foreign, &entry), Ok(()));
        assert!(foreign.log().is_empty(), "voce altrui toccata");

        let writable = FakeStore::new(MenuState::Missing, true);
        assert_eq!(enable(&writable, &entry), Err(MenuError::ExeDirWritable));
        assert!(writable.log().is_empty());
    }
}
