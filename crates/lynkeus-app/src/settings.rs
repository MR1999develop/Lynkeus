// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Impostazioni dell'app ([`Settings`]) e loro file `settings.toml` ([`SettingsFile`]).
//!
//! Formato (versione [`FORMAT_VERSION`]), con chiavi stabili e valori come testo leggibile:
//!
//! ```toml
//! format_version = 1
//! language = "system"        # "system", "it", "en"
//! theme = "system"           # "system", "light", "dark"
//!
//! [search]                   # opzioni, radici e termini correnti e recenti
//! [filters]                  # estensioni, cartelle da saltare, nascosti, solo online, dimensione
//! [layout]                   # pannelli, anteprima, colonne, ordinamento
//! [export]                   # formato, righe, ultima cartella di salvataggio
//! [cache]                    # cache del testo dei documenti: accesa, dimensione massima
//! [integration]              # context_menu: voce "Cerca con Lynkeus" nel clic destro
//! [diagnostics]              # log_level: "error", "warn", "info", "debug"
//! ```
//!
//! Lettura tollerante: ogni campo si legge da solo, un valore mancante o non valido prende
//! il predefinito senza perdere gli altri. Le chiavi sconosciute (di versioni future o
//! scritte a mano) si conservano alla riscrittura. Un file di versione più recente non si
//! riscrive; un file illeggibile come TOML si sposta in `settings.toml.invalid` prima di
//! scriverne uno nuovo. Scrittura durevole: temporaneo nella stessa cartella, `sync_all`,
//! rinomina, flush della cartella dove serve.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use lynkeus_core::{AccentMode, CaseMode, Combine, MatchOptions, Query, Term, WildcardMode};
use lynkeus_engine::{Extension, Filters};
use toml::{Table, Value};

use crate::diagnostics::LogLevel;
use crate::export::{ExportFormat, ExportScope, ExportSettings};
use crate::i18n::Language;
use crate::platform;
use crate::results::{SortColumn, SortOrder, SortSpec};
use crate::theme::ThemeChoice;

/// Versione del formato scritta e letta.
pub const FORMAT_VERSION: i64 = 1;
/// Nome del file nella cartella di configurazione.
pub const FILE_NAME: &str = "settings.toml";
/// Voci massime negli elenchi dei recenti.
pub const MAX_RECENT: usize = 10;
/// Dimensione massima del file letto: oltre, non sono impostazioni nostre.
const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Elementi massimi letti da un elenco e byte massimi di una stringa.
const MAX_LIST_ITEMS: usize = 256;
const MAX_STRING_BYTES: usize = 4096;
/// Limiti della dimensione massima dei file, in MiB.
pub const MAX_FILE_SIZE_MIB_RANGE: std::ops::RangeInclusive<u64> = 1..=1_048_576;
/// Limiti della dimensione della cache dei documenti, in MiB.
pub const CACHE_SIZE_MIB_RANGE: std::ops::RangeInclusive<u64> = 16..=65_536;

/// La cache del testo estratto dai documenti.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheSettings {
    /// Accesa (predefinito): la seconda ricerca negli stessi documenti non li rilegge.
    pub enabled: bool,
    /// Dimensione massima, in MiB, in [`CACHE_SIZE_MIB_RANGE`].
    pub max_size_mib: u64,
}

impl CacheSettings {
    /// Dimensioni proposte nel menu, in MiB.
    pub const SIZE_CHOICES_MIB: [u64; 5] = [100, 250, 500, 1000, 2000];
}

impl Default for CacheSettings {
    fn default() -> Self {
        CacheSettings {
            enabled: true,
            max_size_mib: 500,
        }
    }
}

/// Quali estensioni si filtrano.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExtensionMode {
    /// Tutti i file (predefinito).
    #[default]
    All,
    /// Solo le estensioni elencate.
    Include,
    /// Tutte tranne quelle elencate.
    Exclude,
}

/// Filtri scelti dall'utente. L'elenco delle estensioni e la dimensione restano anche
/// quando il filtro è spento, per riaccenderlo senza riscriverli.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterSettings {
    /// Modalità del filtro sulle estensioni.
    pub extension_mode: ExtensionMode,
    /// Estensioni elencate.
    pub extensions: Vec<Extension>,
    /// Nomi di cartelle da saltare.
    pub skip_dirs: Vec<String>,
    /// Includere nascosti e di sistema.
    pub include_hidden: bool,
    /// Includere i file solo online (verranno scaricati).
    pub include_online_only: bool,
    /// Limitare la dimensione dei file.
    pub limit_file_size: bool,
    /// Dimensione massima, in MiB, in [`MAX_FILE_SIZE_MIB_RANGE`].
    pub max_file_size_mib: u64,
}

