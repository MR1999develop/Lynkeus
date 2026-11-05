// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Istanza unica per gli avvii con `--add` (clic destro): la prima istanza riceve i
//! percorsi delle successive, che escono subito (docs/threat-model.md, 4.8 A).
//!
//! - **Chi è la prima:** il lock su `instance.lock` nella cartella dati privata
//!   ([`start_or_deliver`]); il lock decide solo chi è il server.
//! - **Canale:** named pipe (Windows) o socket Unix in `$XDG_RUNTIME_DIR` (Linux), con un
//!   nome casuale nuovo a ogni avvio ([`crate::platform::InstanceListener`]).
//! - **Segreto:** nome del canale e token da 128 bit dal generatore del sistema stanno nel
//!   file privato `instance` ([`InstanceFile`]), creato nuovo a ogni avvio della prima
//!   istanza e cancellato all'uscita; mai in riga di comando, log o rapporti.
//! - **Messaggio** ([`encode_message`], [`read_message`]): intestazione con magia, versione,
//!   token e lunghezza; poi numero di percorsi e percorsi UTF-8. Tetti di
//!   [`MAX_MESSAGE_BYTES`] e [`MAX_PATHS`] controllati prima di allocare, lettura entro
//!   [`READ_TIMEOUT`], token confrontato in tempo costante. Token sbagliato: la connessione
//!   si chiude senza risposta.
//! - **Percorsi:** un solo guardiano per riga di comando e canale ([`accept_path`]): assoluti,
//!   senza NUL, non UNC né dispositivi, esistenti. Si aggiungono all'elenco dei luoghi senza
//!   avviare la ricerca e senza aprirli.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use lynkeus_engine::{Notify, PrivateFs};
use subtle::ConstantTimeEq;

use crate::platform::{self, InstanceListener, InstanceStopper};

/// Versione del messaggio e del file dell'istanza.
pub const PROTOCOL_VERSION: u16 = 1;
/// Byte massimi di un messaggio, intestazione compresa.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
/// Percorsi massimi in un messaggio.
pub const MAX_PATHS: usize = 256;
/// Caratteri massimi di un percorso (il massimo di Windows con il prefisso lungo).
pub const MAX_PATH_CHARS: usize = 32_767;
/// Tempo massimo per leggere un messaggio o la risposta.
pub const READ_TIMEOUT: Duration = Duration::from_secs(2);
/// Tempo massimo di un client per consegnare un messaggio: connessione, invio e conferma.
/// Il server serve una connessione alla volta e ne dedica al più due volte
/// [`READ_TIMEOUT`] a ciascuna (messaggio e chiusura): il client aspetta di più, così un
/// client lento davanti in coda non fa aprire una seconda finestra.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(5);
/// Tempo massimo che un client aspetta il file dell'istanza (Explorer avvia un processo per
/// elemento selezionato, quasi insieme: il primo può non averlo ancora scritto).
const INSTANCE_FILE_WAIT: Duration = Duration::from_secs(2);
/// Intervallo tra due letture del file dell'istanza mentre la si aspetta.
const INSTANCE_FILE_RETRY: Duration = Duration::from_millis(50);

/// Nome del file di lock nella cartella dati.
pub const LOCK_FILE: &str = "instance.lock";
/// Nome del file con nome del canale e token.
pub const INSTANCE_FILE: &str = "instance";

const MESSAGE_MAGIC: [u8; 4] = *b"LKSM";
const FILE_MAGIC: [u8; 4] = *b"LKSI";
/// Magia, versione, token, lunghezza del resto.
const HEADER_BYTES: usize = 4 + 2 + TOKEN_BYTES + 4;
const TOKEN_BYTES: usize = 16;
/// Risposta del server a un messaggio accettato.
const ACK: u8 = 1;
/// Byte massimi del file dell'istanza.
const MAX_INSTANCE_FILE_BYTES: u64 = 1024;
/// Errori di fila nell'attesa di un client oltre i quali il server si ferma (registrandolo):
/// un canale rotto non diventa un ciclo senza fine.
const MAX_ACCEPT_ERRORS: u32 = 20;
/// Pausa dopo un errore nell'attesa di un client.
const ACCEPT_RETRY_PAUSE: Duration = Duration::from_millis(100);
/// Tempo massimo per cui l'uscita aspetta il thread del server dopo lo stop: oltre una
/// connessione in corso ([`READ_TIMEOUT`] per il messaggio e per la chiusura) e l'intervallo
/// con cui l'attesa controlla lo stop.
const SERVER_JOIN_TIMEOUT: Duration = Duration::from_secs(6);
/// Messaggi ricevuti in attesa del thread dell'interfaccia; oltre si scartano.
const PENDING_MESSAGES: usize = 64;

/// Il segreto che autentica i client: 128 bit dal generatore del sistema. Non si stampa e
/// non ha `PartialEq`: si confronta solo in tempo costante (`Token::matches`).
#[derive(Clone)]
pub struct Token([u8; TOKEN_BYTES]);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(…)")
    }
}

