// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Cache su disco del testo estratto dai documenti: la seconda ricerca negli stessi file non
//! avvia processi estrattori.
//!
//! È un derivato rigenerabile: una voce che manca, è corrotta o di un'altra versione si
//! ignora (e si cancella); niente `fsync`.
//!
//! **Chiave** ([`CacheKey`]): `blake3` di percorso, dimensione, data di modifica, versione del
//! programma (estrattori), versione del protocollo e del formato della cache, opzioni
//! d'estrazione (limiti). Un file cambiato ha un'altra chiave; le voci vecchie escono con
//! l'LRU.
//!
//! **Voce**: un file `<chiave esadecimale>.lkc` nella cartella della cache:
//!
//! ```text
//! magia:            4 byte   "LKC\0"
//! formato:          u16 LE   CACHE_FORMAT_VERSION
//! protocollo:       u16 LE   PROTOCOL_VERSION
//! chiave:           32 byte  la chiave completa (un nome rinominato non inganna)
//! byte originali:   u32 LE   lunghezza dei frame decompressi, al più MAX_ENTRY_BYTES
//! byte compressi:   u32 LE   lunghezza del contenuto che segue
//! crc32:            u32 LE   del contenuto compresso
//! contenuto:        lz4 (formato a blocchi) dei frame del protocollo, uno per ExtractEvent
//! ```
//!
//! In lettura si controllano, in quest'ordine, dimensione del file, magia, versioni, chiave,
//! lunghezze (prima di allocare), CRC, decompressione esatta, frame e sequenza degli eventi.
//!
//! **Scrittura**: file temporaneo creato nuovo (mai sopra un file esistente) nella stessa
//! cartella, poi rinomina sopra la voce. **Pulizia**: LRU sul totale dei byte delle voci; la
//! data di modifica di una voce letta si aggiorna, così l'ordine sopravvive al riavvio. Una
//! voce che da sola supera il limite non si salva.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lynkeus_core::ByteSize;
use lynkeus_extract::protocol::{FrameReader, PROTOCOL_VERSION, encode_frame};
use lynkeus_extract::{ExtractEvent, ExtractOptions, SequenceCheck};

use crate::platform;

/// Versione del formato delle voci.
pub const CACHE_FORMAT_VERSION: u16 = 1;

/// Limite predefinito della cache.
pub const DEFAULT_CACHE_SIZE: ByteSize = ByteSize::from_mib(500);

/// Byte massimi dei frame di una voce, decompressi: il testo di un documento più grande non
/// si salva (si rilegge ogni volta).
pub const MAX_ENTRY_BYTES: usize = 64 * 1024 * 1024;

const MAGIC: [u8; 4] = *b"LKC\0";
const HEADER_BYTES: usize = 4 + 2 + 2 + 32 + 4 + 4 + 4;
const ENTRY_EXTENSION: &str = "lkc";
const TEMP_EXTENSION: &str = "tmp";

/// Un temporaneo più vecchio di così è avanzato da un'interruzione: si cancella.
const STALE_TEMP: Duration = Duration::from_secs(60 * 60);

/// Versione del programma, che fissa anche la versione degli estrattori.
const PROGRAM_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Chiave di una voce: identifica il file così com'è ora e il modo di estrarlo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey([u8; 32]);