impl Default for FilterSettings {
    fn default() -> Self {
        FilterSettings {
            extension_mode: ExtensionMode::All,
            extensions: Vec::new(),
            skip_dirs: Filters::DEFAULT_SKIP_DIRS.map(String::from).to_vec(),
            include_hidden: false,
            include_online_only: false,
            limit_file_size: true,
            max_file_size_mib: Filters::DEFAULT_MAX_FILE_SIZE.bytes() / (1024 * 1024),
        }
    }
}

/// Larghezze delle colonne della tabella, in punti (il contesto prende il resto).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnWidths {
    /// Parola.
    pub word: f32,
    /// Termine.
    pub term: f32,
    /// File.
    pub file: f32,
    /// Posizione.
    pub position: f32,
}

impl ColumnWidths {
    /// Larghezza minima e massima di una colonna.
    pub const RANGE: std::ops::RangeInclusive<f32> = 40.0..=2000.0;
}

impl Default for ColumnWidths {
    fn default() -> Self {
        ColumnWidths {
            word: 160.0,
            term: 110.0,
            file: 240.0,
            position: 120.0,
        }
    }
}

/// Disposizione della finestra.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutSettings {
    /// Larghezza del pannello di ricerca, in punti.
    pub search_panel_width: f32,
    /// Pannello di ricerca visibile.
    pub search_panel_visible: bool,
    /// Colonne della tabella.
    pub columns: ColumnWidths,
    /// Ordinamento scelto; `None` = File → Posizione.
    pub sort: Option<SortSpec>,
    /// Anteprima visibile sotto la tabella.
    pub preview_visible: bool,
    /// Altezza dell'anteprima, in punti.
    pub preview_height: f32,
}

impl LayoutSettings {
    /// Larghezza minima e massima del pannello di ricerca.
    pub const PANEL_RANGE: std::ops::RangeInclusive<f32> = 260.0..=560.0;
    /// Altezza minima e massima dell'anteprima.
    pub const PREVIEW_RANGE: std::ops::RangeInclusive<f32> = 80.0..=900.0;
}

impl Default for LayoutSettings {
    fn default() -> Self {
        LayoutSettings {
            search_panel_width: 320.0,
            search_panel_visible: true,
            columns: ColumnWidths::default(),
            sort: None,
            preview_visible: true,
            preview_height: 220.0,
        }
    }
}

/// Preferenze dell'utente e ultimo stato del modulo di ricerca.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    /// Lingua dell'interfaccia; `None` = quella del sistema.
    pub language: Option<Language>,
    /// Tema.
    pub theme: ThemeChoice,
    /// Opzioni di confronto.
    pub options: MatchOptions,
    /// OR o AND tra i termini.
    pub combine: Combine,
    /// Cartelle e file del modulo.
    pub roots: Vec<PathBuf>,
    /// Termini del modulo, una riga ciascuno.
    pub terms: Vec<String>,
    /// Radici usate di recente, la più recente per prima.
    pub recent_roots: Vec<PathBuf>,
    /// Termini usati di recente, il più recente per primo.
    pub recent_terms: Vec<String>,
    /// Filtri.
    pub filters: FilterSettings,
    /// Disposizione della finestra.
    pub layout: LayoutSettings,
    /// Ultime scelte di esportazione.
    pub export: ExportSettings,
    /// Cache dei documenti.
    pub cache: CacheSettings,
    /// Voce "Cerca con Lynkeus" nel clic destro (spenta per default).
    pub context_menu: bool,
    /// Livello del log.
    pub log_level: LogLevel,
}

/// Inserisce `item` in testa a un elenco di recenti, senza doppioni, al massimo
/// [`MAX_RECENT`] voci.
pub fn push_recent<T: PartialEq>(list: &mut Vec<T>, item: T) {
    list.retain(|existing| *existing != item);
    list.insert(0, item);
    list.truncate(MAX_RECENT);
}