impl Token {
    /// Un token nuovo dal generatore del sistema operativo.
    pub fn random() -> io::Result<Token> {
        let mut bytes = [0u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
        Ok(Token(bytes))
    }

    /// Vero se `bytes` è questo token; confronto in tempo costante.
    fn matches(&self, bytes: &[u8]) -> bool {
        bool::from(self.0.as_slice().ct_eq(bytes))
    }
}

/// Un suffisso casuale per il nome del canale (32 cifre esadecimali).
fn random_suffix() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Perché un messaggio non è stato accettato.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MessageError {
    /// Lettura non riuscita, troncata o oltre il tempo.
    #[error("messaggio non letto")]
    Io,
    /// Magia o versione diverse.
    #[error("versione del messaggio non riconosciuta")]
    Version,
    /// Token sbagliato.
    #[error("token non valido")]
    BadToken,
    /// Oltre [`MAX_MESSAGE_BYTES`].
    #[error("messaggio troppo grande")]
    TooLarge,
    /// Oltre [`MAX_PATHS`] percorsi.
    #[error("troppi percorsi")]
    TooManyPaths,
    /// Struttura non valida (lunghezze, UTF-8, byte in più).
    #[error("messaggio malformato")]
    Malformed,
}

/// Un capo del canale con letture e scritture a tempo limitato.
pub trait Channel {
    /// Legge esattamente `buffer.len()` byte entro `deadline`.
    fn read_exact(&mut self, buffer: &mut [u8], deadline: Instant) -> io::Result<()>;
    /// Scrive tutto `bytes` entro `deadline`.
    fn write_all(&mut self, bytes: &[u8], deadline: Instant) -> io::Result<()>;
}

/// I messaggi che portano `paths` (al più [`MAX_PATHS`] percorsi e [`MAX_MESSAGE_BYTES`]
/// byte ciascuno). I percorsi non rappresentabili in UTF-8 o troppo lunghi si saltano: il
/// server li rifiuterebbe comunque.
pub fn encode_messages(token: &Token, paths: &[PathBuf]) -> Vec<Vec<u8>> {
    let mut messages = Vec::new();
    let mut batch: Vec<&str> = Vec::new();
    let mut batch_bytes = HEADER_BYTES + 2;
    for path in paths {
        let Some(text) = path.to_str() else {
            continue;
        };
        if text.len() > usize::from(u16::MAX) || text.chars().count() > MAX_PATH_CHARS {
            continue;
        }
        let entry_bytes = 2 + text.len();
        if batch.len() == MAX_PATHS || batch_bytes + entry_bytes > MAX_MESSAGE_BYTES {
            messages.push(encode_message(token, &batch));
            batch.clear();
            batch_bytes = HEADER_BYTES + 2;
        }
        if batch_bytes + entry_bytes > MAX_MESSAGE_BYTES {
            continue;
        }
        batch.push(text);
        batch_bytes += entry_bytes;
    }
    if !batch.is_empty() {
        messages.push(encode_message(token, &batch));
    }
    messages
}

/// Un messaggio con `paths` (già entro i limiti).
fn encode_message(token: &Token, paths: &[&str]) -> Vec<u8> {
    let mut payload = Vec::new();
    let count = u16::try_from(paths.len()).unwrap_or(u16::MAX);
    payload.extend_from_slice(&count.to_le_bytes());
    for path in paths.iter().take(usize::from(count)) {
        let len = u16::try_from(path.len()).unwrap_or(u16::MAX);
        payload.extend_from_slice(&len.to_le_bytes());
        payload.extend_from_slice(path.as_bytes().get(..usize::from(len)).unwrap_or_default());
    }
    let mut message = Vec::with_capacity(HEADER_BYTES + payload.len());
    message.extend_from_slice(&MESSAGE_MAGIC);
    message.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    message.extend_from_slice(&token.0);
    let payload_len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    message.extend_from_slice(&payload_len.to_le_bytes());
    message.extend_from_slice(&payload);
    message
}

/// Legge un messaggio da `channel` entro [`READ_TIMEOUT`] e ne restituisce i percorsi
/// (ancora da validare con [`accept_path`]).
///
/// Ordine dei controlli: intestazione di lunghezza fissa, magia e versione, token (tempo
/// costante), lunghezza dichiarata contro il tetto, solo allora l'allocazione, poi numero
/// di percorsi contro il tetto e struttura.
pub fn read_message(
    channel: &mut dyn Channel,
    token: &Token,
) -> Result<Vec<PathBuf>, MessageError> {
    let deadline = Instant::now() + READ_TIMEOUT;
    let mut header = [0u8; HEADER_BYTES];
    channel
        .read_exact(&mut header, deadline)
        .map_err(|_| MessageError::Io)?;
    let (magic, rest) = header.split_at(4);
    let (version, rest) = rest.split_at(2);
    let (received_token, length) = rest.split_at(TOKEN_BYTES);
    if magic != MESSAGE_MAGIC || version != PROTOCOL_VERSION.to_le_bytes() {
        return Err(MessageError::Version);
    }
    if !token.matches(received_token) {
        return Err(MessageError::BadToken);
    }
    let length = u32::from_le_bytes(length.try_into().map_err(|_| MessageError::Malformed)?);
    let length = usize::try_from(length).map_err(|_| MessageError::TooLarge)?;
    if length > MAX_MESSAGE_BYTES - HEADER_BYTES {
        return Err(MessageError::TooLarge);
    }
    let mut payload = vec![0u8; length];
    channel
        .read_exact(&mut payload, deadline)
        .map_err(|_| MessageError::Io)?;
    parse_paths(&payload)
}

/// I percorsi del contenuto di un messaggio.
fn parse_paths(payload: &[u8]) -> Result<Vec<PathBuf>, MessageError> {
    let mut cursor = Cursor { rest: payload };
    let count = usize::from(cursor.u16()?);
    if count > MAX_PATHS {
        return Err(MessageError::TooManyPaths);
    }
    let mut paths = Vec::with_capacity(count);
    for _ in 0..count {
        let len = usize::from(cursor.u16()?);
        let bytes = cursor.take(len)?;
        let text = std::str::from_utf8(bytes).map_err(|_| MessageError::Malformed)?;
        paths.push(PathBuf::from(text));
    }
    if cursor.rest.is_empty() {
        Ok(paths)
    } else {
        Err(MessageError::Malformed)
    }
}

/// Lettore dei campi del contenuto.
struct Cursor<'a> {
    rest: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], MessageError> {
        if self.rest.len() < len {
            return Err(MessageError::Malformed);
        }
        let (taken, rest) = self.rest.split_at(len);
        self.rest = rest;
        Ok(taken)
    }

    fn u16(&mut self) -> Result<u16, MessageError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([
            bytes.first().copied().unwrap_or(0),
            bytes.get(1).copied().unwrap_or(0),
        ]))
    }
}