impl CacheKey {
    /// La chiave di `path` con la sua dimensione e la sua data di modifica, estratto con
    /// `options`.
    pub fn new(path: &Path, size: u64, modified: SystemTime, options: &ExtractOptions) -> CacheKey {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"lynkeus-searcher/cache-key\0");
        hasher.update(&CACHE_FORMAT_VERSION.to_le_bytes());
        hasher.update(&PROTOCOL_VERSION.to_le_bytes());
        hash_bytes(&mut hasher, PROGRAM_VERSION.as_bytes());
        hash_bytes(&mut hasher, path.as_os_str().as_encoded_bytes());
        hasher.update(&size.to_le_bytes());
        let (after_epoch, since) = match modified.duration_since(UNIX_EPOCH) {
            Ok(since) => (1_u8, since),
            Err(before) => (0_u8, before.duration()),
        };
        hasher.update(&[after_epoch]);
        hasher.update(&since.as_secs().to_le_bytes());
        hasher.update(&since.subsec_nanos().to_le_bytes());
        // Le opzioni con la stessa codifica del protocollo: canonica per gli stessi valori.
        let options = postcard_bytes(options);
        hash_bytes(&mut hasher, &options);
        CacheKey(*hasher.finalize().as_bytes())
    }

    /// La chiave di un file dai suoi metadati; `None` se il sistema non dà la data di
    /// modifica (allora il file non si mette in cache).
    pub fn for_file(path: &Path, options: &ExtractOptions) -> Option<CacheKey> {
        let metadata = fs::metadata(path).ok()?;
        let modified = metadata.modified().ok()?;
        Some(CacheKey::new(path, metadata.len(), modified, options))
    }

    /// Il nome del file della voce: solo cifre esadecimali e l'estensione.
    fn file_name(&self) -> String {
        let mut name = String::with_capacity(self.0.len() * 2 + 4);
        for byte in self.0 {
            name.push(hex_digit(byte >> 4));
            name.push(hex_digit(byte & 0x0F));
        }
        name.push('.');
        name.push_str(ENTRY_EXTENSION);
        name
    }
}

fn hash_bytes(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    hasher.update(&len.to_le_bytes());
    hasher.update(bytes);
}

fn postcard_bytes(options: &ExtractOptions) -> Vec<u8> {
    let mut out = Vec::new();
    // Il frame contiene la codifica postcard: basta come impronta delle opzioni.
    if encode_frame(options, &mut out).is_err() {
        out.clear();
    }
    out
}

fn hex_digit(nibble: u8) -> char {
    char::from_digit(u32::from(nibble), 16).unwrap_or('0')
}

/// Perché una voce letta non vale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Invalid {
    /// Da cancellare: corrotta, troncata, di un'altra versione o di un'altra chiave.
    Corrupt,
}

/// Una voce nell'indice in memoria.
#[derive(Debug, Clone, Copy)]
struct IndexEntry {
    bytes: u64,
    /// Ordine d'uso: più alto = usata più di recente.
    used: u64,
}

/// L'indice delle voci, caricato al primo uso.
#[derive(Debug, Default)]
struct Index {
    entries: HashMap<String, IndexEntry>,
    total: u64,
    clock: u64,
}

impl Index {
    fn touch(&mut self, name: &str) {
        self.clock += 1;
        if let Some(entry) = self.entries.get_mut(name) {
            entry.used = self.clock;
        }
    }

    fn insert(&mut self, name: String, bytes: u64) {
        self.clock += 1;
        let entry = IndexEntry {
            bytes,
            used: self.clock,
        };
        if let Some(old) = self.entries.insert(name, entry) {
            self.total = self.total.saturating_sub(old.bytes);
        }
        self.total = self.total.saturating_add(bytes);
    }

    fn remove(&mut self, name: &str) {
        if let Some(old) = self.entries.remove(name) {
            self.total = self.total.saturating_sub(old.bytes);
        }
    }

    /// Toglie dall'indice le voci meno usate finché il totale sta in `max`, tranne `keep`;
    /// restituisce i nomi da cancellare.
    fn evict(&mut self, max: u64, keep: Option<&str>) -> Vec<String> {
        if self.total <= max {
            return Vec::new();
        }
        let mut by_age: Vec<(u64, String)> = self
            .entries
            .iter()
            .filter(|(name, _)| Some(name.as_str()) != keep)
            .map(|(name, entry)| (entry.used, name.clone()))
            .collect();
        by_age.sort_unstable();
        let mut victims = Vec::new();
        for (_, name) in by_age {
            if self.total <= max {
                break;
            }
            self.remove(&name);
            victims.push(name);
        }
        victims
    }
}