/// Perché le impostazioni non si sono salvate.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// Il file è di una versione più recente del programma: non si riscrive.
    #[error("settings.toml è della versione {0}, più recente di questo programma")]
    NewerVersion(i64),
    /// Il file non si è potuto leggere all'avvio: non si riscrive per non perderlo.
    #[error("settings.toml non letto all'avvio: non si riscrive")]
    NotLoaded,
    /// Errore di scrittura.
    #[error("scrittura di settings.toml non riuscita")]
    Io(#[from] io::Error),
    /// Errore di serializzazione (un bug).
    #[error("impostazioni non serializzabili")]
    Serialize(#[from] toml::ser::Error),
}

/// Com'è andata la lettura.
#[derive(Debug)]
pub enum LoadOutcome {
    /// Nessun file: impostazioni predefinite.
    Missing,
    /// File letto.
    Loaded,
    /// File non TOML: spostato in `moved_to` (se riuscito), impostazioni predefinite.
    Invalid {
        /// Dove è stato spostato il file non valido.
        moved_to: Option<PathBuf>,
    },
    /// File di una versione più recente: letto per quanto possibile, non verrà riscritto.
    NewerVersion(i64),
    /// File non leggibile (permessi, troppo grande): predefinite, non verrà riscritto.
    Unreadable(io::Error),
}

/// Il file `settings.toml`: percorso e documento letto, con le chiavi sconosciute.
#[derive(Debug)]
pub struct SettingsFile {
    path: PathBuf,
    document: Table,
    blocked: Option<Blocked>,
}

#[derive(Debug, Clone, Copy)]
enum Blocked {
    Newer(i64),
    NotLoaded,
}

impl SettingsFile {
    /// Legge `path`. Non fallisce mai: in caso di problemi restituisce le predefinite e
    /// dice perché in [`LoadOutcome`].
    pub fn load(path: PathBuf) -> (SettingsFile, Settings, LoadOutcome) {
        let mut file = SettingsFile {
            path,
            document: Table::new(),
            blocked: None,
        };
        let text = match read_limited(&file.path) {
            Ok(Some(text)) => text,
            Ok(None) => return (file, Settings::default(), LoadOutcome::Missing),
            Err(error) => {
                file.blocked = Some(Blocked::NotLoaded);
                return (file, Settings::default(), LoadOutcome::Unreadable(error));
            }
        };
        let Ok(document) = text.parse::<Table>() else {
            let aside = file.path.with_extension("toml.invalid");
            let moved_to = fs::rename(&file.path, &aside).ok().map(|()| aside);
            if moved_to.is_none() {
                file.blocked = Some(Blocked::NotLoaded);
            }
            let outcome = LoadOutcome::Invalid { moved_to };
            return (file, Settings::default(), outcome);
        };
        let settings = from_table(&document);
        let version = document
            .get("format_version")
            .and_then(Value::as_integer)
            .unwrap_or(FORMAT_VERSION);
        file.document = document;
        if version > FORMAT_VERSION {
            file.blocked = Some(Blocked::Newer(version));
            return (file, settings, LoadOutcome::NewerVersion(version));
        }
        (file, settings, LoadOutcome::Loaded)
    }

    /// Il percorso del file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Scrive `settings` in modo durevole, conservando le chiavi sconosciute lette.
    pub fn save(&mut self, settings: &Settings) -> Result<(), SettingsError> {
        match self.blocked {
            Some(Blocked::Newer(version)) => return Err(SettingsError::NewerVersion(version)),
            Some(Blocked::NotLoaded) => return Err(SettingsError::NotLoaded),
            None => {}
        }
        let mut document = self.document.clone();
        merge(&mut document, to_table(settings));
        let text = toml::to_string(&document)?;
        write_durably(&self.path, text.as_bytes())?;
        self.document = document;
        Ok(())
    }
}

/// Legge il file se esiste, rifiutando quelli oltre [`MAX_FILE_BYTES`].
fn read_limited(path: &Path) -> io::Result<Option<String>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "settings.toml troppo grande",
        ));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Scrive `bytes` in `path` in modo atomico e durevole: temporaneo nuovo nella stessa
/// cartella, `sync_all`, rinomina sopra il vecchio, flush della cartella.
pub fn write_durably(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
    let (temp_path, mut temp) = create_temp(dir, &name)?;
    let written = temp
        .write_all(bytes)
        .and_then(|()| temp.sync_all())
        .and_then(|()| {
            drop(temp);
            fs::rename(&temp_path, path)
        });
    if let Err(error) = written {
        // Il temporaneo è nostro: se la pulizia fallisce resta un file .tmp innocuo, e
        // l'errore che conta è quello della scrittura.
        fs::remove_file(&temp_path).ok();
        return Err(error);
    }
    platform::sync_dir(dir)
}

