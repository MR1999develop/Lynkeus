// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Collegamento tra l'app e i processi estrattori: costruzione della fonte dei documenti
//! (pool e cache), unità dei documenti per l'anteprima ([`PoolDocumentUnits`]) e apertura
//! delle voci d'archivio ([`OpenedEntries`]).
//!
//! **Voci d'archivio da aprire.** Il processo estrattore manda i byte originali della voce
//! (con gli stessi limiti degli archivi nella ricerca, e al più il massimo per voce); l'app
//! li scrive in un file nuovo, con nome generato dal programma ed estensione della voce resa
//! innocua ([`opened_extension`]), nella sottocartella di sessione di questa istanza
//! ([`SessionDir`]) dentro la cartella privata dei file aperti. Un file scritto a metà si
//! cancella. Il file riceve la zona d'origine dell'archivio sul disco (Mark-of-the-Web,
//! [`platform::copy_mark_of_the_web`]): un documento da uno zip scaricato si apre protetto
//! come se fosse stato scaricato da solo; se la zona non si scrive, il file si cancella.
//!
//! **Più istanze.** Ogni istanza ha la sua sottocartella `s-<32 cifre esadecimali>` con un
//! file `session.lock` bloccato finché l'istanza vive. All'avvio si cancellano solo le
//! sottocartelle il cui lock non è tenuto da nessuno ([`clean_stale_sessions`]); all'uscita
//! l'istanza cancella la propria. I file di un'altra istanza in corso non si toccano mai.

use std::fs;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use lynkeus_core::{ByteSize, Location, TextUnit};
use lynkeus_engine::{
    DocPath, DocumentSource, EntryEnd, ExtractorPool, Limits, PrivateFs, TextCache, WarningKind,
    default_pool_size,
};
use lynkeus_extract::text::soft_cut;
use lynkeus_extract::{EntrySink, ExtractError, ExtractOptions, Stopped};

use crate::format::ELLIPSIS;
use crate::launch::{ArchiveEntryExtractor, opened_extension};
use crate::platform;
use crate::preview::{DocumentUnits, MAX_UNIT_BYTES};
use crate::private_dirs::PreparedDirs;
use crate::settings::CacheSettings;

/// Limite di memoria di ogni processo estrattore: 1 GiB, abbastanza per un PDF di 200 MiB
/// (la libreria ne copia i byte) e le sue strutture, ben sotto la memoria di un computer
/// d'ufficio. Su Windows lo impone il Job Object, su Linux `RLIMIT_AS`.
pub const WORKER_MEMORY_LIMIT: ByteSize = lynkeus_extract::worker::DEFAULT_MEMORY_LIMIT;

/// Dove sono il processo estrattore e le cartelle che usa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionSetup {
    /// L'eseguibile da avviare con `--extract-worker` (percorso assoluto).
    pub worker_exe: PathBuf,
    /// Cartella della cache; `None` = nessuna cache.
    pub cache_dir: Option<PathBuf>,
    /// Cartella privata dei file estratti dagli archivi per aprirli; `None` = non si aprono.
    pub opened_entries_dir: Option<PathBuf>,
}

impl ExtractionSetup {
    /// Questo programma (`current_exe`, assoluto) con le cartelle private preparate
    /// (una cartella rifiutata spegne la sua funzione).
    pub fn for_this_program(dirs: &PreparedDirs) -> io::Result<ExtractionSetup> {
        Ok(ExtractionSetup {
            worker_exe: std::env::current_exe()?,
            cache_dir: dirs.cache.clone(),
            opened_entries_dir: dirs.opened_entries.clone(),
        })
    }
}