/// Perché un percorso ricevuto (riga di comando o canale) non si accetta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRefusal {
    /// Vuoto.
    Empty,
    /// Oltre [`MAX_PATH_CHARS`] caratteri.
    TooLong,
    /// Contiene un carattere NUL.
    Nul,
    /// Non assoluto.
    Relative,
    /// UNC, `\\?\`, `\\.\` o altro prefisso diverso da una lettera di unità: evita
    /// l'autenticazione verso un host scelto da altri e i nomi di dispositivo.
    NotLocalDrive,
    /// Non esiste (o non si può leggere).
    Missing,
}

/// La forma di `path`, senza accedere al disco.
pub fn check_path_shape(path: &Path) -> Result<(), PathRefusal> {
    let text = path.as_os_str().to_string_lossy();
    if text.is_empty() {
        return Err(PathRefusal::Empty);
    }
    if text.chars().count() > MAX_PATH_CHARS {
        return Err(PathRefusal::TooLong);
    }
    if text.contains('\0') {
        return Err(PathRefusal::Nul);
    }
    if let Some(Component::Prefix(prefix)) = path.components().next()
        && !matches!(prefix.kind(), Prefix::Disk(_))
    {
        return Err(PathRefusal::NotLocalDrive);
    }
    if !path.is_absolute() {
        return Err(PathRefusal::Relative);
    }
    Ok(())
}

/// Il guardiano unico dei percorsi aggiunti dall'esterno: forma valida
/// ([`check_path_shape`]), poi `.` e `..` risolti sul testo, poi esistenza
/// (`symlink_metadata`, senza aprire né seguire il percorso).
pub fn accept_path(path: &Path) -> Result<PathBuf, PathRefusal> {
    check_path_shape(path)?;
    let normalized = lexical_normalize(path);
    match std::fs::symlink_metadata(&normalized) {
        Ok(_) => Ok(normalized),
        Err(_) => Err(PathRefusal::Missing),
    }
}

/// Toglie `.` e applica `..` sul testo; `..` oltre la radice si ignora.
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

/// I percorsi accettati tra `paths`, nell'ordine; gli altri si scartano.
pub fn accept_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths.iter().filter_map(|p| accept_path(p).ok()).collect()
}

/// Serve una connessione: legge il messaggio, risponde con la conferma, aspetta che il
/// client chiuda (al più [`READ_TIMEOUT`]) e restituisce i percorsi accettati. Con un
/// messaggio rifiutato nessuna risposta e nessun percorso.
pub fn serve_connection(
    channel: &mut dyn Channel,
    token: &Token,
) -> Result<Vec<PathBuf>, MessageError> {
    let paths = read_message(channel, token)?;
    let accepted = accept_paths(&paths);
    let deadline = Instant::now() + READ_TIMEOUT;
    // Una risposta non consegnata non cambia nulla: i percorsi sono validi.
    if channel.write_all(&[ACK], deadline).is_ok() {
        let mut end = [0u8; 1];
        let _closed = channel.read_exact(&mut end, deadline);
    }
    Ok(accepted)
}

/// Il file privato con nome del canale e token.
#[derive(Debug, Clone)]
pub struct InstanceFile {
    /// Nome del canale (pipe o socket).
    pub endpoint: String,
    /// Il token.
    pub token: Token,
}

impl InstanceFile {
    /// I byte del file.
    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&FILE_MAGIC);
        bytes.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        bytes.extend_from_slice(&self.token.0);
        let len = u16::try_from(self.endpoint.len()).unwrap_or(u16::MAX);
        bytes.extend_from_slice(&len.to_le_bytes());
        bytes.extend_from_slice(self.endpoint.as_bytes());
        bytes
    }

    /// Il contenuto letto, se valido.
    fn decode(bytes: &[u8]) -> Option<InstanceFile> {
        let mut cursor = Cursor { rest: bytes };
        if cursor.take(4).ok()? != FILE_MAGIC {
            return None;
        }
        if cursor.u16().ok()? != PROTOCOL_VERSION {
            return None;
        }
        let token: [u8; TOKEN_BYTES] = cursor.take(TOKEN_BYTES).ok()?.try_into().ok()?;
        let len = usize::from(cursor.u16().ok()?);
        let endpoint = std::str::from_utf8(cursor.take(len).ok()?).ok()?.to_owned();
        (cursor.rest.is_empty() && !endpoint.is_empty()).then_some(InstanceFile {
            endpoint,
            token: Token(token),
        })
    }

    /// Lo scrive in `path`, nuovo e privato (un file vecchio si toglie prima).
    fn write(&self, path: &Path, private_fs: &dyn PrivateFs) -> io::Result<()> {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let mut file = private_fs.create_new_file(path)?;
        io::Write::write_all(&mut file, &self.encode())?;
        file.sync_all()
    }

    /// Lo legge da `path`, con un tetto alla dimensione.
    fn read(path: &Path) -> io::Result<InstanceFile> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_INSTANCE_FILE_BYTES)
            .read_to_end(&mut bytes)?;
        InstanceFile::decode(&bytes)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "file dell'istanza"))
    }
}

