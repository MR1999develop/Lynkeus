// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Differenze tra sistemi operativi nella scansione, nell'avvio del processo estrattore e
//! nei file privati (cache); nessun altro modulo usa `cfg`.
//!
//! **File privati** ([`PrivateFs`]): l'engine non ha codice `unsafe`, quindi su Windows il
//! descrittore esplicito (solo utente e SYSTEM) lo mette l'app, che inietta la sua
//! implementazione; [`PortablePrivateFs`] è quella di questo crate: su Linux completa
//! (0700 e 0600, verifica di proprietario e permessi di una cartella esistente), su Windows
//! senza descrittore (la cartella eredita i permessi del padre).
//!
//! Gli attributi si leggono dai metadati della scansione: su Windows vengono dall'elenco
//! della cartella (`FindNextFileW`), senza aprire il file, quindi senza scaricare i file
//! solo online.

use std::ffi::OsStr;
use std::fmt;
use std::fs::{File, Metadata};
use std::io;
use std::path::{Component, Path, PathBuf};

/// Variabili d'ambiente che il processo estrattore riceve (tutte le altre si tolgono): su
/// Windows `SystemRoot`, senza la quale alcune DLL di sistema non si inizializzano; su Linux
/// nessuna.
pub const KEPT_ENV_VARS: &[&str] = imp::KEPT_ENV_VARS;

/// Crea cartelle e file privati: accessibili solo all'utente corrente (e a SYSTEM su
/// Windows).
pub trait PrivateFs: Send + Sync + fmt::Debug {
    /// Crea `dir` e le cartelle mancanti sopra di essa, private. Se `dir` esiste già non ci
    /// si fida: se è di un altro proprietario o accessibile ad altri l'errore è
    /// [`NotPrivate`] (vedi [`NotPrivate::is`]), e nulla viene cambiato.
    fn create_dir(&self, dir: &Path) -> io::Result<()>;

    /// Crea un file nuovo, privato, aperto in scrittura; errore `AlreadyExists` se il nome
    /// esiste già, anche come collegamento (che non viene seguito).
    fn create_new_file(&self, path: &Path) -> io::Result<File>;
}

/// Una cartella esistente non è privata: di un altro proprietario, o con permessi per
/// altri. Arriva dentro un `io::Error` di tipo `PermissionDenied`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("cartella non privata: di un altro proprietario o accessibile ad altri")]
pub struct NotPrivate;

impl NotPrivate {
    /// L'errore da restituire.
    pub fn error() -> io::Error {
        io::Error::new(io::ErrorKind::PermissionDenied, NotPrivate)
    }

    /// Vero se `error` dice che una cartella esistente non è privata.
    pub fn is(error: &io::Error) -> bool {
        error
            .get_ref()
            .is_some_and(|inner| inner.downcast_ref::<NotPrivate>().is_some())
    }
}

/// Cartelle e file privati senza codice `unsafe`.
///
/// - Linux: cartelle create con 0700, file con `O_CREAT|O_EXCL|O_NOFOLLOW` e 0600 (la
///   `umask` può solo restringere); una cartella esistente si apre con
///   `O_DIRECTORY|O_NOFOLLOW` e si controlla sul descrittore: stesso uid, nessun permesso
///   per gruppo e altri, altrimenti [`NotPrivate`] (nessun `chmod` silenzioso).
/// - Windows: `create_dir_all` e `create_new`, con i permessi ereditati dal padre. L'app
///   usa al suo posto la versione con il descrittore esplicito.
#[derive(Debug, Clone, Copy, Default)]
pub struct PortablePrivateFs;

impl PrivateFs for PortablePrivateFs {
    fn create_dir(&self, dir: &Path) -> io::Result<()> {
        imp::create_private_dir(dir)
    }

    fn create_new_file(&self, path: &Path) -> io::Result<File> {
        imp::create_new_private_file(path)
    }
}

/// [`PortablePrivateFs::create_dir`].
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    PortablePrivateFs.create_dir(dir)
}