/// La cache del testo estratto, condivisibile tra thread (e tra istanze del programma: due
/// scrittori della stessa voce scrivono lo stesso contenuto).
#[derive(Debug)]
pub struct TextCache {
    dir: PathBuf,
    max_bytes: AtomicU64,
    index: Mutex<Option<Index>>,
}

/// Contatore per i nomi dei temporanei del processo.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl TextCache {
    /// Una cache nella cartella `dir` (creata, privata, al primo salvataggio), con limite
    /// `max`. Non tocca il disco finché non serve.
    pub fn new(dir: PathBuf, max: ByteSize) -> TextCache {
        TextCache {
            dir,
            max_bytes: AtomicU64::new(max.bytes()),
            index: Mutex::new(None),
        }
    }

    /// La cartella della cache.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Il limite attuale.
    pub fn max_size(&self) -> ByteSize {
        ByteSize::from_bytes(self.max_bytes.load(Ordering::Relaxed))
    }

    /// Cambia il limite e, se serve, toglie subito le voci meno usate.
    pub fn set_max_size(&self, max: ByteSize) {
        self.max_bytes.store(max.bytes(), Ordering::Relaxed);
        let victims = {
            let mut guard = self.lock_index();
            self.loaded(&mut guard).evict(max.bytes(), None)
        };
        self.remove_files(&victims);
    }

    /// Byte occupati dalle voci (dopo il primo uso, l'indice in memoria).
    pub fn usage(&self) -> ByteSize {
        let mut guard = self.lock_index();
        ByteSize::from_bytes(self.loaded(&mut guard).total)
    }

    /// Gli eventi salvati per `key`, se la voce c'è ed è valida. Una voce non valida si
    /// cancella.
    pub fn get(&self, key: &CacheKey) -> Option<Vec<ExtractEvent>> {
        let name = key.file_name();
        let path = self.dir.join(&name);
        // Assente o illeggibile ora: nessuna conclusione sulla voce.
        let Ok(Some(bytes)) = read_bounded(&path) else {
            return None;
        };
        match decode_entry(&bytes, key) {
            Ok(events) => {
                if let Some(index) = self.lock_index().as_mut() {
                    index.touch(&name);
                }
                // L'ordine LRU sopravvive al riavvio; se non riesce, poco male.
                let _touched = touch_file(&path);
                Some(events)
            }
            Err(Invalid::Corrupt) => {
                if let Some(index) = self.lock_index().as_mut() {
                    index.remove(&name);
                }
                let _removed = fs::remove_file(&path);
                None
            }
        }
    }

    /// Salva gli eventi di `key`. Una voce più grande del limite (o di [`MAX_ENTRY_BYTES`]
    /// decompressa) non si salva e non è un errore.
    pub fn put(&self, key: &CacheKey, events: &[ExtractEvent]) -> io::Result<()> {
        let Some(entry) = encode_entry(key, events) else {
            return Ok(());
        };
        let entry_bytes = u64::try_from(entry.len()).unwrap_or(u64::MAX);
        let max = self.max_bytes.load(Ordering::Relaxed);
        if entry_bytes > max {
            return Ok(());
        }
        // Prima l'indice (che fa anche la pulizia dei temporanei avanzati), poi il file.
        drop(self.lock_index_loaded());
        platform::create_private_dir(&self.dir)?;
        let name = key.file_name();
        let target = self.dir.join(&name);
        let temp = self.write_temp(&name, &entry)?;
        if let Err(error) = fs::rename(&temp, &target) {
            let _removed = fs::remove_file(&temp);
            return Err(error);
        }
        let victims = {
            let mut guard = self.lock_index();
            let index = self.loaded(&mut guard);
            index.insert(name.clone(), entry_bytes);
            index.evict(max, Some(&name))
        };
        self.remove_files(&victims);
        Ok(())
    }

    /// Cancella tutte le voci (e i temporanei); restituisce i byte liberati.
    pub fn clear(&self) -> io::Result<ByteSize> {
        let mut guard = self.lock_index();
        let mut freed = 0_u64;
        let listing = match fs::read_dir(&self.dir) {
            Ok(listing) => listing,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                *guard = Some(Index::default());
                return Ok(ByteSize::from_bytes(0));
            }
            Err(error) => return Err(error),
        };
        let mut first_error = None;
        for item in listing.flatten() {
            let path = item.path();
            if !is_cache_file(&path) {
                continue;
            }
            let len = item.metadata().map_or(0, |m| m.len());
            match fs::remove_file(&path) {
                Ok(()) => freed = freed.saturating_add(len),
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        // L'indice si ricostruisce al prossimo uso da ciò che è rimasto.
        *guard = None;
        match first_error {
            Some(error) => Err(error),
            None => Ok(ByteSize::from_bytes(freed)),
        }
    }

    fn lock_index(&self) -> MutexGuard<'_, Option<Index>> {
        self.index.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_index_loaded(&self) -> MutexGuard<'_, Option<Index>> {
        let mut guard = self.lock_index();
        self.loaded(&mut guard);
        guard
    }

    /// L'indice, caricato dalla cartella se non c'è ancora.
    fn loaded<'g>(&self, guard: &'g mut MutexGuard<'_, Option<Index>>) -> &'g mut Index {
        guard.get_or_insert_with(|| scan(&self.dir))
    }

    /// Scrive `entry` in un temporaneo nuovo accanto alla voce `name`.
    fn write_temp(&self, name: &str, entry: &[u8]) -> io::Result<PathBuf> {
        let mut attempts = 0;
        loop {
            let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let temp = self.dir.join(format!(
                "{name}.{}-{unique}.{TEMP_EXTENSION}",
                std::process::id()
            ));
            match platform::create_new_private_file(&temp) {
                Ok(mut file) => {
                    let written = file.write_all(entry);
                    drop(file);
                    if let Err(error) = written {
                        let _removed = fs::remove_file(&temp);
                        return Err(error);
                    }
                    return Ok(temp);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempts < 8 => {
                    attempts += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn remove_files(&self, names: &[String]) {
        for name in names {
            // Una voce già sparita (altra istanza, pulizia a mano) va bene.
            let _removed = fs::remove_file(self.dir.join(name));
        }
    }
}

/// Vero per i file che la cache crea: voci e temporanei.
fn is_cache_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e == ENTRY_EXTENSION || e == TEMP_EXTENSION)
}

/// Carica l'indice dalla cartella: voci in ordine di data di modifica; i temporanei vecchi si
/// cancellano.
fn scan(dir: &Path) -> Index {
    let mut index = Index::default();
    let Ok(listing) = fs::read_dir(dir) else {
        return index;
    };
    let now = SystemTime::now();
    let mut found: Vec<(SystemTime, String, u64)> = Vec::new();
    for item in listing.flatten() {
        let path = item.path();
        let Ok(metadata) = item.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
        match path.extension().and_then(|e| e.to_str()) {
            Some(ENTRY_EXTENSION) => {
                if let Some(name) = item.file_name().to_str() {
                    found.push((modified, name.to_owned(), metadata.len()));
                }
            }
            Some(TEMP_EXTENSION) => {
                let stale = now
                    .duration_since(modified)
                    .is_ok_and(|age| age > STALE_TEMP);
                if stale {
                    let _removed = fs::remove_file(&path);
                }
            }
            _ => {}
        }
    }
    found.sort();
    for (_, name, bytes) in found {
        index.insert(name, bytes);
    }
    index
}

/// Il contenuto di una voce, se non supera la dimensione massima possibile; `Ok(None)` se il
/// file non c'è.
fn read_bounded(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    // Al più l'intestazione più il caso peggiore di lz4 sul massimo decompresso.
    let max = HEADER_BYTES + lz4_flex::block::get_maximum_output_size(MAX_ENTRY_BYTES);
    let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// Aggiorna la data di modifica di una voce letta.
fn touch_file(path: &Path) -> io::Result<()> {
    OpenOptions::new()
        .write(true)
        .open(path)?
        .set_modified(SystemTime::now())
}

/// Codifica gli eventi in una voce completa; `None` se oltre [`MAX_ENTRY_BYTES`].
fn encode_entry(key: &CacheKey, events: &[ExtractEvent]) -> Option<Vec<u8>> {
    let mut frames = Vec::new();
    for event in events {
        encode_frame(event, &mut frames).ok()?;
        if frames.len() > MAX_ENTRY_BYTES {
            return None;
        }
    }
    let compressed = lz4_flex::block::compress(&frames);
    let original_len = u32::try_from(frames.len()).ok()?;
    let compressed_len = u32::try_from(compressed.len()).ok()?;
    let crc = crc32fast::hash(&compressed);
    let mut entry = Vec::with_capacity(HEADER_BYTES + compressed.len());
    entry.extend_from_slice(&MAGIC);
    entry.extend_from_slice(&CACHE_FORMAT_VERSION.to_le_bytes());
    entry.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    entry.extend_from_slice(&key.0);
    entry.extend_from_slice(&original_len.to_le_bytes());
    entry.extend_from_slice(&compressed_len.to_le_bytes());
    entry.extend_from_slice(&crc.to_le_bytes());
    entry.extend_from_slice(&compressed);
    Some(entry)
}

/// Legge `N` byte da `bytes` a partire da `*at`, avanzando.
fn take<const N: usize>(bytes: &[u8], at: &mut usize) -> Result<[u8; N], Invalid> {
    let end = at.checked_add(N).ok_or(Invalid::Corrupt)?;
    let slice = bytes.get(*at..end).ok_or(Invalid::Corrupt)?;
    *at = end;
    slice.try_into().map_err(|_| Invalid::Corrupt)
}

/// Decodifica una voce e verifica che sia di `key`.
fn decode_entry(bytes: &[u8], key: &CacheKey) -> Result<Vec<ExtractEvent>, Invalid> {
    let mut at = 0;
    if take::<4>(bytes, &mut at)? != MAGIC
        || u16::from_le_bytes(take(bytes, &mut at)?) != CACHE_FORMAT_VERSION
        || u16::from_le_bytes(take(bytes, &mut at)?) != PROTOCOL_VERSION
        || take::<32>(bytes, &mut at)? != key.0
    {
        return Err(Invalid::Corrupt);
    }
    let original_len =
        usize::try_from(u32::from_le_bytes(take(bytes, &mut at)?)).map_err(|_| Invalid::Corrupt)?;
    let compressed_len =
        usize::try_from(u32::from_le_bytes(take(bytes, &mut at)?)).map_err(|_| Invalid::Corrupt)?;
    let crc = u32::from_le_bytes(take(bytes, &mut at)?);
    if original_len > MAX_ENTRY_BYTES {
        return Err(Invalid::Corrupt);
    }
    let compressed = bytes.get(at..).ok_or(Invalid::Corrupt)?;
    if compressed.len() != compressed_len || crc32fast::hash(compressed) != crc {
        return Err(Invalid::Corrupt);
    }
    let frames =
        lz4_flex::block::decompress(compressed, original_len).map_err(|_| Invalid::Corrupt)?;
    if frames.len() != original_len {
        return Err(Invalid::Corrupt);
    }
    let mut reader = FrameReader::new(frames.as_slice());
    let mut sequence = SequenceCheck::new();
    let mut events = Vec::new();
    while let Some(event) = reader
        .read::<ExtractEvent>()
        .map_err(|_| Invalid::Corrupt)?
    {
        sequence.accept(&event).map_err(|_| Invalid::Corrupt)?;
        events.push(event);
    }
    sequence.finish().map_err(|_| Invalid::Corrupt)?;
    Ok(events)
}

#[cfg(test)]
mod tests {
    use lynkeus_core::{Location, TextUnit};
    use lynkeus_extract::{Format, WarningKind};
    use proptest::prelude::*;

    use super::*;

    fn key(n: u8) -> CacheKey {
        CacheKey::new(
            Path::new("dati/relazione.docx"),
            u64::from(n),
            UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            &ExtractOptions::default(),
        )
    }

    fn events(text: &str) -> Vec<ExtractEvent> {
        vec![
            ExtractEvent::DocumentStart {
                inner: Vec::new(),
                format: Format::Docx,
            },
            ExtractEvent::Unit(TextUnit {
                location: Location::Paragraph(1),
                text: text.to_owned(),
            }),
            ExtractEvent::DocumentEnd,
            ExtractEvent::Warning {
                inner: vec!["voce.pdf".to_owned()],
                kind: WarningKind::NoText,
            },
        ]
    }

    fn cache(dir: &Path, max: ByteSize) -> TextCache {
        TextCache::new(dir.join("cache"), max)
    }

    #[test]
    fn put_then_get_round_trip() {
        let dir = tempfile::tempdir().expect("cartella");
        let cache = cache(dir.path(), DEFAULT_CACHE_SIZE);
        assert_eq!(cache.get(&key(1)), None);
        assert!(cache.put(&key(1), &events("carta")).is_ok());
        assert_eq!(cache.get(&key(1)), Some(events("carta")));
        assert_eq!(cache.get(&key(2)), None, "altra dimensione, altra chiave");
        assert!(cache.usage().bytes() > 0);
    }

    #[test]
    fn key_partitions() {
        let base = UNIX_EPOCH + Duration::from_secs(1_000);
        let options = ExtractOptions::default();
        let k = |path: &str, size: u64, modified: SystemTime, options: &ExtractOptions| {
            CacheKey::new(Path::new(path), size, modified, options)
        };
        let reference = k("a.pdf", 10, base, &options);
        assert_eq!(reference, k("a.pdf", 10, base, &options));
        assert_ne!(reference, k("b.pdf", 10, base, &options));
        assert_ne!(reference, k("a.pdf", 11, base, &options));
        assert_ne!(
            reference,
            k("a.pdf", 10, base + Duration::from_micros(1), &options)
        );
        let mut tighter = options;
        tighter.limits.max_depth = 1;
        assert_ne!(reference, k("a.pdf", 10, base, &tighter));
        // Il nome del file è solo esadecimale, qualunque sia il nome del documento.
        let name = k("strano\u{202E}?*:<>.pdf", 1, base, &options).file_name();
        assert!(
            name.trim_end_matches(".lkc")
                .chars()
                .all(|c| c.is_ascii_hexdigit()),
            "{name}"
        );
    }

    #[test]
    fn corrupt_entries_are_ignored_and_removed() {
        let dir = tempfile::tempdir().expect("cartella");
        let cache = cache(dir.path(), DEFAULT_CACHE_SIZE);
        assert!(cache.put(&key(1), &events("carta")).is_ok());
        let path = cache.dir().join(key(1).file_name());
        let good = fs::read(&path).expect("voce");

        let mut damaged: Vec<Vec<u8>> = Vec::new();
        // CRC sbagliato (un byte del contenuto cambiato).
        let mut flipped = good.clone();
        if let Some(last) = flipped.last_mut() {
            *last ^= 0xFF;
        }
        damaged.push(flipped);
        // Troncata.
        damaged.push(good[..good.len() - 3].to_vec());
        // Versione futura.
        let mut future = good.clone();
        future[4..6].copy_from_slice(&(CACHE_FORMAT_VERSION + 1).to_le_bytes());
        damaged.push(future);
        // Lunghezza decompressa dichiarata enorme.
        let mut huge = good.clone();
        huge[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        damaged.push(huge);
        // Voce di un'altra chiave sotto questo nome.
        let other = encode_entry(&key(2), &events("carta")).expect("voce");
        damaged.push(other);
        // Spazzatura.
        damaged.push(b"non una voce".to_vec());

        for (i, bytes) in damaged.into_iter().enumerate() {
            fs::write(&path, bytes).expect("scrittura");
            assert_eq!(cache.get(&key(1)), None, "caso {i}");
            assert!(!path.exists(), "caso {i}: voce non cancellata");
        }
    }

    #[test]
    fn entries_with_a_bad_event_sequence_are_corrupt() {
        let bad = vec![ExtractEvent::DocumentEnd];
        let entry = encode_entry(&key(1), &bad).expect("voce");
        assert_eq!(decode_entry(&entry, &key(1)), Err(Invalid::Corrupt));
    }

    #[test]
    fn least_recently_used_entries_are_evicted() {
        let dir = tempfile::tempdir().expect("cartella");
        let one = encode_entry(&key(1), &events("carta")).expect("voce").len();
        let max = ByteSize::from_bytes(u64::try_from(one * 2 + one / 2).expect("piccolo"));
        let cache = cache(dir.path(), max);
        assert!(cache.put(&key(1), &events("carta")).is_ok());
        assert!(cache.put(&key(2), &events("carta")).is_ok());
        // La 1 si usa, quindi la meno recente diventa la 2.
        assert!(cache.get(&key(1)).is_some());
        assert!(cache.put(&key(3), &events("carta")).is_ok());
        assert!(cache.get(&key(1)).is_some());
        assert_eq!(cache.get(&key(2)), None);
        assert!(cache.get(&key(3)).is_some());
        assert!(cache.usage() <= max);
    }

    #[test]
    fn an_entry_larger_than_the_cache_is_not_saved() {
        let dir = tempfile::tempdir().expect("cartella");
        let cache = cache(dir.path(), ByteSize::from_bytes(16));
        assert!(cache.put(&key(1), &events("carta")).is_ok());
        assert_eq!(cache.get(&key(1)), None);
        assert_eq!(cache.usage().bytes(), 0);
    }

    #[test]
    fn index_survives_a_new_instance_and_clear_empties_it() {
        let dir = tempfile::tempdir().expect("cartella");
        {
            let first = cache(dir.path(), DEFAULT_CACHE_SIZE);
            assert!(first.put(&key(1), &events("carta")).is_ok());
        }
        let second = cache(dir.path(), DEFAULT_CACHE_SIZE);
        let used = second.usage();
        assert!(used.bytes() > 0);
        assert!(second.get(&key(1)).is_some());
        assert_eq!(second.clear().ok(), Some(used));
        assert_eq!(second.get(&key(1)), None);
        assert_eq!(second.usage().bytes(), 0);
        // Svuotare una cache mai creata non è un errore.
        let never = TextCache::new(dir.path().join("mai"), DEFAULT_CACHE_SIZE);
        assert_eq!(never.clear().ok(), Some(ByteSize::from_bytes(0)));
    }

    #[test]
    fn shrinking_the_limit_evicts_at_once() {
        let dir = tempfile::tempdir().expect("cartella");
        let cache = cache(dir.path(), DEFAULT_CACHE_SIZE);
        assert!(cache.put(&key(1), &events("carta")).is_ok());
        assert!(cache.put(&key(2), &events("carta")).is_ok());
        cache.set_max_size(ByteSize::from_bytes(0));
        assert_eq!(cache.usage().bytes(), 0);
        assert_eq!(cache.get(&key(2)), None);
    }

    proptest! {
        /// Andata e ritorno di testi qualsiasi; byte qualsiasi non fanno mai panic.
        #[test]
        fn entries_round_trip(text in ".{0,300}", noise in proptest::collection::vec(any::<u8>(), 0..200)) {
            let entry = encode_entry(&key(1), &events(&text)).expect("voce");
            prop_assert_eq!(decode_entry(&entry, &key(1)), Ok(events(&text)));
            prop_assert!(decode_entry(&noise, &key(1)).is_err());
        }
    }
}
