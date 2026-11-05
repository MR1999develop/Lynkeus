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
//! lunghezze (prima di allocare), CRC, decompressione esatta, frame e sequenza degli eventi;
//! solo dopo gli eventi partono verso chi legge, uno alla volta (seconda decodifica dei
//! frame), senza tenerli tutti in memoria.
//!
//! **Scrittura**: file temporaneo creato nuovo (mai sopra un file esistente) nella stessa
//! cartella, poi rinomina sopra la voce. Le scritture fallite si contano, con il primo
//! errore ([`TextCache::write_failures`]): la cache non mostra messaggi, l'app li registra.
//!
//! **Pulizia**: LRU sul totale dei byte delle voci; la data di modifica di una voce letta si
//! aggiorna, così l'ordine sopravvive al riavvio e vale tra istanze del programma. Una voce
//! che da sola supera il limite non si salva. L'indice in memoria si rilegge dalla cartella
//! ogni `RESCAN_PUTS` scritture e a ogni cambio di limite, così le voci scritte da un'altra
//! istanza contano nel limite.
//!
//! L'I/O (elenco della cartella, cancellazioni) avviene sempre fuori dal lucchetto
//! dell'indice.

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lynkeus_core::ByteSize;
use lynkeus_extract::protocol::{FrameReader, PROTOCOL_VERSION, encode_frame};
use lynkeus_extract::{EventSink, ExtractEvent, ExtractOptions, SequenceCheck, Stopped};

use crate::platform::PrivateFs;

/// Versione del formato delle voci.
pub const CACHE_FORMAT_VERSION: u16 = 1;

/// Limite predefinito della cache.
pub const DEFAULT_CACHE_SIZE: ByteSize = ByteSize::from_mib(500);

/// Byte massimi dei frame di una voce, decompressi: il testo di un documento più grande non
/// si salva (si rilegge ogni volta).
pub const MAX_ENTRY_BYTES: usize = 64 * 1024 * 1024;

/// Ogni quante scritture l'indice si rilegge dalla cartella.
const RESCAN_PUTS: u64 = 64;

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

/// Esito della lettura di una voce verso un destinatario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CacheRead {
    /// La voce non c'è o non vale (e si è cancellata): nessun evento mandato.
    Missing,
    /// Tutti gli eventi mandati.
    Delivered,
    /// Il destinatario ha chiesto di fermarsi.
    Stopped,
}

/// Le scritture fallite: quante e la prima.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteFailures {
    /// Quante, almeno una.
    pub count: u64,
    /// La prima: tipo e testo dell'errore del sistema.
    pub first: WriteFailure,
}

/// Una scrittura fallita.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteFailure {
    /// Il tipo di errore.
    pub kind: io::ErrorKind,
    /// Il testo dell'errore del sistema (la cartella è quella della cache).
    pub message: String,
}

/// Una voce nell'indice in memoria.
#[derive(Debug, Clone, Copy)]
struct IndexEntry {
    bytes: u64,
    /// Ordine d'uso: più alto = usata più di recente.
    used: u64,
}

/// L'indice delle voci, con l'ordine d'uso già ordinato: sfrattare non riordina tutto.
#[derive(Debug, Default)]
struct Index {
    entries: HashMap<String, IndexEntry>,
    by_use: BTreeMap<u64, String>,
    total: u64,
    clock: u64,
    /// Scritture dall'ultima lettura della cartella.
    puts_since_scan: u64,
}

impl Index {
    fn touch(&mut self, name: &str) {
        self.clock += 1;
        if let Some(entry) = self.entries.get_mut(name) {
            if let Some(owner) = self.by_use.remove(&entry.used) {
                self.by_use.insert(self.clock, owner);
            }
            entry.used = self.clock;
        }
    }

    fn insert(&mut self, name: String, bytes: u64) {
        self.clock += 1;
        let entry = IndexEntry {
            bytes,
            used: self.clock,
        };
        if let Some(old) = self.entries.insert(name.clone(), entry) {
            self.by_use.remove(&old.used);
            self.total = self.total.saturating_sub(old.bytes);
        }
        self.by_use.insert(self.clock, name);
        self.total = self.total.saturating_add(bytes);
    }

