// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Log in file e rapporti di crash, nelle cartelle private dell'app.
//!
//! - **Log** ([`init_logging`]): `tracing` in `logs/lynkeus-searcher.<data>.log`, un file al
//!   giorno, al più [`MAX_LOG_FILES`] (i più vecchi si cancellano); livello scelto nelle
//!   impostazioni e cambiato a runtime ([`LogControl::set_level`]). Solo motivo, percorso e
//!   contatori: mai testo dei documenti, token o nome del canale dell'istanza unica. Le
//!   librerie che scrivono con il crate `log` (eframe, winit, wgpu) finiscono nello stesso
//!   log, con lo stesso livello (`LogTracer` di `tracing-log`).
//! - **Rapporti di crash** ([`install_crash_reporter`]): un panic scrive
//!   `crash-<secondi>-<pid>.txt` con versione, sistema, thread, posizione e messaggio ripulito
//!   da percorsi e testi tra virgolette ([`redact`]); al più [`MAX_CRASH_REPORTS`] file, i più
//!   vecchi cancellati.

use std::io::{self, Write};
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use lynkeus_engine::PrivateFs;
use tracing_log::LogTracer;
use tracing_log::log;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Registry, reload};

/// File di log conservati (uno al giorno).
pub const MAX_LOG_FILES: usize = 7;
/// Rapporti di crash conservati.
pub const MAX_CRASH_REPORTS: usize = 10;
/// Caratteri massimi del messaggio del panic nel rapporto.
const MAX_MESSAGE_CHARS: usize = 300;
/// Prefisso e suffisso dei nomi dei log e dei rapporti.
const LOG_PREFIX: &str = "lynkeus-searcher";
const LOG_SUFFIX: &str = "log";
const REPORT_PREFIX: &str = "crash-";
const REPORT_SUFFIX: &str = ".txt";

/// Livello del log, scelto nelle impostazioni.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LogLevel {
    /// Solo errori.
    Error,
    /// Errori e avvisi (predefinito).
    #[default]
    Warn,
    /// Anche le informazioni (avvio, impostazioni, percorsi ricevuti).
    Info,
    /// Anche i dettagli per la diagnosi.
    Debug,
}

impl LogLevel {
    /// Tutti, dal meno al più dettagliato.
    pub const ALL: [LogLevel; 4] = [
        LogLevel::Error,
        LogLevel::Warn,
        LogLevel::Info,
        LogLevel::Debug,
    ];

    /// Il nome nelle impostazioni.
    pub fn key(self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
        }
    }

    /// Il livello dal nome nelle impostazioni.
    pub fn from_key(key: &str) -> Option<LogLevel> {
        LogLevel::ALL.into_iter().find(|level| level.key() == key)
    }

    fn filter(self) -> LevelFilter {
        match self {
            LogLevel::Error => LevelFilter::ERROR,
            LogLevel::Warn => LevelFilter::WARN,
            LogLevel::Info => LevelFilter::INFO,
            LogLevel::Debug => LevelFilter::DEBUG,
        }
    }

    /// Lo stesso livello per il crate `log`: le librerie non preparano nemmeno i messaggi
    /// sotto il livello scelto.
    fn log_filter(self) -> log::LevelFilter {
        match self {
            LogLevel::Error => log::LevelFilter::Error,
            LogLevel::Warn => log::LevelFilter::Warn,
            LogLevel::Info => log::LevelFilter::Info,
            LogLevel::Debug => log::LevelFilter::Debug,
        }
    }
}

/// Cambia il livello del log a runtime.
#[derive(Debug, Clone)]
pub struct LogControl {
    handle: reload::Handle<LevelFilter, Registry>,
}

impl LogControl {
    /// Da qui in poi registra a `level`, anche i messaggi delle librerie che usano `log`.
    pub fn set_level(&self, level: LogLevel) {
        if let Err(error) = self.handle.modify(|filter| *filter = level.filter()) {
            tracing::warn!(%error, "livello del log non cambiato");
        }
        log::set_max_level(level.log_filter());
    }
}