/// Com'è andato l'avvio con `--add`.
#[derive(Debug)]
pub enum Startup {
    /// Questa è la prima istanza: apre la finestra e riceve i percorsi delle altre.
    Server(InstanceServer),
    /// I percorsi sono stati consegnati alla prima istanza: questa esce.
    Delivered,
    /// Nessuna istanza unica (lock non disponibile, canale non creato, prima istanza che
    /// non risponde): questa apre la sua finestra.
    Alone,
}

/// Decide il ruolo di questo avvio con `--add`: prima istanza se ottiene il lock in
/// `data_dir` (cartella privata già creata), altrimenti client che consegna `paths` (già
/// accettati da [`accept_path`]). Ogni errore porta a [`Startup::Alone`]: nessun blocco,
/// nessun tentativo senza fine.
pub fn start_or_deliver(
    data_dir: &Path,
    private_fs: &Arc<dyn PrivateFs>,
    paths: &[PathBuf],
) -> Startup {
    let lock = match open_lock(&data_dir.join(LOCK_FILE)) {
        Ok(lock) => lock,
        Err(error) => {
            tracing::warn!(%error, "istanza unica: lock non aperto");
            return Startup::Alone;
        }
    };
    match lock.try_lock() {
        Ok(()) => match InstanceServer::start(lock, data_dir, private_fs) {
            Ok(server) => Startup::Server(server),
            Err(error) => {
                tracing::warn!(%error, "istanza unica: canale non creato");
                Startup::Alone
            }
        },
        Err(std::fs::TryLockError::WouldBlock) => {
            match deliver(&data_dir.join(INSTANCE_FILE), paths) {
                Ok(()) => Startup::Delivered,
                Err(error) => {
                    tracing::warn!(%error, "istanza unica: percorsi non consegnati");
                    Startup::Alone
                }
            }
        }
        Err(std::fs::TryLockError::Error(error)) => {
            tracing::warn!(%error, "istanza unica: lock non ottenuto");
            Startup::Alone
        }
    }
}

/// Apre (creandolo se manca) il file di lock nella cartella privata.
fn open_lock(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// Consegna `paths` alla prima istanza: legge il file dell'istanza (aspettandolo al più
/// [`INSTANCE_FILE_WAIT`]), si connette e manda i messaggi, ognuno confermato entro
/// [`DELIVERY_TIMEOUT`].
fn deliver(instance_file: &Path, paths: &[PathBuf]) -> io::Result<()> {
    let started = Instant::now();
    let instance = loop {
        match InstanceFile::read(instance_file) {
            Ok(instance) => break instance,
            Err(error) if started.elapsed() >= INSTANCE_FILE_WAIT => return Err(error),
            Err(_) => thread::sleep(INSTANCE_FILE_RETRY),
        }
    };
    for message in encode_messages(&instance.token, paths) {
        let deadline = Instant::now() + DELIVERY_TIMEOUT;
        let mut channel = platform::connect_instance(&instance.endpoint, deadline)?;
        channel.write_all(&message, deadline)?;
        let mut reply = [0u8; 1];
        channel.read_exact(&mut reply, deadline)?;
        if reply[0] != ACK {
            return Err(io::Error::other("risposta inattesa della prima istanza"));
        }
    }
    Ok(())
}

/// La prima istanza: thread che ascolta sul canale, una connessione alla volta, e passa al
/// thread dell'interfaccia i percorsi accettati. Fermato e atteso nel `Drop`; il file
/// dell'istanza si cancella e il lock si rilascia.
pub struct InstanceServer {
    instance_file: PathBuf,
    stopper: InstanceStopper,
    thread: Option<JoinHandle<()>>,
    // Si chiude (senza messaggi) quando il thread finisce: attesa con scadenza nel `Drop`.
    finished: Receiver<()>,
    received: Receiver<Vec<PathBuf>>,
    waker: Arc<Mutex<Option<Notify>>>,
    // Tenuto aperto (e bloccato) finché l'istanza vive.
    _lock: File,
}

impl std::fmt::Debug for InstanceServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstanceServer")
            .field("instance_file", &self.instance_file)
            .finish_non_exhaustive()
    }
}

impl InstanceServer {
    fn start(
        lock: File,
        data_dir: &Path,
        private_fs: &Arc<dyn PrivateFs>,
    ) -> io::Result<InstanceServer> {
        let token = Token::random()?;
        let listener = bind_new_endpoint()?;
        let instance = InstanceFile {
            endpoint: listener.endpoint().to_owned(),
            token: token.clone(),
        };
        let instance_file = data_dir.join(INSTANCE_FILE);
        instance.write(&instance_file, private_fs.as_ref())?;
        let stopper = listener.stopper();
        let (sender, received) = mpsc::sync_channel(PENDING_MESSAGES);
        let waker: Arc<Mutex<Option<Notify>>> = Arc::new(Mutex::new(None));
        let thread_waker = Arc::clone(&waker);
        let (finished_tx, finished) = mpsc::channel::<()>();
        let thread = thread::Builder::new()
            .name("istanza".to_owned())
            .spawn(move || {
                // Chiuso all'uscita dal thread, anche con un panic.
                let _finished = finished_tx;
                let end = listen(
                    &listener,
                    &token,
                    &sender,
                    &thread_waker,
                    ACCEPT_RETRY_PAUSE,
                );
                tracing::debug!(?end, "istanza unica: server fermo");
            })?;
        Ok(InstanceServer {
            instance_file,
            stopper,
            thread: Some(thread),
            finished,
            received,
            waker,
            _lock: lock,
        })
    }

