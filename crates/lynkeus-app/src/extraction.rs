// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Collegamento tra l'app e i processi estrattori: costruzione della fonte dei documenti
//! (pool e cache), unità dei documenti per l'anteprima ([`PoolDocumentUnits`]) e apertura
//! delle voci d'archivio ([`OpenedEntries`]).
//!
//! **Voci d'archivio da aprire.** Il processo estrattore manda i byte originali della voce
//! (con gli stessi limiti degli archivi nella ricerca, e al più il massimo per voce); l'app
//! li scrive in un file nuovo, con nome generato dal programma ed estensione della voce resa
//! innocua ([`opened_extension`]), nella cartella privata dei file aperti, che si svuota
//! all'avvio e all'uscita. Un file scritto a metà si cancella.

use std::fs;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use lynkeus_core::{ByteSize, Location, TextUnit};
use lynkeus_engine::{
    DocPath, DocumentSource, EntryEnd, ExtractorPool, Limits, TextCache, WarningKind,
    create_new_private_file, create_private_dir, default_pool_size,
};
use lynkeus_extract::text::soft_cut;
use lynkeus_extract::{EntrySink, ExtractError, ExtractOptions, Stopped};

use crate::format::ELLIPSIS;
use crate::launch::{ArchiveEntryExtractor, opened_extension};
use crate::platform;
use crate::preview::{DocumentUnits, MAX_UNIT_BYTES};
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
    /// Questo programma (`current_exe`, assoluto) con le cartelle della piattaforma.
    pub fn for_this_program() -> io::Result<ExtractionSetup> {
        Ok(ExtractionSetup {
            worker_exe: std::env::current_exe()?,
            cache_dir: platform::cache_dir(),
            opened_entries_dir: platform::opened_entries_dir(),
        })
    }
}

/// La fonte dei documenti: pool di `min(core/2, 4)` processi avviati al primo documento e,
/// se c'è la cartella, la cache con le impostazioni `cache`.
pub fn document_source(
    setup: &ExtractionSetup,
    cache: CacheSettings,
) -> io::Result<Arc<DocumentSource>> {
    let launcher = platform::worker_launcher(setup.worker_exe.clone(), WORKER_MEMORY_LIMIT)?;
    let pool = ExtractorPool::new(launcher, default_pool_size());
    let text_cache = setup
        .cache_dir
        .clone()
        .map(|dir| TextCache::new(dir, ByteSize::from_mib(cache.max_size_mib)));
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

/// Apre le voci d'archivio estraendone i byte in un file della cartella privata.
#[derive(Debug, Clone)]
pub struct OpenedEntries {
    source: Arc<DocumentSource>,
    dir: PathBuf,
    limits: Limits,
}

impl OpenedEntries {
    /// Voci estratte in `dir` (creata privata al primo uso), con i limiti delle ricerche.
    pub fn new(source: Arc<DocumentSource>, dir: PathBuf, limits: Limits) -> OpenedEntries {
        OpenedEntries {
            source,
            dir,
            limits,
        }
    }
}

impl ArchiveEntryExtractor for OpenedEntries {
    fn extract_to_private_temp(&self, doc: &DocPath) -> Result<Option<PathBuf>, ExtractError> {
        let Some(name) = doc.inner.last() else {
            return Ok(None);
        };
        let (path, file) = create_new_entry_file(&self.dir, &opened_extension(name))
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
            EntryEnd::Extracted { .. } => sink.finish().map_err(ExtractError::from_io),
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
fn create_new_entry_file(dir: &Path, extension: &str) -> io::Result<(PathBuf, File)> {
    create_private_dir(dir)?;
    for _ in 0..16 {
        let path = dir.join(format!("voce-{:016x}.{extension}", unique_number()));
        match create_new_private_file(&path) {
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

/// Cancella i file della cartella dei file aperti (all'avvio e all'uscita). Un file ancora
/// aperto da un altro programma resta e si riprova la volta dopo.
pub fn clean_opened_entries(dir: &Path) {
    let Ok(listing) = fs::read_dir(dir) else {
        return;
    };
    for item in listing.flatten() {
        if item.file_type().is_ok_and(|t| t.is_file()) {
            let _removed = fs::remove_file(item.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_files_have_generated_names_and_are_new() {
        let dir = tempfile::tempdir().expect("cartella");
        let target = dir.path().join("aperti");
        let (first, mut file) = create_new_entry_file(&target, "pdf").expect("creato");
        assert!(file.write_all(b"%PDF").is_ok());
        drop(file);
        let (second, _) = create_new_entry_file(&target, "txt").expect("creato");
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
        clean_opened_entries(&target);
        assert!(!first.exists() && !second.exists());
        // Una cartella che non c'è non è un errore.
        clean_opened_entries(&dir.path().join("mai"));
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