/// La fonte dei documenti: pool di `min(core/2, 4)` processi avviati al primo documento e,
/// se c'è la cartella, la cache con le impostazioni `cache` e cartella e file creati da
/// `private_fs`.
pub fn document_source(
    setup: &ExtractionSetup,
    cache: CacheSettings,
    private_fs: &Arc<dyn PrivateFs>,
) -> io::Result<Arc<DocumentSource>> {
    let launcher = platform::worker_launcher(setup.worker_exe.clone(), WORKER_MEMORY_LIMIT)?;
    let pool = ExtractorPool::new(launcher, default_pool_size());
    let text_cache = setup.cache_dir.clone().map(|dir| {
        TextCache::at(
            dir,
            ByteSize::from_mib(cache.max_size_mib),
            Arc::clone(private_fs),
        )
    });
    let source = DocumentSource::new(pool, text_cache);
    source.set_cache_enabled(cache.enabled);
    Ok(Arc::new(source))
}

/// Le opzioni d'estrazione dei limiti delle ricerche.
fn options(limits: &Limits) -> ExtractOptions {
    ExtractOptions {
        limits: limits.archive,
    }
}

/// Un errore per l'anteprima dal motivo di un avviso.
fn extract_error(kind: WarningKind) -> ExtractError {
    match kind {
        WarningKind::NotFound => ExtractError::Io(io::Error::from(io::ErrorKind::NotFound)),
        WarningKind::AccessDenied => ExtractError::AccessDenied,
        WarningKind::InUse => ExtractError::InUse,
        WarningKind::Io => ExtractError::Io(io::Error::other("lettura del documento")),
        WarningKind::Encrypted => ExtractError::Encrypted,
        WarningKind::Corrupt => ExtractError::Corrupt {
            reason: "documento non leggibile dal processo estrattore".to_owned(),
        },
        WarningKind::Timeout => ExtractError::Timeout,
        WarningKind::LimitExceeded(limit) => ExtractError::LimitExceeded(limit),
        WarningKind::ContentMismatch => ExtractError::ContentMismatch,
        WarningKind::NoText => ExtractError::NoText,
        WarningKind::ExtractorUnavailable => {
            ExtractError::Io(io::Error::other("processo estrattore non avviato"))
        }
    }
}

/// Le unità dei documenti per l'anteprima: dalla cache o rileggendo con il pool.
#[derive(Debug, Clone)]
pub struct PoolDocumentUnits {
    source: Arc<DocumentSource>,
    limits: Limits,
}

impl PoolDocumentUnits {
    /// Con i limiti delle ricerche (gli stessi della chiave della cache).
    pub fn new(source: Arc<DocumentSource>, limits: Limits) -> PoolDocumentUnits {
        PoolDocumentUnits { source, limits }
    }
}

impl DocumentUnits for PoolDocumentUnits {
    fn units_around(
        &self,
        doc: &DocPath,
        location: &Location,
        radius: usize,
        cancel: &AtomicBool,
    ) -> Result<Option<Vec<TextUnit>>, ExtractError> {
        let stop = || cancel.load(Ordering::Relaxed);
        let units = self
            .source
            .units_around(
                doc,
                location,
                radius,
                &options(&self.limits),
                self.limits.extract_timeout,
                &stop,
            )
            .map_err(extract_error)?;
        Ok(units.map(|units| units.into_iter().map(shorten).collect()))
    }
}

/// Un'unità al più [`MAX_UNIT_BYTES`] byte (più "…"), come le righe dei file di testo.
fn shorten(unit: TextUnit) -> TextUnit {
    if unit.text.len() <= MAX_UNIT_BYTES {
        return unit;
    }
    let cut = soft_cut(&unit.text, MAX_UNIT_BYTES);
    let mut text = unit.text;
    text.truncate(cut);
    text.push_str(ELLIPSIS);
    TextUnit {
        location: unit.location,
        text,
    }
}

/// Apre le voci d'archivio estraendone i byte in un file della sottocartella di sessione.
#[derive(Debug)]
pub struct OpenedEntries {
    source: Arc<DocumentSource>,
    base: PathBuf,
    limits: Limits,
    private_fs: Arc<dyn PrivateFs>,
    // Creata al primo uso; tolta da `close_session`.
    session: Mutex<Option<SessionDir>>,
}