    /// Chi avvisare (un nuovo frame) quando arrivano percorsi.
    pub fn set_waker(&self, waker: Notify) {
        *self.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }

    /// I percorsi arrivati da tutti i messaggi in attesa, senza attendere.
    pub fn take_paths(&self) -> Vec<PathBuf> {
        self.received.try_iter().flatten().collect()
    }
}

impl Drop for InstanceServer {
    /// Ferma il server e lo aspetta al più [`SERVER_JOIN_TIMEOUT`]: oltre, il thread si
    /// lascia andare (registrandolo) e l'uscita continua.
    fn drop(&mut self) {
        self.stopper.stop();
        if let Some(thread) = self.thread.take() {
            match self.finished.recv_timeout(SERVER_JOIN_TIMEOUT) {
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    tracing::warn!("istanza unica: il server non si è fermato in tempo");
                }
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if thread.join().is_err() {
                        tracing::error!("istanza unica: il server è terminato con un panic");
                    }
                }
            }
        }
        let _removed = std::fs::remove_file(&self.instance_file);
    }
}

/// Tentativi di creare il canale con un nome nuovo.
const BIND_ATTEMPTS: usize = 3;

/// Crea il canale con un nome casuale; se il nome è già occupato (oggetto creato da altri)
/// non lo si usa mai: si prova un altro nome, al più [`BIND_ATTEMPTS`] volte.
fn bind_new_endpoint() -> io::Result<InstanceListener> {
    let mut last_error = None;
    for _ in 0..BIND_ATTEMPTS {
        match InstanceListener::bind(&random_suffix()?) {
            Ok(listener) => return Ok(listener),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("canale non creato")))
}

/// Chi aspetta i client del server: il canale vero ([`InstanceListener`]) o, nei test, uno
/// finto.
trait Listener {
    /// Una connessione accettata.
    type Connection<'a>: Channel
    where
        Self: 'a;

    /// Aspetta un client; `None` dopo lo stop.
    fn accept(&self) -> io::Result<Option<Self::Connection<'_>>>;
}

impl Listener for InstanceListener {
    type Connection<'a> = platform::InstanceConnection<'a>;

    fn accept(&self) -> io::Result<Option<Self::Connection<'_>>> {
        InstanceListener::accept(self)
    }
}

/// Perché il ciclo del server è finito.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerEnd {
    /// Lo stop (uscita dell'app).
    Stopped,
    /// Il thread dell'interfaccia non riceve più.
    Disconnected,
    /// [`MAX_ACCEPT_ERRORS`] errori di fila nell'attesa di un client: canale rotto.
    TooManyErrors,
}