/// Crea un file temporaneo nuovo (mai uno esistente) accanto a quello finale.
fn create_temp(dir: &Path, name: &str) -> io::Result<(PathBuf, fs::File)> {
    const ATTEMPTS: u32 = 32;
    let pid = std::process::id();
    let mut last_error = None;
    for attempt in 0..ATTEMPTS {
        let candidate = dir.join(format!("{name}.{pid}-{attempt}.tmp"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                last_error = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("nessun nome temporaneo libero")))
}

/// Copia `update` in `base`: le tabelle si fondono chiave per chiave, gli altri valori si
/// sostituiscono; le chiavi solo in `base` restano.
fn merge(base: &mut Table, update: Table) {
    for (key, value) in update {
        match (base.get_mut(&key), value) {
            (Some(Value::Table(existing)), Value::Table(incoming)) => merge(existing, incoming),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

// --- Scrittura ------------------------------------------------------------------------

fn to_table(s: &Settings) -> Table {
    let mut root = Table::new();
    root.insert("format_version".into(), Value::Integer(FORMAT_VERSION));
    let language = match s.language {
        None => "system",
        Some(Language::Italian) => "it",
        Some(Language::English) => "en",
    };
    root.insert("language".into(), language.into());
    let theme = match s.theme {
        ThemeChoice::System => "system",
        ThemeChoice::Light => "light",
        ThemeChoice::Dark => "dark",
    };
    root.insert("theme".into(), theme.into());

    let mut search = Table::new();
    let case = match s.options.case {
        CaseMode::Insensitive => "insensitive",
        CaseMode::Sensitive => "sensitive",
    };
    search.insert("case".into(), case.into());
    let accents = match s.options.accents {
        AccentMode::Distinct => "distinct",
        AccentMode::Ignored => "ignored",
    };
    search.insert("accents".into(), accents.into());
    search.insert(
        "wildcards".into(),
        (s.options.wildcards == WildcardMode::Enabled).into(),
    );
    let combine = match s.combine {
        Combine::Any => "any",
        Combine::All => "all",
    };
    search.insert("combine".into(), combine.into());
    search.insert("roots".into(), paths_value(&s.roots));
    search.insert("terms".into(), strings_value(&s.terms));
    search.insert("recent_roots".into(), paths_value(&s.recent_roots));
    search.insert("recent_terms".into(), strings_value(&s.recent_terms));
    root.insert("search".into(), Value::Table(search));

    let f = &s.filters;
    let mut filters = Table::new();
    let mode = match f.extension_mode {
        ExtensionMode::All => "all",
        ExtensionMode::Include => "include",
        ExtensionMode::Exclude => "exclude",
    };
    filters.insert("extension_mode".into(), mode.into());
    let extensions: Vec<String> = f.extensions.iter().map(|e| e.as_str().to_owned()).collect();
    filters.insert("extensions".into(), strings_value(&extensions));
    filters.insert("skip_dirs".into(), strings_value(&f.skip_dirs));
    filters.insert("include_hidden".into(), f.include_hidden.into());
    filters.insert("include_online_only".into(), f.include_online_only.into());
    filters.insert("limit_file_size".into(), f.limit_file_size.into());
    filters.insert(
        "max_file_size_mib".into(),
        Value::Integer(i64::try_from(f.max_file_size_mib).unwrap_or(i64::MAX)),
    );
    root.insert("filters".into(), Value::Table(filters));

    let l = &s.layout;
    let mut layout = Table::new();
    layout.insert(
        "search_panel_width".into(),
        f64::from(l.search_panel_width).into(),
    );
    layout.insert("search_panel_visible".into(), l.search_panel_visible.into());
    layout.insert("column_word".into(), f64::from(l.columns.word).into());
    layout.insert("column_term".into(), f64::from(l.columns.term).into());
    layout.insert("column_file".into(), f64::from(l.columns.file).into());
    layout.insert(
        "column_position".into(),
        f64::from(l.columns.position).into(),
    );
    let (column, descending) = match l.sort {
        None => ("none", false),
        Some(spec) => (
            match spec.column {
                SortColumn::Word => "word",
                SortColumn::Term => "term",
                SortColumn::File => "file",
                SortColumn::Position => "position",
                SortColumn::Context => "context",
            },
            spec.order == SortOrder::Descending,
        ),
    };
    layout.insert("sort_column".into(), column.into());
    layout.insert("sort_descending".into(), descending.into());
    layout.insert("preview_visible".into(), l.preview_visible.into());
    layout.insert("preview_height".into(), f64::from(l.preview_height).into());
    root.insert("layout".into(), Value::Table(layout));

    let e = &s.export;
    let mut export = Table::new();
    let format = match e.format {
        ExportFormat::Csv => "csv",
        ExportFormat::Txt => "txt",
    };
    export.insert("format".into(), format.into());
    let scope = match e.scope {
        ExportScope::Visible => "visible",
        ExportScope::Selected => "selected",
    };
    export.insert("scope".into(), scope.into());
    // Come per le radici, un percorso non UTF-8 (raro, solo su Linux) non si salva.
    if let Some(dir) = e.last_dir.as_deref().and_then(Path::to_str) {
        export.insert("last_dir".into(), dir.into());
    }
    root.insert("export".into(), Value::Table(export));

    let mut cache = Table::new();
    cache.insert("enabled".into(), s.cache.enabled.into());
    let max_size = i64::try_from(s.cache.max_size_mib).unwrap_or(i64::MAX);
    cache.insert("max_size_mib".into(), max_size.into());
    root.insert("cache".into(), Value::Table(cache));

    let mut integration = Table::new();
    integration.insert("context_menu".into(), s.context_menu.into());
    root.insert("integration".into(), Value::Table(integration));
    let mut diagnostics = Table::new();
    diagnostics.insert("log_level".into(), s.log_level.key().into());
    root.insert("diagnostics".into(), Value::Table(diagnostics));
    root
}

fn strings_value(items: &[String]) -> Value {
    Value::Array(items.iter().map(|s| Value::String(s.clone())).collect())
}

/// I percorsi non rappresentabili in UTF-8 (rari, solo su Linux) non si salvano.
fn paths_value(paths: &[PathBuf]) -> Value {
    Value::Array(
        paths
            .iter()
            .filter_map(|p| p.to_str())
            .map(|s| Value::String(s.to_owned()))
            .collect(),
    )
}

// --- Lettura tollerante ----------------------------------------------------------------

fn from_table(root: &Table) -> Settings {
    let defaults = Settings::default();
    let empty = Table::new();
    let search = table(root, "search").unwrap_or(&empty);
    let filters = table(root, "filters").unwrap_or(&empty);
    let layout = table(root, "layout").unwrap_or(&empty);
    let export = table(root, "export").unwrap_or(&empty);
    let cache = table(root, "cache").unwrap_or(&empty);
    let integration = table(root, "integration").unwrap_or(&empty);
    let diagnostics = table(root, "diagnostics").unwrap_or(&empty);

    let language = match string(root, "language") {
        Some("it") => Some(Language::Italian),
        Some("en") => Some(Language::English),
        _ => None,
    };
    let theme = match string(root, "theme") {
        Some("light") => ThemeChoice::Light,
        Some("dark") => ThemeChoice::Dark,
        _ => ThemeChoice::System,
    };
    let options = MatchOptions {
        case: match string(search, "case") {
            Some("sensitive") => CaseMode::Sensitive,
            _ => CaseMode::Insensitive,
        },
        accents: match string(search, "accents") {
            Some("ignored") => AccentMode::Ignored,
            _ => AccentMode::Distinct,
        },
        wildcards: match boolean(search, "wildcards") {
            Some(false) => WildcardMode::Literal,
            _ => WildcardMode::Enabled,
        },
    };
    let combine = match string(search, "combine") {
        Some("all") => Combine::All,
        _ => Combine::Any,
    };
    let mut terms: Vec<String> = strings(search, "terms")
        .into_iter()
        .filter(|t| t.chars().count() <= Term::MAX_CHARS)
        .collect();
    terms.truncate(Query::MAX_TERMS);
    let mut recent_terms: Vec<String> = strings(search, "recent_terms")
        .into_iter()
        .filter(|t| Term::parse(t, WildcardMode::Literal).is_ok())
        .collect();
    recent_terms.truncate(MAX_RECENT);
    let mut recent_roots = paths(search, "recent_roots");
    recent_roots.truncate(MAX_RECENT);

    let filter_defaults = &defaults.filters;
    let filters = FilterSettings {
        extension_mode: match string(filters, "extension_mode") {
            Some("include") => ExtensionMode::Include,
            Some("exclude") => ExtensionMode::Exclude,
            _ => ExtensionMode::All,
        },
        extensions: strings(filters, "extensions")
            .iter()
            .filter_map(|e| Extension::parse(e).ok())
            .collect(),
        skip_dirs: match filters.get("skip_dirs") {
            Some(_) => strings(filters, "skip_dirs")
                .into_iter()
                .map(|d| d.trim().to_owned())
                .filter(|d| !d.is_empty())
                .collect(),
            None => filter_defaults.skip_dirs.clone(),
        },
        include_hidden: boolean(filters, "include_hidden")
            .unwrap_or(filter_defaults.include_hidden),
        include_online_only: boolean(filters, "include_online_only")
            .unwrap_or(filter_defaults.include_online_only),
        limit_file_size: boolean(filters, "limit_file_size")
            .unwrap_or(filter_defaults.limit_file_size),
        max_file_size_mib: integer(filters, "max_file_size_mib")
            .and_then(|v| u64::try_from(v).ok())
            .map_or(filter_defaults.max_file_size_mib, |v| {
                v.clamp(
                    *MAX_FILE_SIZE_MIB_RANGE.start(),
                    *MAX_FILE_SIZE_MIB_RANGE.end(),
                )
            }),
    };

    let layout_defaults = defaults.layout;
    let column = |key: &str, default: f32| {
        float(layout, key).map_or(default, |v| {
            v.clamp(*ColumnWidths::RANGE.start(), *ColumnWidths::RANGE.end())
        })
    };
    let sort_column = match string(layout, "sort_column") {
        Some("word") => Some(SortColumn::Word),
        Some("term") => Some(SortColumn::Term),
        Some("file") => Some(SortColumn::File),
        Some("position") => Some(SortColumn::Position),
        Some("context") => Some(SortColumn::Context),
        _ => None,
    };
    let order = if boolean(layout, "sort_descending") == Some(true) {
        SortOrder::Descending
    } else {
        SortOrder::Ascending
    };
    let layout = LayoutSettings {
        search_panel_width: float(layout, "search_panel_width").map_or(
            layout_defaults.search_panel_width,
            |v| {
                v.clamp(
                    *LayoutSettings::PANEL_RANGE.start(),
                    *LayoutSettings::PANEL_RANGE.end(),
                )
            },
        ),
        search_panel_visible: boolean(layout, "search_panel_visible")
            .unwrap_or(layout_defaults.search_panel_visible),
        columns: ColumnWidths {
            word: column("column_word", layout_defaults.columns.word),
            term: column("column_term", layout_defaults.columns.term),
            file: column("column_file", layout_defaults.columns.file),
            position: column("column_position", layout_defaults.columns.position),
        },
        sort: sort_column.map(|column| SortSpec { column, order }),
        preview_visible: boolean(layout, "preview_visible")
            .unwrap_or(layout_defaults.preview_visible),
        preview_height: float(layout, "preview_height").map_or(
            layout_defaults.preview_height,
            |v| {
                v.clamp(
                    *LayoutSettings::PREVIEW_RANGE.start(),
                    *LayoutSettings::PREVIEW_RANGE.end(),
                )
            },
        ),
    };

    let export = ExportSettings {
        format: match string(export, "format") {
            Some("txt") => ExportFormat::Txt,
            _ => ExportFormat::Csv,
        },
        scope: match string(export, "scope") {
            Some("selected") => ExportScope::Selected,
            _ => ExportScope::Visible,
        },
        last_dir: string(export, "last_dir")
            .filter(|s| !s.trim().is_empty() && s.len() <= MAX_STRING_BYTES)
            .map(PathBuf::from),
    };

    let cache_defaults = CacheSettings::default();
    let cache = CacheSettings {
        enabled: boolean(cache, "enabled").unwrap_or(cache_defaults.enabled),
        max_size_mib: integer(cache, "max_size_mib")
            .and_then(|v| u64::try_from(v).ok())
            .map_or(cache_defaults.max_size_mib, |v| {
                v.clamp(*CACHE_SIZE_MIB_RANGE.start(), *CACHE_SIZE_MIB_RANGE.end())
            }),
    };

    Settings {
        export,
        cache,
        context_menu: boolean(integration, "context_menu").unwrap_or(defaults.context_menu),
        log_level: string(diagnostics, "log_level")
            .and_then(LogLevel::from_key)
            .unwrap_or(defaults.log_level),
        language,
        theme,
        options,
        combine,
        roots: paths(search, "roots"),
        terms,
        recent_roots,
        recent_terms,
        filters,
        layout,
    }
}

fn table<'a>(t: &'a Table, key: &str) -> Option<&'a Table> {
    t.get(key).and_then(Value::as_table)
}

fn string<'a>(t: &'a Table, key: &str) -> Option<&'a str> {
    t.get(key).and_then(Value::as_str)
}