impl OpenedEntries {
    /// Voci estratte in una sottocartella di sessione di `base` (create private al primo
    /// uso con `private_fs`), con i limiti delle ricerche.
    pub fn new(
        source: Arc<DocumentSource>,
        base: PathBuf,
        limits: Limits,
        private_fs: Arc<dyn PrivateFs>,
    ) -> OpenedEntries {
        OpenedEntries {
            source,
            base,
            limits,
            private_fs,
            session: Mutex::new(None),
        }
    }

    /// La sottocartella di sessione, creata se manca.
    fn session_path(&self) -> io::Result<PathBuf> {
        let mut session = self.session.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(dir) = session.as_ref() {
            return Ok(dir.path().to_owned());
        }
        let created = SessionDir::create(&self.base, self.private_fs.as_ref())?;
        let path = created.path().to_owned();
        *session = Some(created);
        Ok(path)
    }

    /// Cancella la sottocartella di questa sessione (all'uscita); i file ancora aperti da
    /// altri programmi restano e li toglie un avvio successivo.
    pub fn close_session(&self) {
        let session = self
            .session
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(session) = session {
            session.remove();
        }
    }
}

impl ArchiveEntryExtractor for OpenedEntries {
    fn extract_to_private_temp(&self, doc: &DocPath) -> Result<Option<PathBuf>, ExtractError> {
        let Some(name) = doc.inner.last() else {
            return Ok(None);
        };
        let dir = self.session_path().map_err(ExtractError::from_io)?;
        let (path, file) =
            create_new_entry_file(self.private_fs.as_ref(), &dir, &opened_extension(name))
                .map_err(ExtractError::from_io)?;
        let mut sink = FileSink {
            file: BufWriter::new(file),
            error: None,
        };
        let never = || false;
        let end = self.source.extract_entry(
            doc,
            self.limits.archive.max_entry_size.bytes(),
            &options(&self.limits),
            self.limits.extract_timeout,
            &never,
            &mut sink,
        );
        let result = match end {
            // La zona d'origine dell'archivio (Mark-of-the-Web) passa alla voce; se non si
            // scrive, la voce non si apre.
            EntryEnd::Extracted { .. } => sink
                .finish()
                .and_then(|()| std::path::absolute(&doc.path))
                .and_then(|archive| platform::copy_mark_of_the_web(&archive, &path))
                .map_err(ExtractError::from_io),
            EntryEnd::NotExtracted(kind) => Err(extract_error(WarningKind::from_extract(kind))),
            EntryEnd::Failed(failure) => Err(extract_error(failure.warning_kind())),
            // Solo la scrittura del file ferma l'estrazione.
            EntryEnd::Stopped => Err(sink.error.map_or_else(
                || ExtractError::Io(io::Error::other("scrittura del file interrotta")),
                ExtractError::from_io,
            )),
        };
        match result {
            Ok(()) => Ok(Some(path)),
            Err(error) => {
                // Un file a metà non resta nella cartella.
                let _removed = fs::remove_file(&path);
                Err(error)
            }
        }
    }
}

/// Scrive i byte di una voce nel file aperto; il primo errore di scrittura ferma
/// l'estrazione.
struct FileSink {
    file: BufWriter<File>,
    error: Option<io::Error>,
}

impl FileSink {
    /// Svuota il buffer; errore se una scrittura è fallita.
    fn finish(self) -> io::Result<()> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let mut file = self.file;
        file.flush()
    }
}

impl EntrySink for FileSink {
    fn bytes(&mut self, chunk: &[u8]) -> Result<(), Stopped> {
        self.file.write_all(chunk).map_err(|error| {
            self.error = Some(error);
            Stopped
        })
    }
}