/// [`PortablePrivateFs::create_new_file`].
pub fn create_new_private_file(path: &Path) -> io::Result<File> {
    PortablePrivateFs.create_new_file(path)
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

    use rustix::fs::{Mode, OFlags};

    use super::NotPrivate;

    pub const CASE_INSENSITIVE: bool = false;

    pub const KEPT_ENV_VARS: &[&str] = &[];

    /// Permessi di gruppo e altri: in una cartella privata devono essere tutti spenti.
    const GROUP_OTHER_BITS: u32 = 0o077;

    pub fn create_private_dir(dir: &Path) -> io::Result<()> {
        // Le cartelle mancanti (anche intermedie) nascono 0700; una cartella che esiste già
        // non si tocca, si controlla.
        DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        check_private_dir(dir)
    }

    /// Controlla sul descrittore aperto (senza seguire collegamenti) che `dir` sia una
    /// cartella dell'utente senza permessi per gruppo e altri.
    fn check_private_dir(dir: &Path) -> io::Result<()> {
        let fd = rustix::fs::open(
            dir,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let stat = rustix::fs::fstat(&fd)?;
        let owner_is_me = stat.st_uid == rustix::process::geteuid().as_raw();
        if owner_is_me && stat.st_mode & GROUP_OTHER_BITS == 0 {
            Ok(())
        } else {
            Err(NotPrivate::error())
        }
    }

    pub fn create_new_private_file(path: &Path) -> io::Result<File> {
        let no_follow = i32::try_from(OFlags::NOFOLLOW.bits()).unwrap_or(0);
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(no_follow)
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
    fn private_files_are_new_only() {
        let dir = tempfile::tempdir().expect("cartella");
        let private = dir.path().join("a").join("b");
        assert!(create_private_dir(&private).is_ok());
        // Una seconda volta: la cartella c'è ed è privata.
        assert!(create_private_dir(&private).is_ok());
        let file = private.join("f.tmp");
        assert!(create_new_private_file(&file).is_ok());
        let again = create_new_private_file(&file).map(|_| ());
        assert_eq!(
            again.map_err(|e| e.kind()),
            Err(io::ErrorKind::AlreadyExists)
        );
    }

    #[test]
    fn not_private_is_recognized() {
        assert!(NotPrivate::is(&NotPrivate::error()));
        assert!(!NotPrivate::is(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
    }

    /// Linux: permessi 0700 e 0600, e rifiuto di una cartella aperta ad altri o di un
    /// collegamento al posto del file o della cartella.
    #[cfg(not(windows))]
    #[test]
    fn unix_permissions_and_refusals() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let dir = tempfile::tempdir().expect("cartella");
        let private = dir.path().join("privata");
        create_private_dir(&private).expect("creata");
        let mode = |p: &Path| {
            std::fs::symlink_metadata(p)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0)
        };
        assert_eq!(mode(&private), 0o700);
        let file = private.join("f");
        drop(create_new_private_file(&file).expect("file"));
        assert_eq!(mode(&file), 0o600);

        let open = dir.path().join("aperta");
        std::fs::create_dir(&open).expect("cartella");
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).expect("chmod");
        let refused = create_private_dir(&open).expect_err("rifiutata");
        assert!(NotPrivate::is(&refused));
        assert_eq!(mode(&open), 0o777, "nessun chmod silenzioso");

        let link = private.join("collegamento");
        symlink(dir.path().join("altrove"), &link).expect("symlink");
        assert!(create_new_private_file(&link).is_err());
        assert!(!dir.path().join("altrove").exists());
        let dir_link = dir.path().join("link-cartella");
        symlink(&private, &dir_link).expect("symlink");
        assert!(create_private_dir(&dir_link).is_err());
    }

    #[test]
    fn path_keys_ignore_case_only_on_windows() {
        let a = path_key(Path::new("/Dati/Note"));
        let b = path_key(Path::new("/dati/note"));
        assert_eq!(a == b, cfg!(windows));
    }
}