fn boolean(t: &Table, key: &str) -> Option<bool> {
    t.get(key).and_then(Value::as_bool)
}

fn integer(t: &Table, key: &str) -> Option<i64> {
    t.get(key).and_then(Value::as_integer)
}

/// Un numero finito (intero o decimale) come `f32`.
fn float(t: &Table, key: &str) -> Option<f32> {
    let value = match t.get(key)? {
        Value::Float(f) => *f,
        Value::Integer(i) => i64_to_f64(*i),
        _ => return None,
    };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "il valore si limita subito a un intervallo piccolo"
    )]
    let narrowed = value.clamp(-1.0e6, 1.0e6) as f32;
    value.is_finite().then_some(narrowed)
}

fn i64_to_f64(value: i64) -> f64 {
    // Larghezze e misure: valori piccoli; oltre ±2^31 si satura prima di convertire.
    let clamped = i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX });
    f64::from(clamped)
}

/// Le stringhe di un elenco, saltando i valori non stringa e quelli troppo lunghi.
fn strings(t: &Table, key: &str) -> Vec<String> {
    t.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(MAX_LIST_ITEMS)
                .filter_map(Value::as_str)
                .filter(|s| s.len() <= MAX_STRING_BYTES)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn paths(t: &Table, key: &str) -> Vec<PathBuf> {
    strings(t, key)
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom() -> Settings {
        Settings {
            language: Some(Language::English),
            theme: ThemeChoice::Dark,
            options: MatchOptions {
                case: CaseMode::Sensitive,
                accents: AccentMode::Ignored,
                wildcards: WildcardMode::Literal,
            },
            combine: Combine::All,
            roots: vec![PathBuf::from("dati").join("note"), PathBuf::from("a.txt")],
            terms: vec!["ar".to_owned(), "c?rt*".to_owned()],
            recent_roots: vec![PathBuf::from("dati")],
            recent_terms: vec!["città".to_owned()],
            filters: FilterSettings {
                extension_mode: ExtensionMode::Exclude,
                extensions: vec![Extension::parse("log").expect("valida")],
                skip_dirs: vec!["build".to_owned()],
                include_hidden: true,
                include_online_only: true,
                limit_file_size: false,
                max_file_size_mib: 50,
            },
            layout: LayoutSettings {
                search_panel_width: 300.0,
                search_panel_visible: false,
                columns: ColumnWidths {
                    word: 150.0,
                    term: 90.0,
                    file: 200.0,
                    position: 100.0,
                },
                sort: Some(SortSpec {
                    column: SortColumn::Word,
                    order: SortOrder::Descending,
                }),
                preview_visible: false,
                preview_height: 300.0,
            },
            export: ExportSettings {
                format: ExportFormat::Txt,
                scope: ExportScope::Selected,
                last_dir: Some(PathBuf::from("esportazioni")),
            },
            cache: CacheSettings {
                enabled: false,
                max_size_mib: 250,
            },
            context_menu: true,
            log_level: LogLevel::Debug,
        }
    }

    /// La voce del clic destro è spenta per default; un livello del log sconosciuto torna
    /// al predefinito.
    #[test]
    fn integration_defaults_and_unknown_levels() {
        assert!(!Settings::default().context_menu);
        let table: Table = "[diagnostics]
log_level = \"trace\"
[integration]
context_menu = 1
"
        .parse()
        .expect("toml");
        let settings = from_table(&table);
        assert_eq!(settings.log_level, LogLevel::default());
        assert!(!settings.context_menu);
    }

    #[test]
    fn round_trip_through_the_file_keeps_every_field() {
        let dir = tempfile::tempdir().expect("cartella temporanea");
        let path = dir.path().join(FILE_NAME);
        let (mut file, loaded, outcome) = SettingsFile::load(path.clone());
        assert!(matches!(outcome, LoadOutcome::Missing));
        assert_eq!(loaded, Settings::default());

        let settings = custom();
        file.save(&settings).expect("salvate");
        let (_, reloaded, outcome) = SettingsFile::load(path);
        assert!(matches!(outcome, LoadOutcome::Loaded), "{outcome:?}");
        assert_eq!(reloaded, settings);
    }

    #[test]
    fn defaults_round_trip() {
        let table = to_table(&Settings::default());
        assert_eq!(from_table(&table), Settings::default());
    }

    #[test]
    fn unknown_fields_survive_a_rewrite() {
        let dir = tempfile::tempdir().expect("cartella temporanea");
        let path = dir.path().join(FILE_NAME);
        fs::write(
            &path,
            "format_version = 1\n\
             future_option = \"x\"\n\
             theme = \"dark\"\n\
             [search]\n\
             terms = [\"ar\"]\n\
             future_search_key = 7\n\
             [plugin]\n\
             name = \"altro\"\n",
        )
        .expect("scritto");
        let (mut file, mut settings, _) = SettingsFile::load(path.clone());
        assert_eq!(settings.theme, ThemeChoice::Dark);
        assert_eq!(settings.terms, ["ar"]);
        settings.theme = ThemeChoice::Light;
        file.save(&settings).expect("salvate");

        let text = fs::read_to_string(&path).expect("letto");
        let document: Table = text.parse().expect("TOML valido");
        assert_eq!(string(&document, "future_option"), Some("x"));
        assert_eq!(string(&document, "theme"), Some("light"));
        let search = table(&document, "search").expect("search");
        assert_eq!(integer(search, "future_search_key"), Some(7));
        let plugin = table(&document, "plugin").expect("plugin");
        assert_eq!(string(plugin, "name"), Some("altro"));
        // Nessun temporaneo lasciato nella cartella.
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .expect("cartella")
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn wrong_values_fall_back_one_by_one() {
        let document: Table = "language = 3\n\
             theme = \"viola\"\n\
             [search]\n\
             case = \"sensitive\"\n\
             roots = [\"a\", 5, \"\", \"b\"]\n\
             [filters]\n\
             extensions = [\"txt\", \"tar.gz\", \"md\"]\n\
             max_file_size_mib = -4\n\
             [layout]\n\
             column_word = 1e300\n\
             search_panel_width = \"largo\"\n\
             sort_column = \"colore\"\n\
             [cache]\n\
             enabled = \"si\"\n\
             max_size_mib = 99999999\n"
            .parse()
            .expect("TOML valido");
        let settings = from_table(&document);
        let defaults = Settings::default();
        assert_eq!(settings.language, None);
        assert_eq!(settings.theme, ThemeChoice::System);
        assert_eq!(settings.options.case, CaseMode::Sensitive);
        assert_eq!(settings.roots, [PathBuf::from("a"), PathBuf::from("b")]);
        let extensions: Vec<&str> = settings
            .filters
            .extensions
            .iter()
            .map(Extension::as_str)
            .collect();
        assert_eq!(extensions, ["txt", "md"]);
        assert_eq!(
            settings.filters.max_file_size_mib,
            defaults.filters.max_file_size_mib
        );
        assert_eq!(settings.layout.columns.word, *ColumnWidths::RANGE.end());
        assert_eq!(
            settings.layout.search_panel_width,
            defaults.layout.search_panel_width
        );
        assert_eq!(settings.layout.sort, None);
        assert_eq!(settings.cache.enabled, defaults.cache.enabled);
        assert_eq!(settings.cache.max_size_mib, *CACHE_SIZE_MIB_RANGE.end());
    }

    #[test]
    fn invalid_toml_is_moved_aside_and_not_lost() {
        let dir = tempfile::tempdir().expect("cartella temporanea");
        let path = dir.path().join(FILE_NAME);
        fs::write(&path, "questo non è [toml").expect("scritto");
        let (mut file, settings, outcome) = SettingsFile::load(path.clone());
        assert_eq!(settings, Settings::default());
        let LoadOutcome::Invalid {
            moved_to: Some(aside),
        } = outcome
        else {
            panic!("atteso Invalid con il file spostato: {outcome:?}");
        };
        assert_eq!(
            fs::read_to_string(&aside).expect("conservato"),
            "questo non è [toml"
        );
        file.save(&settings).expect("nuovo file scritto");
        assert!(path.exists());
    }

    #[test]
    fn newer_version_is_read_but_never_rewritten() {
        let dir = tempfile::tempdir().expect("cartella temporanea");
        let path = dir.path().join(FILE_NAME);
        let original = "format_version = 99\ntheme = \"dark\"\n";
        fs::write(&path, original).expect("scritto");
        let (mut file, settings, outcome) = SettingsFile::load(path.clone());
        assert!(matches!(outcome, LoadOutcome::NewerVersion(99)));
        assert_eq!(settings.theme, ThemeChoice::Dark);
        assert!(matches!(
            file.save(&settings),
            Err(SettingsError::NewerVersion(99))
        ));
        assert_eq!(fs::read_to_string(&path).expect("letto"), original);
    }

    #[test]
    fn recent_lists_move_to_front_without_duplicates() {
        let mut list = vec!["b".to_owned(), "a".to_owned()];
        push_recent(&mut list, "a".to_owned());
        assert_eq!(list, ["a", "b"]);
        for i in 0..20 {
            push_recent(&mut list, i.to_string());
        }
        assert_eq!(list.len(), MAX_RECENT);
        assert_eq!(list.first().map(String::as_str), Some("19"));
    }
}