    fn remove(&mut self, name: &str) {
        if let Some(old) = self.entries.remove(name) {
            self.by_use.remove(&old.used);
            self.total = self.total.saturating_sub(old.bytes);
        }
    }

    /// Toglie dall'indice le voci meno usate finché il totale sta in `max`, tranne `keep`;
    /// restituisce i nomi da cancellare.
    fn evict(&mut self, max: u64, keep: Option<&str>) -> Vec<String> {
        let mut victims = Vec::new();
        while self.total > max {
            let oldest = self
                .by_use
                .values()
                .find(|name| Some(name.as_str()) != keep)
                .cloned();
            let Some(name) = oldest else {
                break;
            };
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
    private_fs: Arc<dyn PrivateFs>,
    failed_writes: AtomicU64,
    first_failure: OnceLock<WriteFailure>,
}

/// Contatore per i nomi dei temporanei del processo.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl TextCache {
    /// Una cache nella cartella `dir` (creata al primo salvataggio con `private_fs`, privata),
    /// con limite `max`. Non tocca il disco finché non serve. Su Windows `private_fs` è
    /// quello dell'app (descrittore esplicito); con [`RefusingFs`](crate::RefusingFs) la
    /// cache non scrive nulla.
    pub fn at(dir: PathBuf, max: ByteSize, private_fs: Arc<dyn PrivateFs>) -> TextCache {
        TextCache {
            dir,
            max_bytes: AtomicU64::new(max.bytes()),
            index: Mutex::new(None),
            private_fs,
            failed_writes: AtomicU64::new(0),
            first_failure: OnceLock::new(),
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

    /// Cambia il limite, senza toccare il disco (dal thread dell'interfaccia, nell'ordine
    /// delle scelte dell'utente); le voci in più le toglie [`TextCache::evict_to_limit`] o la
    /// prossima scrittura.
    pub fn set_limit(&self, max: ByteSize) {
        self.max_bytes.store(max.bytes(), Ordering::Relaxed);
    }

    /// Rilegge la cartella (anche le voci di altre istanze) e toglie le voci meno usate
    /// finché il totale sta nel limite attuale. Fa I/O: da un thread di lavoro.
    pub fn evict_to_limit(&self) {
        let fresh = scan(&self.dir);
        let victims = {
            let mut guard = self.lock_index();
            guard.insert(fresh).evict(self.max_bytes(), None)
        };
        self.remove_files(&victims);
    }

    /// [`TextCache::set_limit`] più [`TextCache::evict_to_limit`].
    pub fn set_max_size(&self, max: ByteSize) {
        self.set_limit(max);
        self.evict_to_limit();
    }

    /// Byte occupati dalle voci (dopo il primo uso, l'indice in memoria).
    pub fn usage(&self) -> ByteSize {
        ByteSize::from_bytes(self.with_index(|index| index.total))
    }

    /// Le scritture fallite finora, con la prima; `None` se nessuna.
    pub fn write_failures(&self) -> Option<WriteFailures> {
        let count = self.failed_writes.load(Ordering::Relaxed);
        let first = self.first_failure.get()?.clone();
        (count > 0).then_some(WriteFailures { count, first })
    }

    /// Gli eventi salvati per `key`, se la voce c'è ed è valida. Una voce non valida si
    /// cancella.
    pub fn get(&self, key: &CacheKey) -> Option<Vec<ExtractEvent>> {
        let mut events = Vec::new();
        match self.read(key, &mut events) {
            CacheRead::Delivered => Some(events),
            CacheRead::Missing | CacheRead::Stopped => None,
        }
    }

    /// Manda a `sink` gli eventi salvati per `key`, uno alla volta, dopo aver verificato
    /// tutta la voce: o tutti, o nessuno (salvo che `sink` si fermi). Una voce non valida si
    /// cancella.
    pub(crate) fn read(&self, key: &CacheKey, sink: &mut dyn EventSink) -> CacheRead {
        let name = key.file_name();
        let path = self.dir.join(&name);
        // Assente o illeggibile ora: nessuna conclusione sulla voce.
        let Ok(Some(bytes)) = read_bounded(&path) else {
            return CacheRead::Missing;
        };
        let checked = decompress_entry(bytes, key)
            .and_then(|frames| visit_frames(&frames, &mut |_| Ok(())).map(|_| frames));
        let frames = match checked {
            Ok(frames) => frames,
            Err(Invalid::Corrupt) => {
                if let Some(index) = self.lock_index().as_mut() {
                    index.remove(&name);
                }
                // Già sparita (altra istanza, pulizia) va bene.
                let _removed = fs::remove_file(&path);
                return CacheRead::Missing;
            }
        };
        if let Some(index) = self.lock_index().as_mut() {
            index.touch(&name);
        }
        // L'ordine LRU sopravvive al riavvio; se non riesce, poco male.
        let _touched = touch_file(&path);
        // Gli stessi frame appena verificati: la seconda decodifica non fallisce.
        match visit_frames(&frames, &mut |event| sink.emit(event)) {
            Ok(Ok(())) | Err(Invalid::Corrupt) => CacheRead::Delivered,
            Ok(Err(Stopped)) => CacheRead::Stopped,
        }
    }

    /// Salva gli eventi di `key`. Una voce più grande del limite (o di [`MAX_ENTRY_BYTES`]
    /// decompressa) non si salva e non è un errore. Un errore si conta anche in
    /// [`TextCache::write_failures`].
    pub fn put(&self, key: &CacheKey, events: &[ExtractEvent]) -> io::Result<()> {
        let saved = self.save(key, events);
        if let Err(error) = &saved {
            self.failed_writes.fetch_add(1, Ordering::Relaxed);
            let first = WriteFailure {
                kind: error.kind(),
                message: error.to_string(),
            };
            // Solo la prima resta: le altre sono contate.
            let _kept_first = self.first_failure.set(first);
        }
        saved
    }

    fn save(&self, key: &CacheKey, events: &[ExtractEvent]) -> io::Result<()> {
        let Some(entry) = encode_entry(key, events) else {
            return Ok(());
        };
        let entry_bytes = u64::try_from(entry.len()).unwrap_or(u64::MAX);
        let max = self.max_bytes();
        if entry_bytes > max {
            return Ok(());
        }
        // Prima l'indice (la prima lettura cancella anche i temporanei avanzati), poi il file.
        self.with_index(|_| ());
        self.private_fs.create_dir(&self.dir)?;
        let name = key.file_name();
        let target = self.dir.join(&name);
        let temp = self.write_temp(&name, &entry)?;
        if let Err(error) = fs::rename(&temp, &target) {
            let _removed = fs::remove_file(&temp);
            return Err(error);
        }
        let (victims, rescan) = self.with_index(|index| {
            index.insert(name.clone(), entry_bytes);
            index.puts_since_scan += 1;
            (
                index.evict(max, Some(&name)),
                index.puts_since_scan >= RESCAN_PUTS,
            )
        });
        self.remove_files(&victims);
        if rescan {
            self.evict_to_limit();
        }
        Ok(())
    }

    /// Cancella tutte le voci (e i temporanei); restituisce i byte liberati. L'elenco e le
    /// cancellazioni avvengono senza il lucchetto dell'indice, che alla fine si rilegge.
    pub fn clear(&self) -> io::Result<ByteSize> {
        let listing = match fs::read_dir(&self.dir) {
            Ok(listing) => listing,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                *self.lock_index() = Some(Index::default());
                return Ok(ByteSize::from_bytes(0));
            }
            Err(error) => return Err(error),
        };
        let mut freed = 0_u64;
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
        *self.lock_index() = None;
        match first_error {
            Some(error) => Err(error),
            None => Ok(ByteSize::from_bytes(freed)),
        }
    }

    fn max_bytes(&self) -> u64 {
        self.max_bytes.load(Ordering::Relaxed)
    }

    fn lock_index(&self) -> MutexGuard<'_, Option<Index>> {
        self.index.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Chiama `use_index` con l'indice; se non c'è ancora lo legge dalla cartella, fuori dal
    /// lucchetto.
    fn with_index<T>(&self, use_index: impl FnOnce(&mut Index) -> T) -> T {
        if self.lock_index().is_none() {
            let scanned = scan(&self.dir);
            let mut guard = self.lock_index();
            if guard.is_none() {
                *guard = Some(scanned);
            }
        }
        // Svuotata nel frattempo: l'indice vuoto vale fino alla prossima rilettura.
        use_index(self.lock_index().get_or_insert_with(Index::default))
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
            match self.private_fs.create_new_file(&temp) {
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

/// Verifica intestazione e CRC di una voce di `key` e ne restituisce i frame decompressi; i
/// byte letti si liberano prima di restituire.
fn decompress_entry(bytes: Vec<u8>, key: &CacheKey) -> Result<Vec<u8>, Invalid> {
    let mut at = 0;
    if take::<4>(&bytes, &mut at)? != MAGIC
        || u16::from_le_bytes(take(&bytes, &mut at)?) != CACHE_FORMAT_VERSION
        || u16::from_le_bytes(take(&bytes, &mut at)?) != PROTOCOL_VERSION
        || take::<32>(&bytes, &mut at)? != key.0
    {
        return Err(Invalid::Corrupt);
    }
    let original_len = usize::try_from(u32::from_le_bytes(take(&bytes, &mut at)?))
        .map_err(|_| Invalid::Corrupt)?;
    let compressed_len = usize::try_from(u32::from_le_bytes(take(&bytes, &mut at)?))
        .map_err(|_| Invalid::Corrupt)?;
    let crc = u32::from_le_bytes(take(&bytes, &mut at)?);
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
    // I byte letti non servono più: la memoria si libera prima di decodificare gli eventi.
    drop(bytes);
    Ok(frames)
}

/// Decodifica i frame uno alla volta, controlla la sequenza e passa ogni evento a `visit`.
/// `Err` se i frame non valgono; `Ok(Err(Stopped))` se `visit` si è fermato.
fn visit_frames(
    frames: &[u8],
    visit: &mut dyn FnMut(ExtractEvent) -> Result<(), Stopped>,
) -> Result<Result<(), Stopped>, Invalid> {
    let mut reader = FrameReader::new(frames);
    let mut sequence = SequenceCheck::new();
    while let Some(event) = reader
        .read::<ExtractEvent>()
        .map_err(|_| Invalid::Corrupt)?
    {
        sequence.accept(&event).map_err(|_| Invalid::Corrupt)?;
        if visit(event).is_err() {
            return Ok(Err(Stopped));
        }
    }
    sequence.finish().map_err(|_| Invalid::Corrupt)?;
    Ok(Ok(()))
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

    /// Cartelle e file normali: nei test i permessi non contano.
    #[derive(Debug)]
    struct PlainFs;

    impl PrivateFs for PlainFs {
        fn create_dir(&self, dir: &Path) -> io::Result<()> {
            fs::create_dir_all(dir)
        }

        fn create_new_file(&self, path: &Path) -> io::Result<File> {
            OpenOptions::new().write(true).create_new(true).open(path)
        }
    }

    fn cache(dir: &Path, max: ByteSize) -> TextCache {
        TextCache::at(dir.join("cache"), max, Arc::new(PlainFs))
    }

    /// Decodifica completa di una voce, come la lettura.
    fn decode_entry(entry: &[u8], key: &CacheKey) -> Result<Vec<ExtractEvent>, Invalid> {
        let frames = decompress_entry(entry.to_vec(), key)?;
        let mut events = Vec::new();
        match visit_frames(&frames, &mut |event| {
            events.push(event);
            Ok(())
        })? {
            Ok(()) => Ok(events),
            Err(Stopped) => Err(Invalid::Corrupt),
        }
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
        let never = cache(&dir.path().join("mai"), DEFAULT_CACHE_SIZE);
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

    /// Con [`RefusingFs`](crate::RefusingFs) (il default sicuro dove mancano le cartelle
    /// private dell'app) la cache non scrive nulla: nessuna cartella con i permessi
    /// ereditati dal padre; la scrittura rifiutata si conta.
    #[test]
    fn with_the_refusing_fs_nothing_is_written() {
        let dir = tempfile::tempdir().expect("cartella");
        let cache = TextCache::at(
            dir.path().join("cache"),
            DEFAULT_CACHE_SIZE,
            Arc::new(crate::RefusingFs),
        );
        assert!(cache.put(&key(1), &events("carta")).is_err());
        assert!(!dir.path().join("cache").exists());
        assert_eq!(cache.write_failures().map(|f| f.count), Some(1));
    }

    /// Una scrittura fallita (la cartella della cache è un file) si conta, con il primo
    /// errore; quelle dopo aumentano solo il conteggio.
    #[test]
    fn failed_cache_put_is_counted() {
        let dir = tempfile::tempdir().expect("cartella");
        fs::write(dir.path().join("cache"), b"non una cartella").expect("file");
        let cache = cache(dir.path(), DEFAULT_CACHE_SIZE);
        assert_eq!(cache.write_failures(), None);
        assert!(cache.put(&key(1), &events("carta")).is_err());
        let first = cache.write_failures().expect("errore contato");
        assert_eq!(first.count, 1);
        assert!(!first.first.message.is_empty());
        assert!(cache.put(&key(2), &events("carta")).is_err());
        let second = cache.write_failures().expect("errore contato");
        assert_eq!(second.count, 2);
        assert_eq!(second.first, first.first);
        // Una voce troppo grande per la cache non è un errore.
        let small = TextCache::at(
            dir.path().join("altra"),
            ByteSize::from_bytes(16),
            Arc::new(PlainFs),
        );
        assert!(small.put(&key(1), &events("carta")).is_ok());
        assert_eq!(small.write_failures(), None);
    }

    /// La lettura manda gli eventi uno alla volta e si ferma quando il destinatario lo
    /// chiede; la voce resta valida.
    #[test]
    fn reading_stops_when_the_sink_asks() {
        struct FirstOnly(Vec<ExtractEvent>);
        impl EventSink for FirstOnly {
            fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped> {
                self.0.push(event);
                Err(Stopped)
            }
        }
        let dir = tempfile::tempdir().expect("cartella");
        let cache = cache(dir.path(), DEFAULT_CACHE_SIZE);
        assert!(cache.put(&key(1), &events("carta")).is_ok());
        let mut first = FirstOnly(Vec::new());
        assert_eq!(cache.read(&key(1), &mut first), CacheRead::Stopped);
        assert_eq!(first.0.len(), 1);
        assert_eq!(cache.get(&key(1)), Some(events("carta")));
    }

    /// Le voci scritte da un'altra istanza contano nel limite dopo la rilettura; l'ordine
    /// d'uso dell'indice segue letture e scritture.
    #[test]
    fn rescans_see_entries_of_other_instances() {
        let dir = tempfile::tempdir().expect("cartella");
        let one = encode_entry(&key(1), &events("carta")).expect("voce").len();
        let max = ByteSize::from_bytes(u64::try_from(one * 2).expect("piccolo"));
        let first = cache(dir.path(), max);
        let second = cache(dir.path(), max);
        assert!(first.put(&key(1), &events("carta")).is_ok());
        assert!(first.put(&key(2), &events("carta")).is_ok());
        assert!(second.put(&key(3), &events("carta")).is_ok());
        assert_eq!(
            first.usage().bytes(),
            u64::try_from(one * 2).expect("piccolo")
        );
        first.evict_to_limit();
        assert!(first.usage() <= max);
        let on_disk = fs::read_dir(first.dir()).expect("cartella").count();
        assert_eq!(on_disk, 2);
        // Il limite si cambia senza I/O; lo sfratto lo applica.
        first.set_limit(ByteSize::from_bytes(0));
        assert_eq!(first.max_size(), ByteSize::from_bytes(0));
        first.evict_to_limit();
        assert_eq!(first.usage().bytes(), 0);
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