/// Installa il log globale (da chiamare una volta, nel `main`, prima dei thread): in file
/// con rotazione in `dir` se c'è, altrimenti sullo standard error. `None` se un log era già
/// installato o la cartella non si può usare.
pub fn init_logging(dir: Option<&Path>, level: LogLevel) -> Option<LogControl> {
    let (filter, handle) = reload::Layer::new(level.filter());
    let installed = match dir {
        Some(dir) => {
            let appender = tracing_appender::rolling::Builder::new()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix(LOG_PREFIX)
                .filename_suffix(LOG_SUFFIX)
                .max_log_files(MAX_LOG_FILES)
                .build(dir)
                .ok()?;
            Registry::default()
                .with(filter)
                .with(tracing_subscriber::fmt::layer().with_writer(appender))
                .try_init()
        }
        None => Registry::default()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
            .try_init(),
    };
    installed.ok()?;
    forward_log_records(level);
    Some(LogControl { handle })
}

/// Fa passare a `tracing` i messaggi delle librerie che scrivono con il crate `log`
/// (eframe, winit, wgpu), fino a `level`; [`LogControl::set_level`] lo cambia poi con il
/// resto del log. Se un altro logger di `log` è già installato restano a lui, e lo si
/// registra.
fn forward_log_records(level: LogLevel) {
    if let Err(error) = LogTracer::builder()
        .with_max_level(level.log_filter())
        .init()
    {
        tracing::warn!(%error, "messaggi del crate log non inoltrati");
    }
}

/// Le informazioni di un panic, già senza dati personali.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashReport {
    /// Nome del thread.
    pub thread: String,
    /// File e riga, senza le cartelle locali.
    pub location: String,
    /// Messaggio ripulito ([`redact`]).
    pub message: String,
}

impl CrashReport {
    /// Il rapporto del panic `info` nel thread corrente.
    pub fn from_panic(info: &PanicHookInfo<'_>) -> CrashReport {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let location = info.location().map_or_else(String::new, |location| {
            format!(
                "{}:{}:{}",
                source_file(location.file()),
                location.line(),
                location.column()
            )
        });
        CrashReport {
            thread: std::thread::current()
                .name()
                .map_or_else(|| "senza nome".to_owned(), redact),
            location,
            message: redact(&message),
        }
    }

    /// Il testo del file, scritto all'istante `unix_secs`.
    pub fn text(&self, unix_secs: u64) -> String {
        format!(
            "Lynkeus Searcher: rapporto di crash\n\
             versione: {}\n\
             sistema: {} {}\n\
             istante (secondi dal 1970, UTC): {unix_secs}\n\
             thread: {}\n\
             posizione: {}\n\
             messaggio: {}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            self.thread,
            self.location,
            self.message,
        )
    }
}

/// Il file sorgente senza le cartelle locali: i percorsi relativi del workspace restano, di
/// un percorso assoluto (dipendenze nella cartella dell'utente) solo le ultime due parti.
fn source_file(file: &str) -> String {
    let path = Path::new(file);
    if !path.is_absolute() && !file.starts_with(['/', '\\']) {
        return file.replace('\\', "/");
    }
    let parts: Vec<String> = path
        .components()
        .rev()
        .take(2)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let tail: Vec<&str> = parts.iter().rev().map(String::as_str).collect();
    format!("…/{}", tail.join("/"))
}

/// Il messaggio di un panic senza dati personali: i testi tra virgolette diventano `"…"`;
/// dalla prima parola che sembra un percorso o un nome di file ([`looks_like_path`],
/// [`looks_like_file_name`]) fino alla fine della riga tutto diventa `<percorso>` (un
/// percorso può contenere spazi: `C:\Users\Nome Cognome\x`, e dopo il primo spazio non si
/// sa dove finisce). Le righe si uniscono con uno spazio; al più [`MAX_MESSAGE_CHARS`]
/// caratteri.
pub fn redact(message: &str) -> String {
    let unquoted = mask_quoted(message);
    let lines: Vec<String> = unquoted
        .lines()
        .map(mask_from_first_path)
        .filter(|line| !line.is_empty())
        .collect();
    let joined = lines.join(" ");
    if joined.chars().count() > MAX_MESSAGE_CHARS {
        let mut cut: String = joined.chars().take(MAX_MESSAGE_CHARS).collect();
        cut.push('…');
        cut
    } else {
        joined
    }
}

/// `message` con il contenuto dei testi tra `"` o tra `` ` `` sostituito da `…` (anche un
/// testo aperto e mai chiuso).
fn mask_quoted(message: &str) -> String {
    let mut unquoted = String::with_capacity(message.len());
    let mut quote: Option<char> = None;
    for c in message.chars() {
        match quote {
            Some(open) if c == open => {
                unquoted.push('…');
                unquoted.push(c);
                quote = None;
            }
            Some(_) => {}
            None if c == '"' || c == '`' => {
                unquoted.push(c);
                quote = Some(c);
            }
            None => unquoted.push(c),
        }
    }
    if quote.is_some() {
        unquoted.push('…');
    }
    unquoted
}