/// Il ciclo del server: una connessione alla volta finché non arriva lo stop. Un errore
/// nell'attesa di un client si registra e si riprova dopo `retry_pause`; solo dopo
/// [`MAX_ACCEPT_ERRORS`] errori di fila il server si ferma, registrandolo come errore (il
/// canale si chiude con il thread: i prossimi avvii con `--add` aprono la loro finestra).
fn listen<L: Listener>(
    listener: &L,
    token: &Token,
    sender: &SyncSender<Vec<PathBuf>>,
    waker: &Mutex<Option<Notify>>,
    retry_pause: Duration,
) -> ServerEnd {
    let mut errors_in_a_row = 0u32;
    loop {
        let mut connection = match listener.accept() {
            Ok(Some(connection)) => {
                errors_in_a_row = 0;
                connection
            }
            Ok(None) => return ServerEnd::Stopped,
            Err(error) => {
                errors_in_a_row += 1;
                if errors_in_a_row >= MAX_ACCEPT_ERRORS {
                    tracing::error!(%error, errors_in_a_row, "istanza unica: canale fermato");
                    return ServerEnd::TooManyErrors;
                }
                tracing::warn!(%error, errors_in_a_row, "istanza unica: attesa non riuscita");
                thread::sleep(retry_pause);
                continue;
            }
        };
        match serve_connection(&mut connection, token) {
            Ok(paths) if paths.is_empty() => {}
            Ok(paths) => {
                tracing::info!(count = paths.len(), "istanza unica: percorsi ricevuti");
                match sender.try_send(paths) {
                    Ok(()) => {
                        let waker = waker.lock().unwrap_or_else(PoisonError::into_inner);
                        if let Some(waker) = waker.as_ref() {
                            waker();
                        }
                    }
                    Err(TrySendError::Full(_)) => {
                        tracing::warn!("istanza unica: troppi messaggi in attesa, scartato");
                    }
                    Err(TrySendError::Disconnected(_)) => return ServerEnd::Disconnected,
                }
            }
            Err(error) => tracing::warn!(%error, "istanza unica: messaggio rifiutato"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Un canale in memoria: legge da `input`, registra ciò che si scrive.
    struct MemoryChannel {
        input: Vec<u8>,
        position: usize,
        written: Vec<u8>,
    }

    impl MemoryChannel {
        fn new(input: Vec<u8>) -> MemoryChannel {
            MemoryChannel {
                input,
                position: 0,
                written: Vec::new(),
            }
        }
    }

    impl Channel for MemoryChannel {
        fn read_exact(&mut self, buffer: &mut [u8], _deadline: Instant) -> io::Result<()> {
            let end = self.position + buffer.len();
            let source = self
                .input
                .get(self.position..end)
                .ok_or(io::ErrorKind::UnexpectedEof)?;
            buffer.copy_from_slice(source);
            self.position = end;
            Ok(())
        }

        fn write_all(&mut self, bytes: &[u8], _deadline: Instant) -> io::Result<()> {
            self.written.extend_from_slice(bytes);
            Ok(())
        }
    }

    fn token(byte: u8) -> Token {
        Token([byte; TOKEN_BYTES])
    }

    /// Stesso canale e stesso token (confronto in tempo costante).
    fn same_instance(a: &InstanceFile, b: &InstanceFile) -> bool {
        a.endpoint == b.endpoint && a.token.matches(&b.token.0)
    }

    #[test]
    fn tokens_match_only_themselves() {
        assert!(token(7).matches(&token(7).0));
        assert!(!token(7).matches(&token(8).0));
        assert!(!token(7).matches(&token(7).0[..TOKEN_BYTES - 1]));
        assert!(!token(7).matches(&[]));
    }

    fn existing_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("cartella");
        let path = std::path::absolute(dir.path()).expect("assoluto");
        (dir, path)
    }

    #[test]
    fn a_valid_message_adds_existing_paths_and_is_acknowledged() {
        let (_dir, existing) = existing_dir();
        let missing = existing.join("non-esiste");
        let messages = encode_messages(&token(7), &[existing.clone(), missing]);
        assert_eq!(messages.len(), 1);
        let mut channel = MemoryChannel::new(messages[0].clone());
        let accepted = serve_connection(&mut channel, &token(7)).expect("accettato");
        assert_eq!(accepted, [existing]);
        assert_eq!(channel.written, [ACK]);
    }

    #[test]
    fn a_wrong_token_is_refused_without_reply() {
        let (_dir, existing) = existing_dir();
        for wrong in [token(8), {
            let mut t = token(7);
            t.0[15] ^= 1;
            t
        }] {
            let message = encode_messages(&wrong, std::slice::from_ref(&existing));
            let mut channel = MemoryChannel::new(message[0].clone());
            assert_eq!(
                serve_connection(&mut channel, &token(7)),
                Err(MessageError::BadToken)
            );
            assert!(
                channel.written.is_empty(),
                "risposta a un client non autenticato"
            );
        }
    }

    #[test]
    fn a_message_without_token_is_refused() {
        let mut channel = MemoryChannel::new(b"LKSM\x01\x00".to_vec());
        assert_eq!(read_message(&mut channel, &token(7)), Err(MessageError::Io));
    }

    fn header(token: &Token, payload_len: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MESSAGE_MAGIC);
        bytes.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        bytes.extend_from_slice(&token.0);
        bytes.extend_from_slice(&payload_len.to_le_bytes());
        bytes
    }

    #[test]
    fn oversized_messages_are_refused_before_allocating() {
        for declared in [
            u32::MAX,
            u32::try_from(MAX_MESSAGE_BYTES).expect("u32"),
            u32::try_from(MAX_MESSAGE_BYTES - HEADER_BYTES + 1).expect("u32"),
        ] {
            let mut channel = MemoryChannel::new(header(&token(1), declared));
            assert_eq!(
                read_message(&mut channel, &token(1)),
                Err(MessageError::TooLarge),
                "{declared}"
            );
        }
        // Al limite esatto si legge (qui troncato: errore di lettura, non di dimensione).
        let at_limit = u32::try_from(MAX_MESSAGE_BYTES - HEADER_BYTES).expect("u32");
        let mut channel = MemoryChannel::new(header(&token(1), at_limit));
        assert_eq!(read_message(&mut channel, &token(1)), Err(MessageError::Io));
    }

    #[test]
    fn too_many_paths_are_refused() {
        let mut payload = Vec::new();
        let count = u16::try_from(MAX_PATHS + 1).expect("u16");
        payload.extend_from_slice(&count.to_le_bytes());
        for _ in 0..=MAX_PATHS {
            payload.extend_from_slice(&1u16.to_le_bytes());
            payload.push(b'/');
        }
        let mut message = header(&token(1), u32::try_from(payload.len()).expect("u32"));
        message.extend_from_slice(&payload);
        let mut channel = MemoryChannel::new(message);
        assert_eq!(
            read_message(&mut channel, &token(1)),
            Err(MessageError::TooManyPaths)
        );
    }

    #[test]
    fn malformed_payloads_are_refused() {
        for payload in [
            vec![],
            vec![1, 0],
            vec![1, 0, 5, 0, b'a'],
            vec![0, 0, 9],
            vec![1, 0, 2, 0, 0xFF, 0xFE],
        ] {
            let mut message = header(&token(1), u32::try_from(payload.len()).expect("u32"));
            message.extend_from_slice(&payload);
            let mut channel = MemoryChannel::new(message);
            assert_eq!(
                read_message(&mut channel, &token(1)),
                Err(MessageError::Malformed),
                "{payload:?}"
            );
        }
        let mut wrong_version = header(&token(1), 0);
        wrong_version[4] = 9;
        let mut channel = MemoryChannel::new(wrong_version);
        assert_eq!(
            read_message(&mut channel, &token(1)),
            Err(MessageError::Version)
        );
    }

    #[test]
    fn many_paths_are_split_into_messages_within_limits() {
        let paths: Vec<PathBuf> = (0..600).map(|i| PathBuf::from(format!("/d/{i}"))).collect();
        let messages = encode_messages(&token(3), &paths);
        assert_eq!(messages.len(), 3);
        let mut decoded = Vec::new();
        for message in messages {
            assert!(message.len() <= MAX_MESSAGE_BYTES);
            let mut channel = MemoryChannel::new(message);
            decoded.extend(read_message(&mut channel, &token(3)).expect("letto"));
        }
        assert_eq!(decoded, paths);
        // Percorsi lunghi: anche il tetto in byte divide i messaggi.
        let long: Vec<PathBuf> = (0..10)
            .map(|i| PathBuf::from(format!("/{i}{}", "x".repeat(20_000))))
            .collect();
        for message in encode_messages(&token(3), &long) {
            assert!(message.len() <= MAX_MESSAGE_BYTES);
        }
    }

    #[test]
    fn path_shape_partitions() {
        let (_dir, existing) = existing_dir();
        assert_eq!(check_path_shape(&existing), Ok(()));
        assert_eq!(check_path_shape(Path::new("")), Err(PathRefusal::Empty));
        assert_eq!(
            check_path_shape(Path::new("relativo/cartella")),
            Err(PathRefusal::Relative)
        );
        let with_nul = existing.join("a\0b");
        assert_eq!(check_path_shape(&with_nul), Err(PathRefusal::Nul));
        let long = existing.join("x".repeat(MAX_PATH_CHARS));
        assert_eq!(check_path_shape(&long), Err(PathRefusal::TooLong));
        if cfg!(windows) {
            for refused in [
                r"\\server\share\x",
                r"\\?\C:\Windows",
                r"\\.\PhysicalDrive0",
                r"\\?\UNC\server\share",
            ] {
                assert_eq!(
                    check_path_shape(Path::new(refused)),
                    Err(PathRefusal::NotLocalDrive),
                    "{refused}"
                );
            }
            assert_eq!(
                check_path_shape(Path::new(r"C:relativo")),
                Err(PathRefusal::Relative)
            );
        }
    }

    #[test]
    fn accepted_paths_exist_and_are_normalized() {
        let (_dir, existing) = existing_dir();
        let inner = existing.join("dentro");
        std::fs::create_dir(&inner).expect("cartella");
        let roundabout = inner.join("..").join(".").join("dentro");
        assert_eq!(accept_path(&roundabout), Ok(inner.clone()));
        assert_eq!(
            accept_path(&existing.join("manca")),
            Err(PathRefusal::Missing)
        );
        // Nomi strani ma validi: sono dati, non opzioni né comandi.
        for name in [
            "-r",
            "a b",
            "x & calc",
            "100%",
            "caret^",
            "punto;virgola",
            "città",
        ] {
            let path = existing.join(name);
            std::fs::create_dir(&path).expect("cartella");
            assert_eq!(accept_path(&path), Ok(path.clone()), "{name}");
        }
    }

    #[test]
    fn instance_file_round_trip_and_garbage() {
        let file = InstanceFile {
            endpoint: r"\\.\pipe\lynkeus-searcher-0123".to_owned(),
            token: token(9),
        };
        let decoded = InstanceFile::decode(&file.encode()).expect("decodificato");
        assert!(same_instance(&decoded, &file));
        let mut truncated = file.encode();
        truncated.pop();
        assert!(InstanceFile::decode(&truncated).is_none());
        assert!(InstanceFile::decode(b"LKSI").is_none());
        assert_eq!(format!("{:?}", file.token), "Token(…)");
    }

    #[test]
    fn instance_file_is_written_new_and_private() {
        let (_dir, base) = existing_dir();
        let fs: Arc<dyn PrivateFs> = platform::private_fs();
        fs.create_dir(&base.join("dati")).expect("cartella");
        let path = base.join("dati").join(INSTANCE_FILE);
        let file = InstanceFile {
            endpoint: "x".to_owned(),
            token: token(1),
        };
        file.write(&path, fs.as_ref()).expect("scritto");
        // Riscritto a ogni avvio della prima istanza.
        let newer = InstanceFile {
            endpoint: "y".to_owned(),
            token: token(2),
        };
        newer.write(&path, fs.as_ref()).expect("riscritto");
        assert!(same_instance(
            &InstanceFile::read(&path).expect("letto"),
            &newer
        ));
    }

    /// Il canale vero (pipe o socket): la seconda "istanza" consegna i percorsi alla prima,
    /// che accetta solo quelli esistenti; un client con il token sbagliato non riceve
    /// risposta e non aggiunge nulla; all'uscita il file dell'istanza sparisce.
    #[test]
    fn a_second_launch_delivers_to_the_first() {
        let (_dir, base) = existing_dir();
        let data = base.join("dati");
        let fs: Arc<dyn PrivateFs> = platform::private_fs();
        fs.create_dir(&data).expect("cartella dati");
        let server = match start_or_deliver(&data, &fs, &[]) {
            Startup::Server(server) => server,
            other => {
                // Linux senza `$XDG_RUNTIME_DIR` (alcune sessioni di CI): istanza unica spenta.
                assert!(
                    !cfg!(windows) && std::env::var_os("XDG_RUNTIME_DIR").is_none(),
                    "{other:?}"
                );
                return;
            }
        };
        let missing = base.join("manca");
        let delivered = start_or_deliver(&data, &fs, &[base.clone(), missing]);
        assert!(matches!(delivered, Startup::Delivered), "{delivered:?}");
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut received = Vec::new();
        while received.is_empty() && Instant::now() < deadline {
            received = server.take_paths();
            thread::yield_now();
        }
        assert_eq!(received, std::slice::from_ref(&base));

        // Token sbagliato sul canale vero: nessuna risposta, nessun percorso.
        let instance = InstanceFile::read(&data.join(INSTANCE_FILE)).expect("file");
        let mut wrong = instance.token.clone();
        wrong.0[0] ^= 0xFF;
        let deadline = Instant::now() + READ_TIMEOUT * 2;
        let mut channel =
            platform::connect_instance(&instance.endpoint, deadline).expect("connesso");
        let message = encode_messages(&wrong, std::slice::from_ref(&base)).remove(0);
        channel.write_all(&message, deadline).expect("scritto");
        let mut reply = [0u8; 1];
        assert!(
            channel.read_exact(&mut reply, deadline).is_err(),
            "risposta a un intruso"
        );
        drop(channel);
        assert!(server.take_paths().is_empty());

        drop(server);
        assert!(!data.join(INSTANCE_FILE).exists());
        // Senza prima istanza, il lock si ottiene di nuovo.
        assert!(matches!(
            start_or_deliver(&data, &fs, &[]),
            Startup::Server(_)
        ));
    }

    /// T-D08: un client che si connette e non scrive tiene il canale al più
    /// [`READ_TIMEOUT`]; la consegna del client successivo, avviata subito dopo, aspetta e
    /// riesce (nessuna seconda finestra).
    #[test]
    fn a_slow_client_does_not_block_the_next() {
        let (_dir, base) = existing_dir();
        let data = base.join("dati");
        let fs: Arc<dyn PrivateFs> = platform::private_fs();
        fs.create_dir(&data).expect("cartella dati");
        let server = match start_or_deliver(&data, &fs, &[]) {
            Startup::Server(server) => server,
            other => {
                // Linux senza `$XDG_RUNTIME_DIR`: istanza unica spenta.
                assert!(
                    !cfg!(windows) && std::env::var_os("XDG_RUNTIME_DIR").is_none(),
                    "{other:?}"
                );
                return;
            }
        };
        let instance = InstanceFile::read(&data.join(INSTANCE_FILE)).expect("file");
        let slow = platform::connect_instance(&instance.endpoint, Instant::now() + READ_TIMEOUT)
            .expect("client lento connesso");
        let delivered = start_or_deliver(&data, &fs, std::slice::from_ref(&base));
        assert!(matches!(delivered, Startup::Delivered), "{delivered:?}");
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut received = Vec::new();
        while received.is_empty() && Instant::now() < deadline {
            received = server.take_paths();
            thread::yield_now();
        }
        assert_eq!(received, std::slice::from_ref(&base));
        drop(slow);
    }

    /// Un canale finto: ogni attesa dà il prossimo esito della lista (dopo l'ultimo, lo
    /// stop), e conta le attese.
    struct ScriptedListener {
        outcomes: Mutex<std::collections::VecDeque<Option<Vec<u8>>>>,
        accepts: std::sync::atomic::AtomicU32,
    }

    impl ScriptedListener {
        /// `None` = errore nell'attesa; `Some(byte)` = un client che manda quei byte.
        fn new(outcomes: Vec<Option<Vec<u8>>>) -> ScriptedListener {
            ScriptedListener {
                outcomes: Mutex::new(outcomes.into()),
                accepts: std::sync::atomic::AtomicU32::new(0),
            }
        }

        fn accepts(&self) -> u32 {
            self.accepts.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    impl Listener for ScriptedListener {
        type Connection<'a> = MemoryChannel;

        fn accept(&self) -> io::Result<Option<MemoryChannel>> {
            self.accepts
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let next = self
                .outcomes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front();
            match next {
                None => Ok(None),
                Some(None) => Err(io::Error::other("pipe rotta")),
                Some(Some(bytes)) => Ok(Some(MemoryChannel::new(bytes))),
            }
        }
    }

    /// T-D09: venti errori di fila nell'attesa fermano il server, che lo dichiara; un
    /// client servito in mezzo azzera il conto.
    #[test]
    fn twenty_accept_errors_stop_the_server() {
        let (sender, received) = mpsc::sync_channel(4);
        let waker = Mutex::new(None);
        let errors = |count: u32| (0..count).map(|_| None).collect::<Vec<_>>();

        let broken = ScriptedListener::new(errors(MAX_ACCEPT_ERRORS + 5));
        let end = listen(&broken, &token(1), &sender, &waker, Duration::ZERO);
        assert_eq!(end, ServerEnd::TooManyErrors);
        assert_eq!(broken.accepts(), MAX_ACCEPT_ERRORS);

        let (_dir, existing) = existing_dir();
        let message = encode_messages(&token(1), std::slice::from_ref(&existing)).remove(0);
        let mut outcomes = errors(MAX_ACCEPT_ERRORS - 1);
        outcomes.push(Some(message));
        outcomes.extend(errors(MAX_ACCEPT_ERRORS - 1));
        let flaky = ScriptedListener::new(outcomes);
        let end = listen(&flaky, &token(1), &sender, &waker, Duration::ZERO);
        assert_eq!(end, ServerEnd::Stopped);
        assert_eq!(flaky.accepts(), 2 * MAX_ACCEPT_ERRORS);
        assert_eq!(received.try_recv().ok(), Some(vec![existing]));
    }

    proptest! {
        /// Byte qualsiasi: nessun panic e nessun percorso senza il token giusto.
        #[test]
        fn random_bytes_are_refused(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let mut channel = MemoryChannel::new(bytes);
            prop_assert!(read_message(&mut channel, &token(42)).is_err());
        }

        /// Un messaggio valido con un byte cambiato: nessun panic, e mai più percorsi di
        /// quelli mandati.
        #[test]
        fn tampered_messages_never_add_paths(index in 0usize..64, flip in 1u8..=255) {
            let paths = vec![PathBuf::from("/a/b"), PathBuf::from("/c")];
            let mut message = encode_messages(&token(5), &paths).remove(0);
            let len = message.len();
            if let Some(byte) = message.get_mut(index % len) {
                *byte ^= flip;
            }
            let mut channel = MemoryChannel::new(message);
            if let Ok(decoded) = read_message(&mut channel, &token(5)) {
                prop_assert!(decoded.len() <= paths.len());
            }
        }
    }
}