/// Contatore per i nomi dei file aperti.
static OPENED_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Crea un file nuovo `voce-<16 cifre esadecimali>.<extension>` in `dir` (creata privata se
/// manca); il nome non dipende mai dal nome della voce, l'estensione è già resa innocua da
/// chi chiama.
fn create_new_entry_file(
    private_fs: &dyn PrivateFs,
    dir: &Path,
    extension: &str,
) -> io::Result<(PathBuf, File)> {
    private_fs.create_dir(dir)?;
    for _ in 0..16 {
        let path = dir.join(format!("voce-{:016x}.{extension}", unique_number()));
        match private_fs.create_new_file(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::from(io::ErrorKind::AlreadyExists))
}

// Un numero diverso a ogni chiamata e tra istanze del programma (tempo, processo,
/// contatore, mescolati con `blake3`). Non serve imprevedibile: il file si crea solo se non
/// esiste.
fn unique_number() -> u64 {
    let mut hasher = blake3::Hasher::new();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    hasher.update(&nanos.to_le_bytes());
    hasher.update(&std::process::id().to_le_bytes());
    hasher.update(&OPENED_COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    let digest = hasher.finalize();
    let mut first = [0u8; 8];
    first.copy_from_slice(&digest.as_bytes()[..8]);
    u64::from_le_bytes(first)
}

/// Prefisso delle sottocartelle di sessione.
const SESSION_PREFIX: &str = "s-";
/// Il file di lock dentro ogni sottocartella di sessione.
const SESSION_LOCK: &str = "session.lock";
/// Tentativi di creare una sottocartella di sessione.
const SESSION_ATTEMPTS: usize = 4;

/// La sottocartella dei file aperti di questa istanza: nome casuale e `session.lock`
/// bloccato finché il valore vive (il sistema rilascia il lock anche se il processo muore).
#[derive(Debug)]
pub struct SessionDir {
    path: PathBuf,
    // Tenuto aperto e bloccato: dice alle altre istanze che la sessione è viva.
    lock: File,
}

impl SessionDir {
    /// Crea in `base` (creata privata se manca) una sottocartella nuova con il lock già
    /// preso. Se una pulizia concorrente la toglie prima che il lock sia preso, si riprova
    /// con un altro nome, al più [`SESSION_ATTEMPTS`] volte.
    pub fn create(base: &Path, private_fs: &dyn PrivateFs) -> io::Result<SessionDir> {
        for _ in 0..SESSION_ATTEMPTS {
            let path = base.join(format!("{SESSION_PREFIX}{}", random_hex()?));
            private_fs.create_dir(&path)?;
            let lock_path = path.join(SESSION_LOCK);
            let lock = match private_fs.create_new_file(&lock_path) {
                Ok(lock) => lock,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::AlreadyExists
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            };
            match lock.try_lock() {
                Ok(()) => {}
                Err(fs::TryLockError::WouldBlock) => continue,
                Err(fs::TryLockError::Error(error)) => return Err(error),
            }
            // Il lock preso su un file già tolto da una pulizia non protegge nulla.
            if fs::symlink_metadata(&lock_path).is_ok() {
                return Ok(SessionDir { path, lock });
            }
        }
        Err(io::Error::other("sottocartella di sessione non creata"))
    }

    /// La sottocartella.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Rilascia il lock e cancella la sottocartella con il suo contenuto. Se qualcosa resta
    /// (file aperto da un altro programma), lo toglie la pulizia di un avvio successivo.
    pub fn remove(self) {
        let SessionDir { path, lock } = self;
        drop(lock);
        let _removed = fs::remove_dir_all(&path);
    }
}

/// 32 cifre esadecimali dal generatore del sistema.
fn random_hex() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Pulizia all'avvio della cartella dei file aperti `base`: cancella le sottocartelle di
/// sessione il cui lock non è tenuto da nessuno (istanze finite, anche in modo anomalo) e
/// i file sciolti lasciati dalle versioni che non usavano le sottocartelle. Le sottocartelle
/// con il lock preso (altre istanze in corso, o questa) restano. Un file ancora aperto da
/// un altro programma resta e si riprova la volta dopo. I collegamenti non si seguono.
pub fn clean_stale_sessions(base: &Path) {
    let Ok(listing) = fs::read_dir(base) else {
        return;
    };
    for item in listing.flatten() {
        let Ok(kind) = item.file_type() else {
            continue;
        };
        let path = item.path();
        if kind.is_file() {
            let _removed = fs::remove_file(&path);
        } else if kind.is_dir() && is_session_name(&item.file_name()) && session_is_stale(&path) {
            let _removed = fs::remove_dir_all(&path);
        }
    }
}

/// Vero per un nome `s-<32 cifre esadecimali>`.
fn is_session_name(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .and_then(|n| n.strip_prefix(SESSION_PREFIX))
        .is_some_and(|hex| hex.len() == 32 && hex.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Vero se nessuna istanza tiene il lock della sottocartella `dir`. Senza file di lock:
/// abbandonata solo se non è vuota (un'istanza crea il lock prima di qualunque file; una
/// cartella vuota può essere appena nata, e si lascia).
fn session_is_stale(dir: &Path) -> bool {
    match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.join(SESSION_LOCK))
    {
        // Il lock si rilascia chiudendo il file, prima di cancellare.
        Ok(lock) => lock.try_lock().is_ok(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::read_dir(dir)
            .map(|mut items| items.next().is_some())
            .unwrap_or(false),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_files_have_generated_names_and_are_new() {
        let dir = tempfile::tempdir().expect("cartella");
        let target = dir.path().join("aperti");
        let fs = platform::private_fs();
        let (first, mut file) = create_new_entry_file(fs.as_ref(), &target, "pdf").expect("creato");
        assert!(file.write_all(b"%PDF").is_ok());
        drop(file);
        let (second, _) = create_new_entry_file(fs.as_ref(), &target, "txt").expect("creato");
        assert_ne!(first, second);
        for (path, extension) in [(&first, ".pdf"), (&second, ".txt")] {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            assert!(
                name.starts_with("voce-") && name.ends_with(extension),
                "{name}"
            );
            assert_eq!(path.parent(), Some(target.as_path()));
        }
        assert_eq!(fs::read(&first).ok(), Some(b"%PDF".to_vec()));
    }

    /// La pulizia all'avvio toglie le sessioni finite e i file sciolti, mai i file della
    /// sessione di un'altra istanza in corso; all'uscita la sessione toglie sé stessa.
    #[test]
    fn startup_cleaning_spares_the_files_of_a_running_instance() {
        let base_dir = tempfile::tempdir().expect("cartella");
        let base = base_dir.path().join("temp");
        let fs_impl = platform::private_fs();
        let running = SessionDir::create(&base, fs_impl.as_ref()).expect("sessione in corso");
        let (open_file, _) =
            create_new_entry_file(fs_impl.as_ref(), running.path(), "pdf").expect("file");
        let ended = SessionDir::create(&base, fs_impl.as_ref()).expect("sessione finita");
        let ended_path = ended.path().to_owned();
        let (left_over, _) =
            create_new_entry_file(fs_impl.as_ref(), &ended_path, "txt").expect("file");
        // Il lock rilasciato senza cancellare: come un'istanza terminata in modo anomalo.
        drop(ended);
        let loose = base.join("voce-0123456789abcdef.txt");
        drop(fs_impl.create_new_file(&loose).expect("file sciolto"));
        let foreign = base.join("non-una-sessione");
        fs::create_dir(&foreign).expect("cartella estranea");

        clean_stale_sessions(&base);
        assert!(open_file.exists(), "file di un'istanza in corso cancellato");
        assert!(!left_over.exists() && !ended_path.exists());
        assert!(!loose.exists());
        assert!(foreign.exists(), "cartella non di sessione toccata");

        let running_path = running.path().to_owned();
        running.remove();
        assert!(!running_path.exists());
        // Una cartella che non c'è non è un errore.
        clean_stale_sessions(&base_dir.path().join("mai"));
    }

    #[test]
    fn session_names_are_recognized() {
        let valid = format!("s-{}", "a1".repeat(16));
        assert!(is_session_name(std::ffi::OsStr::new(&valid)));
        for name in ["s-", "s-xyz", "sessione", &format!("s-{}", "a".repeat(33))] {
            assert!(!is_session_name(std::ffi::OsStr::new(name)), "{name}");
        }
    }

    #[test]
    fn long_units_are_shortened_for_the_preview() {
        let unit = TextUnit {
            location: Location::Paragraph(1),
            text: "parola ".repeat(MAX_UNIT_BYTES),
        };
        let short = shorten(unit);
        assert!(short.text.len() <= MAX_UNIT_BYTES + ELLIPSIS.len());
        assert!(short.text.ends_with(ELLIPSIS));
    }

    /// Un processo estrattore in un thread di questo processo, collegato con due pipe: il
    /// codice dell'estrattore vero, senza l'eseguibile dell'app.
    struct ThreadLauncher;

    /// Il limite di memoria del "processo" di [`ThreadLauncher`]: nessun limite effettivo.
    /// `worker::run` lo vuole sempre, ma qui varrebbe per tutto il processo dei test
    /// (`RLIMIT_AS` su Linux); `u64::MAX` è `RLIM_INFINITY`, il valore che quel limite ha
    /// già. Su Windows il limite lo impone il Job Object, qui assente.
    const THREAD_WORKER_MEMORY: ByteSize = ByteSize::from_bytes(u64::MAX);

    /// Il "processo" di [`ThreadLauncher`]: esce quando lo stdin si chiude.
    struct ThreadWorker {
        done: std::sync::mpsc::Receiver<bool>,
        exited: Option<lynkeus_engine::WorkerExit>,
    }

    impl lynkeus_engine::WorkerProcess for ThreadWorker {
        fn wait_timeout(
            &mut self,
            timeout: std::time::Duration,
        ) -> io::Result<Option<lynkeus_engine::WorkerExit>> {
            if self.exited.is_none() {
                self.exited = match self.done.recv_timeout(timeout) {
                    Ok(true) => Some(lynkeus_engine::WorkerExit::Success),
                    Ok(false) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        Some(lynkeus_engine::WorkerExit::Failed { code: None })
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                };
            }
            Ok(self.exited)
        }

        fn kill(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl lynkeus_engine::WorkerLauncher for ThreadLauncher {
        fn launch(&self) -> io::Result<lynkeus_engine::LaunchedWorker> {
            let (child_stdin, parent_stdin) = io::pipe()?;
            let (parent_stdout, child_stdout) = io::pipe()?;
            let (done_tx, done) = std::sync::mpsc::channel();
            std::thread::Builder::new()
                .name("estrattore-di-prova".to_owned())
                .spawn(move || {
                    let config = lynkeus_extract::worker::WorkerConfig {
                        memory_limit: Some(THREAD_WORKER_MEMORY),
                    };
                    let ended = lynkeus_extract::worker::run(child_stdin, child_stdout, &config);
                    let _sent = done_tx.send(ended.is_ok());
                })?;
            Ok(lynkeus_engine::LaunchedWorker {
                stdin: Box::new(parent_stdin),
                stdout: Box::new(parent_stdout),
                process: Box::new(ThreadWorker { done, exited: None }),
            })
        }
    }

    /// Le voci aperte da `base`, con un estrattore nello stesso processo.
    fn opened_entries(base: &Path) -> OpenedEntries {
        let pool = ExtractorPool::new(Arc::new(ThreadLauncher), std::num::NonZeroUsize::MIN);
        OpenedEntries::new(
            Arc::new(DocumentSource::new(pool, None)),
            base.to_owned(),
            Limits::default(),
            platform::private_fs(),
        )
    }

    /// Uno zip con la voce `name` di contenuto `bytes`.
    fn write_zip(path: &Path, name: &str, bytes: &[u8]) {
        let file = File::create(path).expect("zip");
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .expect("voce");
        zip.write_all(bytes).expect("byte della voce");
        zip.finish().expect("zip chiuso");
    }

    /// Con l'estrattore nello stesso processo: la voce estratta ha i byte originali e
    /// l'estensione della voce.
    #[test]
    fn entries_are_extracted_with_their_bytes() {
        let dir = tempfile::tempdir().expect("cartella");
        let archive = dir.path().join("archivio.zip");
        write_zip(&archive, "cartella/nota.txt", b"testo della nota");
        let entries = opened_entries(&dir.path().join("aperti"));
        let extracted = entries
            .extract_to_private_temp(&DocPath {
                path: archive,
                inner: vec!["cartella/nota.txt".to_owned()],
            })
            .expect("voce estratta")
            .expect("estrazione disponibile");
        assert_eq!(
            fs::read(&extracted).ok(),
            Some(b"testo della nota".to_vec())
        );
        assert_eq!(extracted.extension().and_then(|e| e.to_str()), Some("txt"));
        entries.close_session();
    }

    /// T-D02: una voce estratta da un archivio scaricato (con il Mark-of-the-Web, zona
    /// Internet) riceve la stessa zona, quindi il programma che la apre la tratta come
    /// scaricata (vista protetta, macro bloccate); da un archivio locale nessuna zona.
    #[cfg(windows)]
    #[test]
    fn extracted_entry_keeps_mark_of_the_web() {
        let dir = tempfile::tempdir().expect("cartella");
        let downloaded = dir.path().join("scaricato.zip");
        write_zip(&downloaded, "fattura.docm", b"documento con macro");
        let mut stream = downloaded.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        fs::write(
            &stream,
            "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.invalid/scaricato.zip\r\n",
        )
        .expect("zona dell'archivio");
        let local = dir.path().join("locale.zip");
        write_zip(&local, "nota.txt", b"testo");

        let entries = opened_entries(&dir.path().join("aperti"));
        let entry = DocPath {
            path: downloaded,
            inner: vec!["fattura.docm".to_owned()],
        };
        let extracted = entries
            .extract_to_private_temp(&entry)
            .expect("voce estratta")
            .expect("estrazione disponibile");
        assert_eq!(
            fs::read(&extracted).ok(),
            Some(b"documento con macro".to_vec())
        );
        let mut extracted_stream = extracted.as_os_str().to_owned();
        extracted_stream.push(":Zone.Identifier");
        let zone = fs::read_to_string(&extracted_stream).expect("zona sulla voce estratta");
        assert!(zone.contains("ZoneId=3"), "{zone:?}");
        assert!(!zone.contains("example.invalid"), "{zone:?}");

        let plain = entries
            .extract_to_private_temp(&DocPath {
                path: local,
                inner: vec!["nota.txt".to_owned()],
            })
            .expect("voce estratta")
            .expect("estrazione disponibile");
        let mut plain_stream = plain.as_os_str().to_owned();
        plain_stream.push(":Zone.Identifier");
        assert_eq!(
            fs::read(&plain_stream).map_err(|e| e.kind()),
            Err(io::ErrorKind::NotFound)
        );
        entries.close_session();
    }

    #[test]
    fn warning_kinds_round_trip_through_the_preview_error() {
        for kind in [
            WarningKind::AccessDenied,
            WarningKind::Encrypted,
            WarningKind::Corrupt,
            WarningKind::Timeout,
            WarningKind::NoText,
            WarningKind::NotFound,
        ] {
            assert_eq!(
                WarningKind::from_extract_error(&extract_error(kind)),
                Some(kind)
            );
        }
    }
}