/// Le parole di `line` fino alla prima che sembra un percorso o un nome di file, poi
/// `<percorso>` al posto di tutto il resto della riga.
fn mask_from_first_path(line: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for word in line.split_whitespace() {
        if looks_like_path(word) || looks_like_file_name(word) {
            kept.push("<percorso>");
            break;
        }
        kept.push(word);
    }
    kept.join(" ")
}

/// Vero per una parola con un separatore di percorso, `~` iniziale o una lettera di unità.
fn looks_like_path(word: &str) -> bool {
    let bytes = word.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    word.contains(['/', '\\']) || word.starts_with('~') || drive
}

/// Vero per una parola come `bilancio.xlsx` o `note.txt:`: un nome, un punto e
/// un'estensione da 1 a 5 lettere o cifre con almeno una lettera (non `1.5`).
fn looks_like_file_name(word: &str) -> bool {
    let trimmed = word.trim_end_matches([':', ',', ';', ')', ']', '}', '.']);
    let Some((name, extension)) = trimmed.rsplit_once('.') else {
        return false;
    };
    !name.is_empty()
        && (1..=5).contains(&extension.len())
        && extension.chars().all(|c| c.is_ascii_alphanumeric())
        && extension.chars().any(|c| c.is_ascii_alphabetic())
}

/// Scrive `report` in un file nuovo e privato di `dir` (creata se manca), dopo aver tolto i
/// rapporti più vecchi oltre [`MAX_CRASH_REPORTS`] − 1. Restituisce il percorso.
pub fn write_crash_report(
    dir: &Path,
    private_fs: &dyn PrivateFs,
    report: &CrashReport,
    now: SystemTime,
) -> io::Result<PathBuf> {
    private_fs.create_dir(dir)?;
    prune_reports(dir, MAX_CRASH_REPORTS.saturating_sub(1));
    let secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let pid = std::process::id();
    for attempt in 0..16u32 {
        let name = format!("{REPORT_PREFIX}{secs:020}-{pid}-{attempt}{REPORT_SUFFIX}");
        let path = dir.join(name);
        match private_fs.create_new_file(&path) {
            Ok(mut file) => {
                file.write_all(report.text(secs).as_bytes())?;
                file.sync_all()?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::from(io::ErrorKind::AlreadyExists))
}

/// I rapporti in `dir`, dal più vecchio (il nome comincia con l'istante a cifre fisse).
fn reports(dir: &Path) -> Vec<PathBuf> {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = listing
        .flatten()
        .filter(|item| item.file_type().is_ok_and(|t| t.is_file()))
        .map(|item| item.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(REPORT_PREFIX) && n.ends_with(REPORT_SUFFIX))
        })
        .collect();
    found.sort();
    found
}

/// Cancella i rapporti più vecchi finché ne restano al più `keep`.
fn prune_reports(dir: &Path, keep: usize) {
    let found = reports(dir);
    let excess = found.len().saturating_sub(keep);
    for old in found.iter().take(excess) {
        let _removed = std::fs::remove_file(old);
    }
}

/// Installa il gestore dei panic (nel `main`, prima dei thread): scrive un rapporto in
/// `dir`, lo registra nel log, poi chiama il gestore precedente.
pub fn install_crash_reporter(dir: PathBuf, private_fs: Arc<dyn PrivateFs>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let report = CrashReport::from_panic(info);
        match write_crash_report(&dir, private_fs.as_ref(), &report, SystemTime::now()) {
            Ok(_) => tracing::error!(
                thread = %report.thread,
                location = %report.location,
                "panic: rapporto di crash scritto"
            ),
            Err(error) => tracing::error!(%error, "panic: rapporto di crash non scritto"),
        }
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::platform;

    #[test]
    fn redaction_partitions() {
        assert_eq!(redact("indice 5 oltre la fine"), "indice 5 oltre la fine");
        assert_eq!(
            redact(r"file C:\Users\utente\note.txt non trovato"),
            "file <percorso>"
        );
        assert_eq!(redact("in /home/utente/x: errore"), "in <percorso>");
        assert_eq!(redact("~/segreti"), "<percorso>");
        assert_eq!(
            redact(r#"Corrupt { reason: "testo privato del documento" }"#),
            r#"Corrupt { reason: "…" }"#
        );
        assert_eq!(redact("aperto \"senza fine"), "aperto \"…");
        assert_eq!(redact("versione 1.5 attesa"), "versione 1.5 attesa");
        let long = "parola ".repeat(200);
        assert!(redact(&long).chars().count() <= MAX_MESSAGE_CHARS + 1);
    }

    /// Un percorso con spazi non lascia parti del nome dopo il primo spazio; un nome di file
    /// senza virgolette né cartelle non resta; la riga successiva si legge di nuovo.
    #[test]
    fn paths_with_spaces_and_bare_file_names_are_masked() {
        let redacted = redact(r"C:\Users\Mario Rossi Bianchi\x");
        assert_eq!(redacted, "<percorso>");
        for name in ["Rossi", "Bianchi"] {
            assert!(!redacted.contains(name), "{redacted}");
        }
        assert_eq!(
            redact("impossibile aprire Rossi.xlsx adesso"),
            "impossibile aprire <percorso>"
        );
        assert_eq!(
            redact("errore in note.txt: riga 3\nseconda riga senza dati"),
            "errore in <percorso> seconda riga senza dati"
        );
        assert_eq!(
            redact("cartella /home/mario rossi/documenti mancante"),
            "cartella <percorso>"
        );
    }

    #[test]
    fn source_files_lose_local_folders() {
        assert_eq!(
            source_file(r"crates\lynkeus-app\src\app.rs"),
            "crates/lynkeus-app/src/app.rs"
        );
        let absolute = if cfg!(windows) {
            r"C:\Users\utente\.cargo\registry\src\x\egui\src\ui.rs"
        } else {
            "/home/utente/.cargo/registry/src/x/egui/src/ui.rs"
        };
        assert_eq!(source_file(absolute), "…/src/ui.rs");
    }

    /// Un buffer condiviso in cui il log di prova scrive.
    #[derive(Clone, Default)]
    struct Captured(Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .map_err(|_| io::Error::other("buffer avvelenato"))?
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Captured {
        fn text(&self) -> String {
            let bytes = self.0.lock().map(|b| b.clone()).unwrap_or_default();
            String::from_utf8_lossy(&bytes).into_owned()
        }
    }

    /// Gli avvisi di una libreria che scrive con il crate `log` arrivano nel log dell'app;
    /// i messaggi sotto il livello scelto no.
    #[test]
    fn log_crate_records_reach_the_log() {
        forward_log_records(LogLevel::Warn);
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = Registry::default()
            .with(tracing_subscriber::fmt::layer().with_writer(move || writer.clone()));
        tracing::subscriber::with_default(subscriber, || {
            log::warn!(target: "wgpu_core", "superficie persa");
            log::debug!(target: "winit", "evento della finestra");
        });
        let text = captured.text();
        assert!(text.contains("superficie persa"), "{text}");
        assert!(!text.contains("evento della finestra"), "{text}");
    }

    #[test]
    fn levels_round_trip() {
        for level in LogLevel::ALL {
            assert_eq!(LogLevel::from_key(level.key()), Some(level));
        }
        assert_eq!(LogLevel::from_key("trace"), None);
    }

    fn report() -> CrashReport {
        CrashReport {
            thread: "main".to_owned(),
            location: "crates/lynkeus-app/src/app.rs:1:1".to_owned(),
            message: "prova".to_owned(),
        }
    }

    /// Al più dieci rapporti: i più vecchi si cancellano; i file sono nuovi e privati.
    #[test]
    fn at_most_ten_reports_are_kept() {
        let base = tempfile::tempdir().expect("cartella");
        let dir = base.path().join("crash-reports");
        let fs = platform::private_fs();
        let start = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut written = Vec::new();
        for i in 0..13 {
            let now = start + Duration::from_secs(i);
            written.push(write_crash_report(&dir, fs.as_ref(), &report(), now).expect("scritto"));
        }
        let left = reports(&dir);
        assert_eq!(left.len(), MAX_CRASH_REPORTS);
        assert_eq!(left, written[3..].to_vec());
        let text = std::fs::read_to_string(&left[0]).expect("letto");
        assert!(text.contains("messaggio: prova"), "{text}");
        assert!(text.contains(&format!("versione: {}", env!("CARGO_PKG_VERSION"))));
    }
}
