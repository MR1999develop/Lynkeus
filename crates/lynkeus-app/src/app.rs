// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Stato dell'applicazione e casi d'uso, senza finestra.
//!
//! [`Application`] possiede impostazioni, traduttore, modulo di ricerca, risultati con la
//! loro vista e ricerca in corso. La vista (`ui`) la disegna e le passa i gesti; i test la
//! usano direttamente: impostano radici e termini, avviano, pompano gli eventi e leggono
//! risultati e stato.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use lynkeus_core::{ByteSize, CompiledQuery};
use lynkeus_engine::{
    DocPath, DocumentSource, Hit, Limits, Notify, Outcome, PrivateFs, ProgressSnapshot, Received,
    SearchEvent, SearchHandle, SearchRequest, Summary, Warning, start_search,
};

use crate::context_menu::{self, ContextMenuStore, MenuEntry, MenuError};
use crate::diagnostics::{LogControl, LogLevel};
use crate::instance::InstanceServer;

use crate::export::{self, ExportScope, ExportSettings};
use crate::extraction::{self, ExtractionSetup, OpenedEntries, PoolDocumentUnits};
use crate::form::{
    FilterForm, FormErrors, FormField, MAX_ROOTS, PathInputError, PathInputStatus, RootAdded,
    RootKind, SearchForm, TermRowError, absolute_root,
};
use crate::i18n::{I18n, Language};
use crate::launch::{
    self, ArchiveEntryExtractor, EntryFailure, LaunchOutcome, LaunchTask, NoArchiveExtraction,
    OpenPlan,
};
use crate::platform;
use crate::preview::{self, DocumentUnits, Loaded, NoDocumentUnits, PreviewError, PreviewRequest};
use crate::results::{ResultsView, SortColumn, SortSpec, Step};
use crate::root_check::{RootCheck, RootChecks};
use crate::settings::{
    CACHE_SIZE_MIB_RANGE, CacheSettings, LayoutSettings, Settings, SettingsError, SettingsFile,
    SettingsProblem, push_recent,
};
use crate::theme::{SystemContrast, ThemeChoice};

/// Righe di risultato inserite al massimo in una chiamata di
/// [`Application::pump_events`] fatta dalla vista: il frame resta veloce anche con
/// 200 000 risultati in arrivo.
pub const MAX_HITS_PER_FRAME: usize = 5_000;

/// Dopo quanto tempo la barra d'avanzamento compare.
pub const PROGRESS_DELAY: Duration = Duration::from_secs(1);

/// Quanto si aspetta all'uscita un salvataggio delle impostazioni ancora in corso.
const SETTINGS_SAVE_WAIT: Duration = Duration::from_secs(2);

/// Perché una ricerca non è partita.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// Uno o più campi del modulo non sono validi; ognuno mostra il suo errore.
    #[error("modulo non valido")]
    Form(FormErrors),
    /// Il sistema non ha creato il thread di ricerca.
    #[error("thread di ricerca non avviato")]
    Spawn(#[source] io::Error),
}

/// Stato della ricerca, da mostrare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchStatus {
    /// Nessuna ricerca avviata.
    Idle,
    /// Ricerca in corso.
    Running {
        /// Contatori in questo istante.
        counts: ProgressSnapshot,
        /// Tempo dall'avvio.
        elapsed: Duration,
    },
    /// Ricerca finita (completata, annullata o al limite dei risultati).
    Finished(Summary),
    /// Il worker si è fermato senza riepilogo (panic).
    WorkerLost,
}

/// L'avanzamento da mostrare durante una ricerca.
///
/// Il motore conta i file man mano che li trova e non sa quanti ne troverà: il totale non
/// è mai noto prima della fine, quindi l'avanzamento è indeterminato, con i conteggi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Totale ignoto: `done` file esaminati su `seen` trovati finora.
    Indeterminate {
        /// File letti, saltati o con un avviso.
        done: u64,
        /// File trovati finora (almeno `done`).
        seen: u64,
    },
}

impl SearchStatus {
    /// L'avanzamento da mostrare: solo durante una ricerca, dopo [`PROGRESS_DELAY`].
    pub fn progress(&self) -> Option<Progress> {
        let SearchStatus::Running { counts, elapsed } = self else {
            return None;
        };
        if *elapsed < PROGRESS_DELAY {
            return None;
        }
        let done = counts
            .files_searched
            .saturating_add(counts.files_skipped)
            .saturating_add(counts.binary_skipped)
            .saturating_add(counts.warnings);
        Some(Progress::Indeterminate {
            done,
            seen: counts.files_seen.max(done),
        })
    }
}

/// Esito di [`Application::pump_events`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pump {
    /// Nessuna ricerca in corso.
    NoSearch,
    /// Coda vuota per ora; la ricerca continua.
    Drained,
    /// Raggiunto il limite per chiamata: restano eventi da leggere.
    MorePending,
    /// La ricerca è appena finita (riepilogo ricevuto o worker perso).
    Ended,
}

/// Che cosa mostrare al posto della tabella vuota.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyState {
    /// Ci sono righe: nessuno stato vuoto.
    None,
    /// Nessuna radice: il primo passo è aggiungerne una.
    AddRoot,
    /// Radici presenti, nessuna ricerca fatta: scrivere un termine e cercare.
    WriteTerm,
    /// Ricerca in corso, ancora nessuna riga.
    Searching,
    /// Ricerca arrivata in fondo senza righe.
    NothingFound,
    /// Ricerca annullata prima di trovare righe.
    Cancelled,
    /// Ricerca interrotta da un errore interno prima di trovare righe.
    Interrupted,
}

/// Scheda del pannello centrale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    /// I risultati.
    #[default]
    Results,
    /// Gli avvisi.
    Warnings,
}

/// Stato della vista che non si salva.
#[derive(Debug, Clone, Default)]
pub struct ViewState {
    /// Scheda mostrata.
    pub tab: Tab,
    /// Finestra Informazioni aperta.
    pub about_open: bool,
    /// Richiesta di portare il focus su un campo del modulo al prossimo frame (Ctrl+F, o il
    /// primo campo sbagliato quando la ricerca non parte).
    pub focus_field: Option<FormField>,
    /// Richiesta di aprire la sezione Filtri al prossimo frame.
    pub open_filters: bool,
    /// Dialogo "Impostazioni di esportazione" aperto, con la copia che modifica (Annulla
    /// la butta).
    pub export_dialog: Option<ExportSettings>,
    /// Anteprima già portata in vista sull'unità del risultato: si scorre una volta sola
    /// per ogni caricamento.
    pub preview_scrolled: Option<PreviewKey>,
}

/// Quale anteprima: risultato `hit` della ricerca numero `generation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewKey {
    /// Numero della ricerca ([`Results::generation`]).
    pub generation: u64,
    /// Indice del risultato in [`Results::hits`].
    pub hit: usize,
}

/// Stato dell'anteprima.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PreviewState {
    /// Nessun risultato con il cursore, o anteprima nascosta.
    #[default]
    NoRow,
    /// Lettura in corso, dall'istante indicato.
    Loading(PreviewKey, Instant),
    /// Pronta.
    Ready(PreviewKey, Loaded),
    /// Non riuscita.
    Failed(PreviewKey, PreviewError),
}

impl PreviewState {
    /// Il risultato a cui si riferisce.
    pub fn key(&self) -> Option<PreviewKey> {
        match self {
            PreviewState::NoRow => None,
            PreviewState::Loading(key, _)
            | PreviewState::Ready(key, _)
            | PreviewState::Failed(key, _) => Some(*key),
        }
    }
}

/// Un messaggio per l'utente sopra la tabella, finché non lo chiude o ne arriva un altro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// Il file è un programma o uno script: non si apre.
    Executable {
        /// Il documento (per "Mostra nella cartella").
        doc: Arc<DocPath>,
    },
    /// Aprire una voce d'archivio non è ancora possibile.
    ArchiveEntry,
    /// Il sistema non ha aperto il file.
    OpenFailed {
        /// Il documento, come mostrato.
        file: String,
    },
    /// Il sistema non ha mostrato il file nella cartella.
    RevealFailed {
        /// Il documento, come mostrato.
        file: String,
    },
    /// Righe copiate negli appunti.
    Copied {
        /// Quante.
        rows: usize,
    },
    /// Esportazione riuscita.
    Exported {
        /// Righe scritte.
        rows: usize,
        /// Il file scritto.
        file: String,
    },
    /// Esportazione non riuscita: il file di prima, se c'era, è intatto.
    ExportFailed {
        /// Il file che si voleva scrivere.
        file: String,
    },
    /// Il sistema non ha avviato il thread di lavoro.
    NoThread,
    /// Cache dei documenti svuotata.
    CacheCleared {
        /// MiB liberati (arrotondati per eccesso).
        freed_mib: u64,
    },
    /// La cache non si è svuotata del tutto (file in uso o permessi).
    CacheClearFailed,
    /// Una voce d'archivio non si è estratta per aprirla.
    EntryFailed {
        /// Il documento, come mostrato.
        file: String,
        /// Perché.
        reason: EntryFailure,
    },
    /// La voce del clic destro non si è accesa o spenta.
    ContextMenuFailed(MenuError),
    /// Cartelle dell'app non private (di altri o aperte ad altri): le funzioni che le
    /// usano sono spente per questa sessione.
    PrivateDirsRefused {
        /// Quante cartelle.
        count: usize,
    },
    /// Luoghi non aggiunti: l'elenco ha già il massimo di voci.
    RootsLimit {
        /// Il massimo ([`MAX_ROOTS`]).
        max: usize,
    },
    /// Filtri riportati ai predefiniti; "Annulla" rimette questi.
    FiltersReset {
        /// I filtri di prima.
        previous: Box<FilterForm>,
    },
    /// Le modifiche alle impostazioni non arrivano sul disco.
    SettingsNotSaved(SettingsProblem),
    /// Il file delle impostazioni non era valido: messo da parte, si riparte dai
    /// predefiniti.
    SettingsReplaced,
}

/// Una scelta fatta in un dialogo di sistema, aperto in un thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChoice {
    /// Cartelle o file da aggiungere ai luoghi.
    AddRoots(Vec<PathBuf>),
    /// Il file in cui esportare.
    ExportTo(PathBuf),
}

/// Una modifica della voce del clic destro in corso.
struct ContextMenuJob {
    /// Lo stato dell'opzione se riesce.
    enabled: bool,
    done: Receiver<Result<(), MenuError>>,
}

/// La voce del clic destro: dove scriverla e per quale eseguibile.
struct ContextMenuSetup {
    store: Arc<dyn ContextMenuStore>,
    exe: PathBuf,
}

/// Una copia negli appunti in preparazione: il testo si costruisce in un thread.
struct CopyJob {
    rows: usize,
    started: Instant,
    done: Receiver<String>,
}

/// Un salvataggio delle impostazioni in corso: le impostazioni scritte e l'esito.
struct SaveJob {
    done: Receiver<(Settings, Result<(), SettingsError>)>,
}

/// Un'esportazione in corso.
struct ExportJob {
    rows: usize,
    file: String,
    done: Receiver<io::Result<()>>,
}

/// Un'anteprima in lettura.
struct PreviewJob {
    key: PreviewKey,
    cancel: Arc<AtomicBool>,
    done: Receiver<Result<Loaded, PreviewError>>,
}

/// Un'apertura in corso.
struct LaunchJob {
    task: LaunchTask,
    file: String,
    done: Receiver<LaunchOutcome>,
}

/// Una ricerca in corso.
struct Job {
    handle: SearchHandle,
    started: Instant,
}

/// Stato della ricerca, interno.
enum JobState {
    Idle,
    Running(Job),
    Finished(Summary),
    WorkerLost,
}

/// Risultati di una ricerca.
#[derive(Debug, Default)]
pub struct Results {
    /// Numero della ricerca che li ha prodotti; cambia a ogni avvio (es. per riportare
    /// la tabella in cima).
    pub generation: u64,
    /// Le radici della ricerca, per i percorsi relativi ([`Hit::root`] è un indice qui).
    pub roots: Vec<PathBuf>,
    /// I termini della ricerca, per la colonna Termine (indici di [`Hit::terms`]).
    pub terms: Vec<String>,
    /// La query compilata della ricerca, per evidenziare l'anteprima.
    pub query: Option<Arc<CompiledQuery>>,
    /// Le righe, nell'ordine di arrivo; condivise (senza copia) con i thread che copiano
    /// ed esportano.
    pub hits: Arc<Vec<Hit>>,
    /// Gli avvisi, nell'ordine di arrivo.
    pub warnings: Vec<Warning>,
}

/// L'applicazione: radice dell'albero di proprietà dello stato.
pub struct Application {
    settings: Settings,
    /// Il file delle impostazioni, condiviso con il thread che lo scrive.
    settings_file: Option<Arc<Mutex<SettingsFile>>>,
    /// Le impostazioni come sono sul disco, per scrivere solo se cambiano.
    saved: Option<Settings>,
    save_job: Option<SaveJob>,
    /// Vero se il file letto all'avvio non va riscritto (più recente o illeggibile).
    settings_blocked: bool,
    /// Vero dopo il primo salvataggio fallito: il messaggio si mostra una volta.
    save_failure_reported: bool,
    copy_job: Option<CopyJob>,
    /// Testo pronto per gli appunti, che la vista consegna al sistema.
    clipboard: Option<String>,
    dialog_job: Option<Receiver<Option<FileChoice>>>,
    /// Chi avvisare (un nuovo frame) quando un lavoro finisce senza che la vista lo chieda.
    waker: Notify,
    /// Colori del tema a contrasto elevato del sistema, se attivo.
    system_contrast: Option<SystemContrast>,
    i18n: I18n,
    limits: Limits,
    /// Campi del modulo, modificati dalla vista.
    pub form: SearchForm,
    /// Disposizione della finestra, modificata dalla vista e salvata.
    pub layout: LayoutSettings,
    /// Stato della vista non salvato.
    pub view_state: ViewState,
    start_error: Option<StartError>,
    results: Results,
    view: ResultsView,
    job: JobState,
    current_file: Option<Arc<DocPath>>,
    root_checks: RootChecks,
    preview: PreviewState,
    preview_job: Option<PreviewJob>,
    documents: Arc<dyn DocumentUnits>,
    archive_extractor: Arc<dyn ArchiveEntryExtractor>,
    launch_job: Option<LaunchJob>,
    export_job: Option<ExportJob>,
    notice: Option<Notice>,
    /// Pool dei processi estrattori e cache; `None` finché non si collega (test senza
    /// documenti).
    document_source: Option<Arc<DocumentSource>>,
    /// Cartella privata dei file estratti dagli archivi per aprirli.
    opened_entries: Option<Arc<OpenedEntries>>,
    cache_job: Option<Receiver<io::Result<ByteSize>>>,
    /// Cartelle e file privati (descrittore esplicito su Windows).
    private_fs: Arc<dyn PrivateFs>,
    /// Prima istanza degli avvii con `--add`: riceve i percorsi delle altre.
    instance: Option<InstanceServer>,
    context_menu: Option<ContextMenuSetup>,
    context_menu_job: Option<ContextMenuJob>,
    log_control: Option<LogControl>,
}

impl Application {
    /// Composition root: costruisce l'app dalle impostazioni (lingua del sistema se non
    /// scelta), senza file di impostazioni. Usata dai test d'accettazione.
    pub fn build(settings: Settings) -> anyhow::Result<Application> {
        Ok(Application::with_file(settings, None))
    }

    /// Come [`Application::build`], con il file in cui salvare le impostazioni.
    pub fn build_persistent(settings: Settings, file: SettingsFile) -> anyhow::Result<Application> {
        Ok(Application::with_file(settings, Some(file)))
    }

    fn with_file(settings: Settings, settings_file: Option<SettingsFile>) -> Application {
        let language = settings.language.unwrap_or_else(Language::from_system);
        // Le radici si verificano dopo, fuori dal thread dell'interfaccia.
        let form = SearchForm::from_settings(&settings, |_| RootKind::Pending);
        let saved = settings_file.as_ref().map(|_| settings.clone());
        let blocked = settings_file.as_ref().and_then(SettingsFile::save_blocked);
        let notice = match (blocked, &settings_file) {
            (Some(problem), _) => Some(Notice::SettingsNotSaved(problem)),
            (None, Some(file)) => file.replaced_invalid().then_some(Notice::SettingsReplaced),
            (None, None) => None,
        };
        let mut root_checks = RootChecks::new();
        for root in &form.roots {
            root_checks.request(root.path.clone());
        }
        Application {
            i18n: I18n::new(language),
            limits: Limits::default(),
            form,
            layout: settings.layout,
            view_state: ViewState::default(),
            start_error: None,
            results: Results::default(),
            view: ResultsView::new(settings.layout.sort),
            job: JobState::Idle,
            current_file: None,
            root_checks,
            preview: PreviewState::NoRow,
            preview_job: None,
            documents: Arc::new(NoDocumentUnits),
            archive_extractor: Arc::new(NoArchiveExtraction),
            launch_job: None,
            export_job: None,
            notice,
            document_source: None,
            opened_entries: None,
            cache_job: None,
            private_fs: platform::private_fs(),
            instance: None,
            context_menu: None,
            context_menu_job: None,
            log_control: None,
            settings,
            settings_file: settings_file.map(|file| Arc::new(Mutex::new(file))),
            saved,
            save_job: None,
            settings_blocked: blocked.is_some(),
            save_failure_reported: false,
            copy_job: None,
            clipboard: None,
            dialog_job: None,
            waker: Arc::new(|| {}),
            system_contrast: None,
        }
    }

    /// Chi avvisare (di solito la richiesta di un nuovo frame) quando un lavoro avviato
    /// senza la vista finisce: salvataggio periodico, dialoghi.
    pub fn set_waker(&mut self, waker: Notify) {
        self.waker = waker;
    }

    /// I colori del tema a contrasto elevato del sistema, letti all'avvio dalla piattaforma;
    /// `None` se il tema non è attivo. La vista li applica con [`crate::setup_context`].
    pub fn set_system_contrast(&mut self, contrast: Option<SystemContrast>) {
        self.system_contrast = contrast;
    }

    /// I colori del tema a contrasto elevato, se attivo.
    pub fn system_contrast(&self) -> Option<SystemContrast> {
        self.system_contrast
    }

    /// Il traduttore.
    pub fn i18n(&self) -> &I18n {
        &self.i18n
    }

    /// Le impostazioni come sarebbero salvate ora (modulo e disposizione compresi).
    pub fn settings(&self) -> Settings {
        let mut settings = self.settings.clone();
        self.form.write_settings(&mut settings);
        settings.layout = self.layout;
        settings.layout.sort = self.view.sort();
        settings
    }

    /// I limiti delle ricerche.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Lingua scelta; `None` = quella del sistema.
    pub fn language_choice(&self) -> Option<Language> {
        self.settings.language
    }

    /// Cambia la lingua dell'interfaccia; `None` = quella del sistema.
    pub fn set_language(&mut self, language: Option<Language>) {
        self.settings.language = language;
        self.i18n = I18n::new(language.unwrap_or_else(Language::from_system));
    }

    /// Il tema scelto.
    pub fn theme(&self) -> ThemeChoice {
        self.settings.theme
    }

    /// Cambia il tema (la vista lo applica al contesto).
    pub fn set_theme(&mut self, theme: ThemeChoice) {
        self.settings.theme = theme;
    }

    /// Le radici usate di recente.
    pub fn recent_roots(&self) -> &[PathBuf] {
        &self.settings.recent_roots
    }

    /// I termini usati di recente.
    pub fn recent_terms(&self) -> &[String] {
        &self.settings.recent_terms
    }

    /// Sostituisce le radici con la sola `root` (cartella o file).
    pub fn set_root(&mut self, root: &Path) {
        self.form.roots.clear();
        self.add_root(root.to_owned());
    }

    /// Aggiunge una radice "da verificare" e ne avvia la verifica sul disco; falso se
    /// c'era già o se l'elenco è pieno ([`MAX_ROOTS`], con un avviso).
    pub fn add_root(&mut self, root: PathBuf) -> bool {
        let root = absolute_root(root);
        match self.form.add_root(root.clone(), RootKind::Pending) {
            RootAdded::Added => {
                self.root_checks.request(root);
                true
            }
            RootAdded::Duplicate => false,
            RootAdded::LimitReached => {
                self.notice = Some(Notice::RootsLimit { max: MAX_ROOTS });
                false
            }
        }
    }

    /// Verifica il percorso scritto nel campo apposito, senza bloccare: intanto
    /// [`SearchForm::path_status`] è [`PathInputStatus::Checking`]; quando il disco
    /// risponde ([`Application::pump_root_checks`]) il percorso diventa una radice se
    /// esiste, altrimenti il campo mostra l'errore.
    pub fn add_typed_path(&mut self) {
        let typed = self.form.path_input.trim().trim_matches('"');
        if typed.is_empty() {
            self.form.path_status = PathInputStatus::Idle;
            return;
        }
        let path = absolute_root(PathBuf::from(typed));
        if self.form.roots.iter().any(|r| r.path == path) {
            self.form.path_input.clear();
            self.form.path_status = PathInputStatus::Idle;
        } else if self.form.roots_full() {
            self.form.path_status = PathInputStatus::Error(PathInputError::TooManyRoots);
        } else if self.root_checks.request(path.clone()) {
            self.form.path_status = PathInputStatus::Checking(path);
        } else {
            // Nessun thread per la verifica: la radice resta da verificare e, se manca,
            // la ricerca lo segnalerà negli avvisi.
            self.form.add_root(path, RootKind::Pending);
            self.form.path_input.clear();
            self.form.path_status = PathInputStatus::Idle;
        }
    }

    /// Chi avvisare (di solito la richiesta di un nuovo frame) quando il disco risponde a
    /// una verifica; vale solo la prima chiamata.
    pub fn set_root_check_waker(&self, waker: Notify) {
        self.root_checks.set_waker(waker);
    }

    /// Vero se c'è già chi avvisare per le verifiche delle radici.
    pub fn has_root_check_waker(&self) -> bool {
        self.root_checks.has_waker()
    }

    /// Applica le risposte arrivate dalle verifiche delle radici, senza attendere.
    pub fn pump_root_checks(&mut self) {
        while let Some(RootCheck { path, kind }) = self.root_checks.try_recv() {
            self.form.set_root_kind(&path, kind);
            if self.form.path_status != PathInputStatus::Checking(path.clone()) {
                continue;
            }
            if kind == RootKind::Missing {
                self.form.path_status = PathInputStatus::Error(PathInputError::NotFound);
            } else if self.form.add_root(path, kind) == RootAdded::LimitReached {
                self.form.path_status = PathInputStatus::Error(PathInputError::TooManyRoots);
            } else {
                self.form.path_input.clear();
                self.form.path_status = PathInputStatus::Idle;
            }
        }
    }

    /// Vero se qualche verifica delle radici non ha ancora risposto.
    pub fn root_checks_pending(&self) -> bool {
        self.root_checks.is_pending()
    }

    /// Sostituisce i termini con il solo `term`.
    pub fn set_term(&mut self, term: &str) {
        self.form.set_terms(&[term]);
    }

    /// Sostituisce i termini.
    pub fn set_terms<S: AsRef<str>>(&mut self, terms: &[S]) {
        self.form.set_terms(terms);
    }

    /// L'errore dell'ultimo tentativo di avvio.
    pub fn start_error(&self) -> Option<&StartError> {
        self.start_error.as_ref()
    }

    /// L'errore "troppo complesso" della riga `index` all'ultimo tentativo: le altre
    /// validazioni dei termini si vedono dal vivo con [`SearchForm::term_error`].
    pub fn term_compile_error(&self, index: usize) -> bool {
        matches!(
            &self.start_error,
            Some(StartError::Form(errors))
                if errors.terms.get(index) == Some(&Some(TermRowError::TooComplex))
        )
    }

    /// Dimentica l'errore dell'ultimo tentativo (il modulo è cambiato).
    pub fn clear_start_error(&mut self) {
        self.start_error = None;
    }

    /// Vero se una ricerca è in corso.
    pub fn is_searching(&self) -> bool {
        matches!(self.job, JobState::Running(_))
    }

    /// Avvia una ricerca con i campi del modulo; una ricerca in corso viene annullata e i
    /// suoi risultati scartati.
    ///
    /// Se il modulo non è valido nulla cambia: la ricerca precedente (in corso o finita) e
    /// i suoi risultati restano. Se il modulo è valido la ricerca in corso si annulla prima
    /// di avviare la nuova; se poi il thread non parte, la precedente finisce come
    /// annullata e i suoi risultati restano.
    ///
    /// `notify` è chiamata dal worker a ogni evento (la vista passa `request_repaint`).
    /// L'errore resta anche in [`Application::start_error`] e il modulo mostra da ora gli
    /// errori dei campi. Con un modulo non valido il pannello di ricerca ricompare, i Filtri
    /// si aprono se l'errore è lì e il focus va al primo campo da correggere.
    pub fn start_search(&mut self, notify: Notify) -> Result<(), &StartError> {
        self.start_error = None;
        self.form.reveal_all_errors();
        let search = match self.form.validate() {
            Ok(search) => search,
            Err(errors) => {
                self.reveal_form_errors(&errors);
                return Err(self.start_error.insert(StartError::Form(errors)));
            }
        };
        let query = search.query.clone();
        let request = SearchRequest {
            roots: search.roots.clone(),
            query: search.query,
            filters: search.filters,
            limits: self.limits,
        };
        // Una sola ricerca alla volta sul disco.
        self.cancel_search();
        let handle = match start_search(request, self.document_source.clone(), notify) {
            Ok(handle) => handle,
            Err(error) => return Err(self.start_error.insert(StartError::Spawn(error))),
        };
        self.results = Results {
            generation: self.results.generation.wrapping_add(1),
            roots: search.roots,
            terms: search.terms,
            query: Some(Arc::new(query)),
            hits: Arc::default(),
            warnings: Vec::new(),
        };
        self.view.reset();
        self.current_file = None;
        self.job = JobState::Running(Job {
            handle,
            started: Instant::now(),
        });
        self.remember_recent();
        Ok(())
    }

    /// Porta in vista i campi sbagliati: pannello di ricerca, Filtri se servono, focus sul
    /// primo campo da correggere.
    fn reveal_form_errors(&mut self, errors: &FormErrors) {
        self.layout.search_panel_visible = true;
        if errors.extensions.is_some() {
            self.view_state.open_filters = true;
        }
        self.view_state.focus_field = errors.first_field();
    }

    /// Chiede di annullare la ricerca in corso; il riepilogo arriverà con gli eventi.
    pub fn cancel_search(&mut self) {
        if let JobState::Running(job) = &self.job {
            job.handle.cancel();
        }
    }

    /// Legge gli eventi arrivati, al massimo circa `max_hits` righe di risultato.
    pub fn pump_events(&mut self, max_hits: usize) -> Pump {
        let pump = self.receive(max_hits);
        self.view.extend_to(&self.results.hits, &self.results.roots);
        if pump == Pump::Ended {
            self.view.finish(&self.results.hits, &self.results.roots);
        }
        pump
    }

    fn receive(&mut self, max_hits: usize) -> Pump {
        let JobState::Running(job) = &mut self.job else {
            return Pump::NoSearch;
        };
        if let Some(current) = job.handle.progress().current_file() {
            self.current_file = Some(current);
        }
        // Una copia o un'esportazione sta leggendo le righe: gli eventi aspettano in coda
        // (il motore rallenta) invece di copiare tutte le righe per aggiungerne di nuove.
        if Arc::strong_count(&self.results.hits) > 1 {
            return Pump::Drained;
        }
        let mut inserted = 0usize;
        loop {
            if inserted >= max_hits {
                return Pump::MorePending;
            }
            match job.handle.try_recv() {
                Received::Event(SearchEvent::Hits(hits)) => {
                    inserted += hits.len();
                    Arc::make_mut(&mut self.results.hits).extend(hits);
                }
                Received::Event(SearchEvent::Warning(warning)) => {
                    self.results.warnings.push(warning);
                }
                Received::Event(SearchEvent::Finished(summary)) => {
                    self.job = JobState::Finished(summary);
                    self.current_file = None;
                    return Pump::Ended;
                }
                Received::Empty => return Pump::Drained,
                Received::Ended | Received::WorkerLost => {
                    self.job = JobState::WorkerLost;
                    self.current_file = None;
                    return Pump::Ended;
                }
            }
        }
    }

    /// Stato della ricerca in questo istante.
    pub fn status(&self) -> SearchStatus {
        match &self.job {
            JobState::Idle => SearchStatus::Idle,
            JobState::Running(job) => SearchStatus::Running {
                counts: job.handle.progress().snapshot(),
                elapsed: job.started.elapsed(),
            },
            JobState::Finished(summary) => SearchStatus::Finished(*summary),
            JobState::WorkerLost => SearchStatus::WorkerLost,
        }
    }

    /// Vero se l'ultima ricerca si è fermata al limite dei risultati (banner).
    pub fn limit_reached(&self) -> bool {
        matches!(&self.job, JobState::Finished(s) if s.outcome == Outcome::LimitReached)
    }

    /// Che cosa mostrare se la tabella è vuota.
    pub fn empty_state(&self) -> EmptyState {
        empty_state(
            !self.results.hits.is_empty(),
            !self.form.roots.is_empty(),
            &self.job,
        )
    }

    /// Il documento in lettura (l'ultimo visto), durante una ricerca.
    pub fn current_file(&self) -> Option<&DocPath> {
        self.current_file.as_deref()
    }

    /// I risultati dell'ultima ricerca.
    pub fn results(&self) -> &Results {
        &self.results
    }

    /// Ordine e selezione delle righe.
    pub fn view(&self) -> &ResultsView {
        &self.view
    }

    /// Ordine e selezione delle righe, da cambiare con i gesti sulla tabella.
    pub fn view_mut(&mut self) -> &mut ResultsView {
        &mut self.view
    }

    /// Ordina per `column` (un clic sull'intestazione: la seconda volta inverte).
    pub fn sort_by(&mut self, column: SortColumn) {
        let spec = SortSpec::after_click(self.view.sort(), column);
        self.view
            .set_sort(spec, &self.results.hits, &self.results.roots);
    }

    // --- Anteprima -------------------------------------------------------------------

    /// Collega la fonte delle unità di testo dei documenti per l'anteprima (di solito la fa
    /// [`Application::connect_extraction`]). Senza, i documenti mostrano il solo contesto.
    pub fn set_document_units(&mut self, source: Arc<dyn DocumentUnits>) {
        self.documents = source;
    }

    /// Lo stato dell'anteprima.
    pub fn preview(&self) -> &PreviewState {
        &self.preview
    }

    /// Il risultato con il cursore, se c'è.
    pub fn cursor_hit(&self) -> Option<&Hit> {
        self.results.hits.get(self.view.cursor_hit()?)
    }

    /// Allinea l'anteprima al risultato con il cursore, senza attendere: raccoglie la
    /// lettura finita e, se il cursore è cambiato, annulla quella in corso e ne avvia
    /// un'altra in un thread. `notify` è chiamata a lettura finita.
    pub fn update_preview(&mut self, notify: &Notify) {
        self.poll_preview();
        let wanted = self
            .view
            .cursor_hit()
            .filter(|_| self.layout.preview_visible)
            .map(|hit| PreviewKey {
                generation: self.results.generation,
                hit,
            });
        if wanted == self.preview.key() {
            return;
        }
        if let Some(job) = self.preview_job.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.preview = match wanted {
            None => PreviewState::NoRow,
            Some(key) => self.start_preview(key, notify),
        };
    }

    fn start_preview(&mut self, key: PreviewKey, notify: &Notify) -> PreviewState {
        let (Some(hit), Some(query)) = (self.results.hits.get(key.hit), &self.results.query) else {
            return PreviewState::Ready(key, Loaded::ContextOnly);
        };
        let request = PreviewRequest {
            doc: Arc::clone(&hit.doc),
            location: hit.location.clone(),
            query: Arc::clone(query),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let documents = Arc::clone(&self.documents);
        let worker_cancel = Arc::clone(&cancel);
        let notify = Arc::clone(notify);
        let spawned = thread::Builder::new()
            .name("anteprima".to_owned())
            .spawn(move || {
                let loaded = preview::load(&request, documents.as_ref(), &worker_cancel);
                // Se l'app ha cambiato risultato nel frattempo nessuno ascolta più.
                if tx.send(loaded).is_ok() {
                    notify();
                }
            });
        match spawned {
            Ok(_) => {
                self.preview_job = Some(PreviewJob {
                    key,
                    cancel,
                    done: rx,
                });
                PreviewState::Loading(key, Instant::now())
            }
            Err(_) => PreviewState::Failed(key, PreviewError::NoThread),
        }
    }

    fn poll_preview(&mut self) {
        let Some(job) = &self.preview_job else {
            return;
        };
        let state = match job.done.try_recv() {
            Ok(Ok(loaded)) => PreviewState::Ready(job.key, loaded),
            Ok(Err(error)) => PreviewState::Failed(job.key, error),
            Err(TryRecvError::Empty) => return,
            // Il thread è finito senza risposta: un panic nella lettura.
            Err(TryRecvError::Disconnected) => {
                PreviewState::Failed(job.key, PreviewError::NoThread)
            }
        };
        self.preview = state;
        self.preview_job = None;
    }

    /// Vero se l'anteprima sta leggendo.
    pub fn preview_loading(&self) -> bool {
        self.preview_job.is_some()
    }

    /// Dove porta `step` dal cursore (posizione mostrata), se c'è una riga.
    pub fn step_target(&self, step: Step) -> Option<usize> {
        self.view.step_target(step)
    }

    /// Sposta cursore e selezione con `step`; falso se non c'è una riga in quella
    /// direzione.
    pub fn step(&mut self, step: Step) -> bool {
        match self.step_target(step) {
            Some(position) => {
                self.view.jump_to(position);
                true
            }
            None => false,
        }
    }

    // --- Apri e mostra nella cartella -------------------------------------------------

    /// Collega l'estrazione delle voci d'archivio in una cartella temporanea privata (di
    /// solito la fa [`Application::connect_extraction`]). Senza, "Apri" su una voce
    /// d'archivio mostra un messaggio.
    pub fn set_archive_extractor(&mut self, extractor: Arc<dyn ArchiveEntryExtractor>) {
        self.archive_extractor = extractor;
    }

    /// Apre il risultato con il cursore con il programma predefinito, in un thread; un
    /// programma o uno script non si apre e lo dice un messaggio ([`Notice::Executable`]).
    pub fn open_current(&mut self, notify: &Notify) {
        let Some(doc) = self.cursor_hit().map(|hit| Arc::clone(&hit.doc)) else {
            return;
        };
        let task = match launch::open_plan(&doc) {
            OpenPlan::Executable => {
                self.notice = Some(Notice::Executable { doc });
                return;
            }
            OpenPlan::ArchiveEntry(entry) => LaunchTask::OpenArchiveEntry(entry),
            OpenPlan::File(path) => LaunchTask::Open(path),
        };
        self.start_launch(task, doc.to_string(), notify);
    }

    /// Mostra nella cartella il risultato con il cursore (per una voce d'archivio,
    /// l'archivio), in un thread.
    pub fn reveal_current(&mut self, notify: &Notify) {
        if let Some(doc) = self.cursor_hit().map(|hit| Arc::clone(&hit.doc)) {
            self.reveal(&doc, notify);
        }
    }

    /// Mostra `doc` nella sua cartella (per una voce d'archivio, l'archivio).
    pub fn reveal(&mut self, doc: &DocPath, notify: &Notify) {
        let task = LaunchTask::Reveal(doc.path.clone());
        self.start_launch(task, doc.to_string(), notify);
    }

    fn start_launch(&mut self, task: LaunchTask, file: String, notify: &Notify) {
        let (tx, rx) = mpsc::channel();
        let extractor = Arc::clone(&self.archive_extractor);
        let worker_task = task.clone();
        let notify = Arc::clone(notify);
        let spawned = thread::Builder::new()
            .name("apertura".to_owned())
            .spawn(move || {
                let outcome = launch::run(&worker_task, extractor.as_ref());
                if tx.send(outcome).is_ok() {
                    notify();
                }
            });
        match spawned {
            Ok(_) => {
                self.launch_job = Some(LaunchJob {
                    task,
                    file,
                    done: rx,
                });
            }
            Err(_) => self.notice = Some(Notice::NoThread),
        }
    }

    fn poll_launch(&mut self) {
        let Some(job) = &self.launch_job else {
            return;
        };
        let outcome = match job.done.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => LaunchOutcome::Failed,
        };
        let file = job.file.clone();
        self.notice = match (outcome, &job.task) {
            (LaunchOutcome::Done, _) => self.notice.take(),
            (LaunchOutcome::ArchiveUnavailable, _) => Some(Notice::ArchiveEntry),
            (LaunchOutcome::EntryFailed(reason), _) => Some(Notice::EntryFailed { file, reason }),
            (LaunchOutcome::Failed, LaunchTask::Reveal(_)) => Some(Notice::RevealFailed { file }),
            (LaunchOutcome::Failed, _) => Some(Notice::OpenFailed { file }),
        };
        self.launch_job = None;
    }

    // --- Copia ed esportazione --------------------------------------------------------

    /// Copia negli appunti le righe selezionate, come TSV: il testo si costruisce in un
    /// thread; quando è pronto [`Application::take_clipboard_text`] lo consegna alla vista
    /// e un messaggio dice quante righe. Senza selezione o con una copia in corso non fa
    /// nulla.
    pub fn copy_selection(&mut self, notify: &Notify) {
        if self.copy_job.is_some() {
            return;
        }
        let indices: Vec<usize> = self.view.selected_hits().collect();
        if indices.is_empty() {
            return;
        }
        let rows = indices.len();
        let hits = Arc::clone(&self.results.hits);
        let terms = self.results.terms.clone();
        let i18n = self.i18n.clone();
        let (tx, rx) = mpsc::channel();
        let notify = Arc::clone(notify);
        let spawned = thread::Builder::new()
            .name("copia".to_owned())
            .spawn(move || {
                let table = export::results_table(&i18n, &hits, &terms, indices);
                drop(hits);
                if tx.send(export::to_tsv(&table.rows)).is_ok() {
                    notify();
                }
            });
        match spawned {
            Ok(_) => {
                self.copy_job = Some(CopyJob {
                    rows,
                    started: Instant::now(),
                    done: rx,
                });
            }
            Err(_) => self.notice = Some(Notice::NoThread),
        }
    }

    /// Vero se una copia negli appunti è in preparazione.
    pub fn is_copying(&self) -> bool {
        self.copy_job.is_some()
    }

    /// Da quanto è in preparazione la copia negli appunti, se c'è.
    pub fn copy_elapsed(&self) -> Option<Duration> {
        self.copy_job.as_ref().map(|job| job.started.elapsed())
    }

    /// Il testo pronto per gli appunti, una volta sola: la vista lo consegna al sistema.
    pub fn take_clipboard_text(&mut self) -> Option<String> {
        self.clipboard.take()
    }

    fn poll_copy(&mut self) {
        let Some(job) = &self.copy_job else {
            return;
        };
        match job.done.try_recv() {
            Ok(text) => {
                self.clipboard = Some(text);
                self.notice = Some(Notice::Copied { rows: job.rows });
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                tracing::error!("thread di copia interrotto");
                self.notice = Some(Notice::NoThread);
            }
        }
        self.copy_job = None;
    }

    /// Le ultime scelte di esportazione.
    pub fn export_settings(&self) -> &ExportSettings {
        &self.settings.export
    }

    /// Cambia le scelte di esportazione (salvate con le impostazioni).
    pub fn set_export_settings(&mut self, settings: ExportSettings) {
        self.settings.export = settings;
    }

    /// Righe che l'esportazione scriverebbe con `scope`.
    pub fn export_row_count(&self, scope: ExportScope) -> usize {
        match scope {
            ExportScope::Visible => self.view.len(),
            ExportScope::Selected => self.view.selected_count(),
        }
    }

    /// Vero se un'esportazione sta scrivendo.
    pub fn is_exporting(&self) -> bool {
        self.export_job.is_some()
    }

    /// Esporta in `path` le righe e nel formato delle ultime scelte, in un thread; ricorda
    /// la cartella. La conferma della sovrascrittura l'ha già chiesta il dialogo di
    /// salvataggio; la tabella si costruisce e si scrive nel thread, in modo atomico: il
    /// file precedente resta se fallisce.
    pub fn export_to(&mut self, path: PathBuf, notify: &Notify) {
        let settings = self.settings.export.clone();
        let indices: Vec<usize> = match settings.scope {
            ExportScope::Visible => (0..self.view.len())
                .filter_map(|p| self.view.hit_at(p))
                .collect(),
            ExportScope::Selected => self.view.selected_hits().collect(),
        };
        let rows = indices.len();
        let hits = Arc::clone(&self.results.hits);
        let terms = self.results.terms.clone();
        let i18n = self.i18n.clone();
        let language = self.i18n.language();
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            self.settings.export.last_dir = Some(dir.to_owned());
        }
        let file = path.display().to_string();
        let (tx, rx) = mpsc::channel();
        let notify = Arc::clone(notify);
        let spawned = thread::Builder::new()
            .name("esportazione".to_owned())
            .spawn(move || {
                let table = export::results_table(&i18n, &hits, &terms, indices);
                drop(hits);
                let bytes = export::encode(&table, settings.format, language);
                let written = export::write_export(&path, &bytes);
                if tx.send(written).is_ok() {
                    notify();
                }
            });
        match spawned {
            Ok(_) => {
                self.export_job = Some(ExportJob {
                    rows,
                    file,
                    done: rx,
                });
            }
            Err(_) => self.notice = Some(Notice::NoThread),
        }
    }

    fn poll_export(&mut self) {
        let Some(job) = &self.export_job else {
            return;
        };
        let written = match job.done.try_recv() {
            Ok(written) => written,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(io::Error::other("thread interrotto")),
        };
        let file = job.file.clone();
        self.notice = Some(match written {
            Ok(()) => Notice::Exported {
                rows: job.rows,
                file,
            },
            Err(_) => Notice::ExportFailed { file },
        });
        self.export_job = None;
    }

    // --- Processi estrattori e cache ---------------------------------------------------

    /// Collega i documenti: pool dei processi estrattori (`setup.worker_exe` con
    /// `--extract-worker`), cache e apertura delle voci d'archivio. Le ricerche successive
    /// leggono anche PDF, Office e archivi; l'anteprima e "Apri" li usano. Nella cartella
    /// dei file aperti si cancellano subito, in un thread, le sessioni finite (avanzi di
    /// un'uscita anomala); quelle di altre istanze in corso restano.
    ///
    /// Errore se il lanciatore non si può creare (eseguibile senza percorso assoluto): l'app
    /// resta senza documenti.
    pub fn connect_extraction(&mut self, setup: &ExtractionSetup) -> io::Result<()> {
        let source = extraction::document_source(setup, self.settings.cache, &self.private_fs)?;
        self.documents = Arc::new(PoolDocumentUnits::new(Arc::clone(&source), self.limits));
        if let Some(dir) = &setup.opened_entries_dir {
            let opened = Arc::new(OpenedEntries::new(
                Arc::clone(&source),
                dir.clone(),
                self.limits,
                Arc::clone(&self.private_fs),
            ));
            self.archive_extractor = Arc::clone(&opened) as Arc<dyn ArchiveEntryExtractor>;
            self.opened_entries = Some(opened);
            let dir = dir.clone();
            // Solo le sessioni finite: quelle di altre istanze in corso restano. Se il thread
            // non parte, ci penserà un avvio successivo.
            let _cleaning = thread::Builder::new()
                .name("pulizia".to_owned())
                .spawn(move || extraction::clean_stale_sessions(&dir));
        }
        self.document_source = Some(source);
        Ok(())
    }

    /// La fonte dei documenti collegata, se c'è.
    pub fn document_source(&self) -> Option<&Arc<DocumentSource>> {
        self.document_source.as_ref()
    }

    /// Chiude i processi estrattori liberi e cancella la sottocartella dei file aperti di
    /// questa sessione (all'uscita).
    pub fn shutdown_extraction(&mut self) {
        if let Some(source) = &self.document_source {
            source.pool().shutdown();
        }
        if let Some(opened) = &self.opened_entries {
            opened.close_session();
        }
    }

    /// Le impostazioni della cache.
    pub fn cache_settings(&self) -> CacheSettings {
        self.settings.cache
    }

    /// Vero se c'è una cartella per la cache (si può accendere e svuotare).
    pub fn has_cache(&self) -> bool {
        self.document_source
            .as_ref()
            .is_some_and(|source| source.cache().is_some())
    }

    /// Accende o spegne la cache (le voci restano: si svuota a parte).
    pub fn set_cache_enabled(&mut self, enabled: bool) {
        self.settings.cache.enabled = enabled;
        if let Some(source) = &self.document_source {
            source.set_cache_enabled(enabled);
        }
    }

    /// Cambia la dimensione massima della cache (riportata all'intervallo ammesso); le voci
    /// in più si tolgono subito, in un thread.
    pub fn set_cache_size_mib(&mut self, mib: u64) {
        let mib = mib.clamp(*CACHE_SIZE_MIB_RANGE.start(), *CACHE_SIZE_MIB_RANGE.end());
        self.settings.cache.max_size_mib = mib;
        if let Some(source) = self.document_source.clone() {
            let _resizing = thread::Builder::new()
                .name("cache".to_owned())
                .spawn(move || {
                    if let Some(cache) = source.cache() {
                        cache.set_max_size(ByteSize::from_mib(mib));
                    }
                });
        }
    }

    /// Vero se la cache si sta svuotando.
    pub fn is_clearing_cache(&self) -> bool {
        self.cache_job.is_some()
    }

    /// Svuota la cache in un thread; l'esito arriva come messaggio.
    pub fn clear_cache(&mut self, notify: &Notify) {
        let Some(source) = self.document_source.clone() else {
            return;
        };
        if source.cache().is_none() || self.cache_job.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let notify = Arc::clone(notify);
        let spawned = thread::Builder::new()
            .name("cache".to_owned())
            .spawn(move || {
                let cleared = source.cache().map_or(
                    Ok(ByteSize::from_bytes(0)),
                    lynkeus_engine::TextCache::clear,
                );
                if tx.send(cleared).is_ok() {
                    notify();
                }
            });
        match spawned {
            Ok(_) => self.cache_job = Some(rx),
            Err(_) => self.notice = Some(Notice::NoThread),
        }
    }

    fn poll_cache(&mut self) {
        let Some(job) = &self.cache_job else {
            return;
        };
        let cleared = match job.try_recv() {
            Ok(cleared) => cleared,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(io::Error::other("thread interrotto")),
        };
        self.notice = Some(match cleared {
            Ok(freed) => Notice::CacheCleared {
                freed_mib: freed.bytes().div_ceil(1024 * 1024),
            },
            Err(_) => Notice::CacheClearFailed,
        });
        self.cache_job = None;
    }

    // --- Messaggi -----------------------------------------------------------------------

    /// Il messaggio da mostrare, se c'è.
    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    /// Mostra un messaggio al posto di quello attuale.
    pub fn set_notice(&mut self, notice: Notice) {
        self.notice = Some(notice);
    }

    /// Chiude il messaggio.
    pub fn dismiss_notice(&mut self) {
        self.notice = None;
    }

    /// Raccoglie, senza attendere, l'esito delle aperture, delle copie, delle esportazioni,
    /// dei dialoghi, dei salvataggi e delle modifiche al clic destro finite.
    pub fn pump_actions(&mut self) {
        self.poll_launch();
        self.poll_copy();
        self.poll_export();
        self.poll_dialog();
        self.poll_cache();
        self.poll_context_menu();
        self.poll_save();
    }

    /// Riporta i filtri ai predefiniti, senza conferma: il messaggio offre "Annulla".
    pub fn reset_filters(&mut self) {
        let previous = Box::new(self.form.filters.clone());
        self.form.filters.reset();
        self.notice = Some(Notice::FiltersReset { previous });
    }

    /// Rimette i filtri di prima dell'ultimo ripristino, se il suo messaggio è ancora
    /// quello mostrato.
    pub fn undo_filters_reset(&mut self) {
        match self.notice.take() {
            Some(Notice::FiltersReset { previous }) => self.form.filters = *previous,
            other => self.notice = other,
        }
    }

    // --- Dialoghi di sistema ------------------------------------------------------------

    /// Un dialogo di sistema è aperto in un thread: la scelta arriverà su `done` (`None` =
    /// annullato) e si applica da sola (luoghi aggiunti, esportazione avviata). Intanto la
    /// finestra si ridisegna e la ricerca continua.
    pub fn wait_for_dialog(&mut self, done: Receiver<Option<FileChoice>>) {
        self.dialog_job = Some(done);
    }

    /// Vero se un dialogo di sistema è aperto.
    pub fn is_dialog_open(&self) -> bool {
        self.dialog_job.is_some()
    }

    fn poll_dialog(&mut self) {
        let Some(done) = &self.dialog_job else {
            return;
        };
        let choice = match done.try_recv() {
            Ok(choice) => choice,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                tracing::error!("thread del dialogo interrotto");
                None
            }
        };
        self.dialog_job = None;
        match choice {
            Some(FileChoice::AddRoots(paths)) => {
                for path in paths {
                    self.add_root(path);
                }
            }
            Some(FileChoice::ExportTo(path)) => {
                let waker = Arc::clone(&self.waker);
                self.export_to(path, &waker);
            }
            None => {}
        }
    }

    // --- Avvio con `--add` e istanza unica ----------------------------------------------

    /// Sostituisce le radici con `paths` (avvio con `--add`, percorsi già accettati dal
    /// guardiano unico): la finestra si apre con quei luoghi, senza avviare la ricerca.
    pub fn open_with_roots(&mut self, paths: Vec<PathBuf>) {
        self.form.roots.clear();
        self.add_external_roots(paths);
    }

    /// Aggiunge `paths` (già accettati) all'elenco dei luoghi, senza avviare la ricerca e
    /// senza aprirli; vero se ne è arrivato almeno uno.
    pub fn add_external_roots(&mut self, paths: Vec<PathBuf>) -> bool {
        let arrived = !paths.is_empty();
        for path in paths {
            self.add_root(path);
        }
        arrived
    }

    /// Questa è la prima istanza: riceve i percorsi degli avvii successivi.
    pub fn attach_instance(&mut self, server: InstanceServer) {
        self.instance = Some(server);
    }

    /// Chi avvisare (un nuovo frame) quando altre istanze mandano percorsi.
    pub fn set_instance_waker(&self, waker: Notify) {
        if let Some(server) = &self.instance {
            server.set_waker(waker);
        }
    }

    /// Aggiunge i percorsi arrivati dalle altre istanze; vero se ne è arrivato qualcuno (la
    /// vista porta la finestra in primo piano).
    pub fn pump_instance(&mut self) -> bool {
        let paths = self
            .instance
            .as_ref()
            .map(InstanceServer::take_paths)
            .unwrap_or_default();
        self.add_external_roots(paths)
    }

    /// Chiude il canale dell'istanza unica (all'uscita).
    pub fn shutdown_instance(&mut self) {
        self.instance = None;
    }

    // --- Clic destro --------------------------------------------------------------------

    /// Collega la voce del clic destro: `store` (registro o script) per l'eseguibile `exe`.
    pub fn connect_context_menu(&mut self, store: Arc<dyn ContextMenuStore>, exe: PathBuf) {
        self.context_menu = Some(ContextMenuSetup { store, exe });
    }

    /// Vero se la voce del clic destro è accesa nelle impostazioni.
    pub fn context_menu_enabled(&self) -> bool {
        self.settings.context_menu
    }

    /// Vero se si può cambiare la voce (collegata e nessuna modifica in corso).
    pub fn can_change_context_menu(&self) -> bool {
        self.context_menu.is_some() && self.context_menu_job.is_none()
    }

    /// La voce per l'eseguibile collegato, con il testo nella lingua attuale.
    fn menu_entry(&self, exe: &Path) -> Result<MenuEntry, MenuError> {
        let label = self.i18n.tr("context-menu-label");
        let others: Vec<String> = Language::ALL
            .iter()
            .map(|language| I18n::new(*language).tr("context-menu-label"))
            .collect();
        MenuEntry::new(exe, &label, &others)
    }

    /// Accende o spegne la voce del clic destro, in un thread. L'impostazione cambia solo
    /// se l'operazione riesce; altrimenti un messaggio dice perché.
    pub fn set_context_menu(&mut self, enabled: bool, notify: &Notify) {
        self.start_context_menu_job(enabled, notify);
    }

    /// All'avvio, con l'opzione accesa: riscrive la voce se manca o è cambiata (eseguibile
    /// spostato, altra lingua). Una voce altrui o una cartella non sicura dà un messaggio.
    pub fn refresh_context_menu(&mut self, notify: &Notify) {
        if self.settings.context_menu {
            self.start_context_menu_job(true, notify);
        }
    }

    fn start_context_menu_job(&mut self, enabled: bool, notify: &Notify) {
        if self.context_menu_job.is_some() {
            return;
        }
        let Some(setup) = &self.context_menu else {
            return;
        };
        let entry = match self.menu_entry(&setup.exe) {
            Ok(entry) => entry,
            Err(error) => {
                self.notice = Some(Notice::ContextMenuFailed(error));
                return;
            }
        };
        let store = Arc::clone(&setup.store);
        let (tx, rx) = mpsc::channel();
        let notify = Arc::clone(notify);
        let spawned = thread::Builder::new()
            .name("clic-destro".to_owned())
            .spawn(move || {
                let result = if enabled {
                    context_menu::refresh(store.as_ref(), &entry).map(|_| ())
                } else {
                    context_menu::disable(store.as_ref(), &entry)
                };
                if tx.send(result).is_ok() {
                    notify();
                }
            });
        match spawned {
            Ok(_) => self.context_menu_job = Some(ContextMenuJob { enabled, done: rx }),
            Err(_) => self.notice = Some(Notice::NoThread),
        }
    }

    /// Vero se la voce del clic destro si sta modificando.
    pub fn is_updating_context_menu(&self) -> bool {
        self.context_menu_job.is_some()
    }

    fn poll_context_menu(&mut self) {
        let Some(job) = &self.context_menu_job else {
            return;
        };
        let result = match job.done.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(MenuError::Io(io::ErrorKind::Other)),
        };
        match result {
            Ok(()) => {
                self.settings.context_menu = job.enabled;
                tracing::info!(enabled = job.enabled, "voce del clic destro aggiornata");
            }
            Err(error) => {
                tracing::warn!(%error, enabled = job.enabled, "voce del clic destro non aggiornata");
                self.notice = Some(Notice::ContextMenuFailed(error));
            }
        }
        self.context_menu_job = None;
    }

    // --- Diagnostica --------------------------------------------------------------------

    /// Collega il log installato nel `main`, per cambiarne il livello.
    pub fn set_log_control(&mut self, control: LogControl) {
        control.set_level(self.settings.log_level);
        self.log_control = Some(control);
    }

    /// Il livello del log.
    pub fn log_level(&self) -> LogLevel {
        self.settings.log_level
    }

    /// Cambia il livello del log (subito e nelle impostazioni).
    pub fn set_log_level(&mut self, level: LogLevel) {
        self.settings.log_level = level;
        if let Some(control) = &self.log_control {
            control.set_level(level);
        }
    }

    /// Salva le impostazioni se sono cambiate dall'ultimo salvataggio, in un thread: il
    /// thread dell'interfaccia non aspetta il disco. L'esito arriva con
    /// [`Application::pump_actions`] e un nuovo frame lo chiede chi è stato dato a
    /// [`Application::set_waker`]; un errore si mostra una volta. Senza file (test), con
    /// un salvataggio in corso o con il file bloccato dall'avvio non fa nulla.
    pub fn save_settings_in_background(&mut self) {
        if self.save_job.is_some() {
            return;
        }
        let current = self.settings();
        let Some(file) = &self.settings_file else {
            return;
        };
        if self.saved.as_ref() == Some(&current) {
            return;
        }
        // Il file più recente o illeggibile non si riscrive: lo dice già il messaggio
        // dell'avvio.
        if self.settings_blocked {
            return;
        }
        let file = Arc::clone(file);
        let (tx, rx) = mpsc::channel();
        let notify = Arc::clone(&self.waker);
        let spawned = thread::Builder::new()
            .name("impostazioni".to_owned())
            .spawn(move || {
                let saved = save_to(&file, &current);
                if tx.send((current, saved)).is_ok() {
                    notify();
                }
            });
        match spawned {
            Ok(_) => self.save_job = Some(SaveJob { done: rx }),
            Err(error) => tracing::warn!(%error, "thread delle impostazioni non avviato"),
        }
    }

    /// Vero se le impostazioni si stanno scrivendo.
    pub fn is_saving_settings(&self) -> bool {
        self.save_job.is_some()
    }

    fn poll_save(&mut self) {
        let Some(job) = &self.save_job else {
            return;
        };
        let outcome = match job.done.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                tracing::error!("thread delle impostazioni interrotto");
                self.save_job = None;
                return;
            }
        };
        self.save_job = None;
        self.record_save(outcome);
    }

    /// Registra l'esito di un salvataggio: le impostazioni scritte, o l'errore (nel log e,
    /// la prima volta, in un messaggio).
    fn record_save(&mut self, (written, saved): (Settings, Result<(), SettingsError>)) {
        match saved {
            Ok(()) => self.saved = Some(written),
            Err(error) => {
                tracing::warn!(%error, "impostazioni non salvate");
                if !std::mem::replace(&mut self.save_failure_reported, true) {
                    self.notice = Some(Notice::SettingsNotSaved(error.problem()));
                }
            }
        }
    }

    /// Salva le impostazioni subito, nel thread che chiama (all'uscita), dopo aver atteso
    /// per un tempo limitato un salvataggio in corso. Senza file (test) non fa nulla.
    pub fn save_settings(&mut self) -> Result<(), SettingsError> {
        if let Some(job) = self.save_job.take() {
            match job.done.recv_timeout(SETTINGS_SAVE_WAIT) {
                Ok(outcome) => self.record_save(outcome),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    tracing::warn!("salvataggio delle impostazioni in corso non concluso");
                }
            }
        }
        let current = self.settings();
        let Some(file) = &self.settings_file else {
            return Ok(());
        };
        if self.saved.as_ref() == Some(&current) {
            return Ok(());
        }
        save_to(file, &current)?;
        self.saved = Some(current);
        Ok(())
    }

    /// Ricorda radici e termini della ricerca appena avviata tra i recenti.
    fn remember_recent(&mut self) {
        for root in self.results.roots.iter().rev() {
            push_recent(&mut self.settings.recent_roots, root.clone());
        }
        for term in self.results.terms.iter().rev() {
            push_recent(&mut self.settings.recent_terms, term.clone());
        }
    }
}

fn empty_state(has_hits: bool, has_roots: bool, job: &JobState) -> EmptyState {
    match job {
        _ if has_hits => EmptyState::None,
        JobState::Running(_) => EmptyState::Searching,
        JobState::Finished(summary) => match summary.outcome {
            Outcome::Cancelled => EmptyState::Cancelled,
            Outcome::Completed | Outcome::LimitReached => EmptyState::NothingFound,
        },
        JobState::WorkerLost => EmptyState::Interrupted,
        JobState::Idle if has_roots => EmptyState::WriteTerm,
        JobState::Idle => EmptyState::AddRoot,
    }
}

/// Scrive `settings` nel file condiviso; un altro thread che lo scriveva e si è fermato a
/// metà (lock avvelenato) rende il file inutilizzabile per la sessione.
fn save_to(file: &Mutex<SettingsFile>, settings: &Settings) -> Result<(), SettingsError> {
    let mut file = file
        .lock()
        .map_err(|_| SettingsError::Io(io::Error::other("scrittura precedente interrotta")))?;
    file.save(settings)
}

#[cfg(test)]
mod tests {
    use lynkeus_engine::Outcome;

    use super::*;

    fn finished(outcome: Outcome) -> JobState {
        JobState::Finished(Summary {
            outcome,
            elapsed: Duration::ZERO,
            counts: ProgressSnapshot::default(),
        })
    }

    /// T-C10 (C-10): ogni fine senza righe ha il suo stato vuoto; "nessuna parola
    /// trovata" solo se la ricerca è arrivata in fondo.
    #[test]
    fn empty_state_partitions() {
        assert_eq!(
            empty_state(false, false, &JobState::Idle),
            EmptyState::AddRoot
        );
        assert_eq!(
            empty_state(false, true, &JobState::Idle),
            EmptyState::WriteTerm
        );
        assert_eq!(
            empty_state(false, true, &finished(Outcome::Completed)),
            EmptyState::NothingFound
        );
        assert_eq!(
            empty_state(false, true, &finished(Outcome::Cancelled)),
            EmptyState::Cancelled
        );
        assert_eq!(
            empty_state(false, true, &JobState::WorkerLost),
            EmptyState::Interrupted
        );
        assert_eq!(
            empty_state(true, true, &finished(Outcome::Completed)),
            EmptyState::None
        );
        assert_eq!(
            empty_state(true, true, &finished(Outcome::Cancelled)),
            EmptyState::None
        );
    }

    /// T-C08 (C-06): il motore non conosce il totale dei file finché la scansione non
    /// finisce: l'avanzamento è indeterminato, con i conteggi, mai una percentuale.
    #[test]
    fn progress_indeterminate_while_walking() {
        let counts = ProgressSnapshot {
            files_seen: 40,
            files_searched: 30,
            files_skipped: 2,
            binary_skipped: 1,
            warnings: 1,
            ..ProgressSnapshot::default()
        };
        let running = |elapsed| SearchStatus::Running { counts, elapsed };
        assert_eq!(running(Duration::from_millis(999)).progress(), None);
        assert_eq!(
            running(PROGRESS_DELAY).progress(),
            Some(Progress::Indeterminate { done: 34, seen: 40 })
        );
        assert_eq!(SearchStatus::Idle.progress(), None);
        assert_eq!(SearchStatus::WorkerLost.progress(), None);
    }

    /// C-04: un modulo non valido mostra il pannello, apre i Filtri se serve e chiede il
    /// focus per il primo campo da correggere.
    #[test]
    fn a_form_error_reveals_the_panel_and_the_field() {
        let mut app = Application::build(Settings::default()).expect("app");
        app.form.add_root(missing_root(), RootKind::Missing);
        app.set_term("ar");
        app.form.filters.extension_mode = crate::settings::ExtensionMode::Include;
        app.layout.search_panel_visible = false;
        assert!(app.start_search(Arc::new(|| {})).is_err());
        assert!(app.layout.search_panel_visible);
        assert!(app.view_state.open_filters);
        assert_eq!(app.view_state.focus_field, Some(FormField::Extensions));

        let mut app = Application::build(Settings::default()).expect("app");
        app.form.add_root(missing_root(), RootKind::Missing);
        app.set_terms(&["ar", "**"]);
        assert!(app.start_search(Arc::new(|| {})).is_err());
        assert!(!app.view_state.open_filters, "i Filtri non c'entrano");
        assert_eq!(app.view_state.focus_field, Some(FormField::Term(1)));
    }

    /// C-14: "Ripristina i filtri predefiniti" non chiede conferma e si annulla.
    #[test]
    fn reset_filters_can_be_undone() {
        let mut app = Application::build(Settings::default()).expect("app");
        app.form.filters.extension_mode = crate::settings::ExtensionMode::Exclude;
        app.form.filters.extensions = "log".to_owned();
        let customized = app.form.filters.clone();
        app.reset_filters();
        assert!(!app.form.filters.is_customized());
        assert!(matches!(app.notice(), Some(Notice::FiltersReset { .. })));
        app.undo_filters_reset();
        assert_eq!(app.form.filters, customized);
        assert!(app.notice().is_none());
        // Senza un ripristino da annullare non cambia nulla.
        app.undo_filters_reset();
        assert_eq!(app.form.filters, customized);
    }

    /// T-C13 (C-11): un settings.toml più recente o illeggibile si dice all'utente, perché
    /// le modifiche della sessione non si salveranno.
    #[test]
    fn settings_newer_version_is_reported() {
        let dir = tempfile::tempdir().expect("cartella");
        let path = dir.path().join(crate::settings::FILE_NAME);
        std::fs::write(&path, "format_version = 99\n").expect("scritto");
        let (file, settings, _) = SettingsFile::load(path.clone());
        let app = Application::build_persistent(settings, file).expect("app");
        assert_eq!(
            app.notice(),
            Some(&Notice::SettingsNotSaved(SettingsProblem::NewerVersion))
        );

        let unreadable = dir.path().join("cartella-al-posto-del-file");
        std::fs::create_dir(&unreadable).expect("cartella");
        let (file, settings, _) = SettingsFile::load(unreadable);
        let app = Application::build_persistent(settings, file).expect("app");
        assert_eq!(
            app.notice(),
            Some(&Notice::SettingsNotSaved(SettingsProblem::Unreadable))
        );

        std::fs::write(&path, "non è toml [[[").expect("scritto");
        let (file, settings, _) = SettingsFile::load(path.clone());
        let app = Application::build_persistent(settings, file).expect("app");
        assert_eq!(app.notice(), Some(&Notice::SettingsReplaced));
    }

    /// C-11: il salvataggio periodico scrive in un thread; la vista raccoglie l'esito.
    #[test]
    fn settings_are_saved_off_the_ui_thread() {
        let dir = tempfile::tempdir().expect("cartella");
        let path = dir.path().join(crate::settings::FILE_NAME);
        let (file, settings, _) = SettingsFile::load(path.clone());
        let mut app = Application::build_persistent(settings, file).expect("app");
        app.set_theme(ThemeChoice::Dark);
        app.save_settings_in_background();
        assert!(app.is_saving_settings());
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.is_saving_settings() {
            assert!(Instant::now() < deadline, "salvataggio non finito");
            app.pump_actions();
            thread::yield_now();
        }
        let written = std::fs::read_to_string(&path).expect("letto");
        assert!(written.contains("dark"), "{written}");
        // Nulla di cambiato: nessun altro salvataggio.
        app.save_settings_in_background();
        assert!(!app.is_saving_settings());
    }

    /// Un file con `lines` righe "ar": tante parole trovate in un file solo.
    fn many_words_file(dir: &Path, lines: usize) -> PathBuf {
        let path = dir.join("parole.txt");
        std::fs::write(&path, "ar\n".repeat(lines)).expect("scritto");
        path
    }

    /// C-07: la tabella da copiare si costruisce in un thread; il testo arriva agli
    /// appunti dalla vista, poi il messaggio.
    #[test]
    fn copy_is_built_off_the_ui_thread() {
        let dir = tempfile::tempdir().expect("cartella");
        let file = many_words_file(dir.path(), 3);
        let mut app = Application::build(Settings::default()).expect("app");
        app.form.add_root(file, RootKind::File);
        app.set_term("ar");
        run_to_end(&mut app);
        app.view_mut().select_all();
        app.copy_selection(&(Arc::new(|| {}) as Notify));
        assert!(app.is_copying());
        let deadline = Instant::now() + Duration::from_secs(10);
        let text = loop {
            app.pump_actions();
            if let Some(text) = app.take_clipboard_text() {
                break text;
            }
            assert!(Instant::now() < deadline, "copia non finita");
            thread::yield_now();
        };
        assert_eq!(text.lines().count(), 3, "{text}");
        assert_eq!(app.notice(), Some(&Notice::Copied { rows: 3 }));
        assert!(!app.is_copying());
    }

    /// Misura P-08 (C-07): `cargo test --release -p lynkeus-app --lib copy_200k --
    /// --ignored --nocapture`. Con 200 000 righe selezionate, la copia occupa il thread
    /// dell'interfaccia per meno di 50 ms (il resto è nel thread di copia).
    #[test]
    #[ignore = "misura di prestazioni, da eseguire in release"]
    fn copy_200k() {
        let dir = tempfile::tempdir().expect("cartella");
        let file = many_words_file(dir.path(), 200_000);
        let mut app = Application::build(Settings::default()).expect("app");
        app.form.add_root(file, RootKind::File);
        app.set_term("ar");
        run_to_end(&mut app);
        assert_eq!(app.results().hits.len(), 200_000);
        app.view_mut().select_all();
        let notify: Notify = Arc::new(|| {});
        let started = Instant::now();
        app.copy_selection(&notify);
        let on_ui_thread = started.elapsed();
        let mut longest_pump = Duration::ZERO;
        let text = loop {
            let pump = Instant::now();
            app.pump_actions();
            let taken = app.take_clipboard_text();
            longest_pump = longest_pump.max(pump.elapsed());
            if let Some(text) = taken {
                break text;
            }
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "copia non finita"
            );
            thread::sleep(Duration::from_millis(1));
        };
        println!(
            "copy_200k: thread dell'interfaccia {on_ui_thread:?} all'avvio, \
             {longest_pump:?} al massimo per frame, totale {:?}, {} byte",
            started.elapsed(),
            text.len()
        );
        assert!(on_ui_thread < Duration::from_millis(50), "{on_ui_thread:?}");
        assert!(longest_pump < Duration::from_millis(50), "{longest_pump:?}");
    }

    /// Una radice che non esiste: la ricerca finisce subito con un avviso.
    fn missing_root() -> PathBuf {
        std::env::temp_dir().join("lynkeus-non-esiste-7f3a9c")
    }

    /// Avvia la ricerca e legge gli eventi fino alla fine.
    fn run_to_end(app: &mut Application) {
        app.start_search(Arc::new(|| {})).expect("ricerca avviata");
        let deadline = Instant::now() + Duration::from_secs(30);
        while app.pump_events(usize::MAX) != Pump::Ended {
            assert!(Instant::now() < deadline, "ricerca non finita");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Un percorso relativo diventa assoluto appena entra nell'elenco delle radici: le
    /// ricerche, la cache e il processo estrattore (con un'altra cartella corrente) vedono
    /// tutti lo stesso file.
    #[test]
    fn relative_roots_become_absolute() {
        let mut app = Application::build(Settings::default()).expect("app");
        let absolute_src = std::path::absolute("src").expect("assoluto");
        app.form.path_input = "src".to_owned();
        app.add_typed_path();
        assert_eq!(
            app.form.path_status,
            PathInputStatus::Checking(absolute_src.clone())
        );
        settle_root_checks(&mut app);
        assert_eq!(app.form.roots.len(), 1);
        assert_eq!(app.form.roots[0].path, absolute_src);
        assert_eq!(app.form.roots[0].kind, RootKind::Folder);

        assert!(app.add_root(PathBuf::from("Cargo.toml")));
        let absolute_file = std::path::absolute("Cargo.toml").expect("assoluto");
        assert_eq!(app.form.roots[1].path, absolute_file);
        // Lo stesso percorso scritto in modo diverso non si aggiunge due volte.
        assert!(!app.add_root(absolute_file));
        settle_root_checks(&mut app);
        assert_eq!(app.form.roots[1].kind, RootKind::File);
    }

    /// Con l'elenco pieno, i luoghi in arrivo (clic destro, dialoghi) non entrano e un
    /// avviso lo dice; il percorso scritto mostra l'errore sotto il campo.
    #[test]
    fn a_full_list_of_places_refuses_more_with_a_notice() {
        let mut app = Application::build(Settings::default()).expect("app");
        for i in 0..MAX_ROOTS {
            app.form
                .add_root(PathBuf::from(format!("luogo-{i}")), RootKind::Folder);
        }
        assert!(app.notice().is_none());
        assert!(app.add_external_roots(vec![std::path::absolute("src").expect("assoluto")]));
        assert_eq!(app.form.roots.len(), MAX_ROOTS);
        assert!(matches!(
            app.notice(),
            Some(Notice::RootsLimit { max: MAX_ROOTS })
        ));
        app.form.path_input = "Cargo.toml".to_owned();
        app.add_typed_path();
        assert_eq!(
            app.form.path_status,
            PathInputStatus::Error(PathInputError::TooManyRoots)
        );
    }

    #[test]
    fn a_refused_start_keeps_the_previous_search() {
        let mut app = Application::build(Settings::default()).expect("app");
        app.form.add_root(missing_root(), RootKind::Missing);
        app.set_term("ar");
        run_to_end(&mut app);
        let before = app.status();
        assert!(matches!(before, SearchStatus::Finished(_)));
        let generation = app.results().generation;

        app.set_term("  ");
        assert!(app.start_search(Arc::new(|| {})).is_err());
        assert_eq!(
            app.status(),
            before,
            "lo stato della ricerca precedente resta"
        );
        assert_eq!(app.results().generation, generation);
    }

    #[test]
    fn recent_terms_are_trimmed_and_unique() {
        let mut app = Application::build(Settings::default()).expect("app");
        app.form.add_root(missing_root(), RootKind::Missing);
        app.set_terms(&[" ar", "ar  ", "carta"]);
        run_to_end(&mut app);
        assert_eq!(app.recent_terms(), ["ar", "carta"]);
        assert_eq!(app.results().terms, ["ar", "ar", "carta"]);
    }

    /// Legge le risposte delle verifiche delle radici finché non ne restano.
    fn settle_root_checks(app: &mut Application) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            app.pump_root_checks();
            if !app.root_checks_pending() {
                return;
            }
            assert!(Instant::now() < deadline, "verifiche senza risposta");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn typed_path_must_exist() {
        let mut app = Application::build(Settings::default()).expect("app");
        app.form.path_input = "  ".to_owned();
        app.add_typed_path();
        assert!(app.form.roots.is_empty());
        assert_eq!(app.form.path_status, PathInputStatus::Idle);

        // Il percorso si verifica fuori dal thread dell'interfaccia: prima "in verifica".
        let missing = missing_root();
        app.form.path_input = missing.display().to_string();
        app.add_typed_path();
        assert_eq!(app.form.path_status, PathInputStatus::Checking(missing));
        settle_root_checks(&mut app);
        assert_eq!(
            app.form.path_status,
            PathInputStatus::Error(PathInputError::NotFound)
        );
        assert!(app.form.roots.is_empty());

        let here = env!("CARGO_MANIFEST_DIR");
        app.form.path_input = format!("\"{here}\"");
        app.add_typed_path();
        assert!(app.form.roots.is_empty(), "aggiunta solo dopo la verifica");
        settle_root_checks(&mut app);
        assert_eq!(app.form.path_status, PathInputStatus::Idle);
        assert_eq!(app.form.roots.len(), 1);
        assert_eq!(app.form.roots[0].kind, RootKind::Folder);
        assert!(app.form.path_input.is_empty());
    }

    #[test]
    fn a_typed_path_edited_during_the_check_is_left_alone() {
        let mut app = Application::build(Settings::default()).expect("app");
        app.form.path_input = env!("CARGO_MANIFEST_DIR").to_owned();
        app.add_typed_path();
        // L'utente cambia il campo: la vista rimette lo stato a riposo.
        app.form.path_input = "altro".to_owned();
        app.form.path_status = PathInputStatus::Idle;
        settle_root_checks(&mut app);
        assert!(app.form.roots.is_empty());
        assert_eq!(app.form.path_input, "altro");
    }

    #[test]
    fn roots_are_pending_until_the_disk_answers() {
        let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let settings = Settings {
            roots: vec![here.clone(), missing_root()],
            ..Settings::default()
        };
        let mut app = Application::build(settings).expect("app");
        let kinds = |app: &Application| app.form.roots.iter().map(|r| r.kind).collect::<Vec<_>>();
        assert_eq!(kinds(&app), [RootKind::Pending, RootKind::Pending]);
        assert!(app.add_root(here.join("Cargo.toml")));
        assert_eq!(app.form.roots[2].kind, RootKind::Pending);
        settle_root_checks(&mut app);
        assert_eq!(
            kinds(&app),
            [RootKind::Folder, RootKind::Missing, RootKind::File]
        );
    }

    #[test]
    fn cache_settings_without_a_document_source() {
        let mut app = Application::build(Settings::default()).expect("app");
        assert!(!app.has_cache(), "nessuna cartella senza collegamento");
        app.set_cache_enabled(false);
        app.set_cache_size_mib(1);
        assert_eq!(
            app.cache_settings(),
            CacheSettings {
                enabled: false,
                max_size_mib: *CACHE_SIZE_MIB_RANGE.start(),
            }
        );
        app.set_cache_size_mib(u64::MAX);
        assert_eq!(
            app.settings().cache.max_size_mib,
            *CACHE_SIZE_MIB_RANGE.end()
        );
        // Svuotare senza cache non fa nulla.
        app.clear_cache(&(Arc::new(|| {}) as Notify));
        assert!(!app.is_clearing_cache());
    }

    /// Un messaggio valido dell'istanza unica aggiunge luoghi senza avviare la ricerca.
    #[test]
    fn external_roots_are_added_without_searching() {
        let mut app = Application::build(Settings::default()).expect("app");
        app.open_with_roots(vec![missing_root()]);
        assert_eq!(app.form.roots.len(), 1);
        let other = std::env::temp_dir();
        assert!(app.add_external_roots(vec![other.clone()]));
        assert!(!app.add_external_roots(Vec::new()));
        assert_eq!(app.form.roots.len(), 2);
        assert!(!app.is_searching());
        assert!(matches!(app.status(), SearchStatus::Idle));
        // Senza istanza unica non arriva nulla.
        assert!(!app.pump_instance());
    }

    /// La voce del clic destro: l'impostazione cambia solo se l'operazione riesce.
    #[test]
    fn the_context_menu_setting_follows_the_outcome() {
        use std::sync::atomic::AtomicBool;

        use crate::context_menu::MenuState;

        struct Store {
            writable: AtomicBool,
        }
        impl ContextMenuStore for Store {
            fn state(&self, _entry: &MenuEntry) -> io::Result<MenuState> {
                Ok(MenuState::Missing)
            }
            fn install(&self, _entry: &MenuEntry) -> io::Result<()> {
                Ok(())
            }
            fn remove(&self, _entry: &MenuEntry) -> io::Result<()> {
                Ok(())
            }
            fn exe_writable_by_others(&self, _exe: &Path) -> io::Result<bool> {
                Ok(self.writable.load(Ordering::Relaxed))
            }
        }

        let wait = |app: &mut Application| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.is_updating_context_menu() && Instant::now() < deadline {
                app.pump_actions();
                thread::yield_now();
            }
        };
        let notify: Notify = Arc::new(|| {});
        let mut app = Application::build(Settings::default()).expect("app");
        assert!(!app.can_change_context_menu());
        let store = Arc::new(Store {
            writable: AtomicBool::new(true),
        });
        let exe = std::env::temp_dir().join("lynkeus-searcher.exe");
        app.connect_context_menu(Arc::clone(&store) as Arc<dyn ContextMenuStore>, exe);
        app.set_context_menu(true, &notify);
        wait(&mut app);
        assert!(!app.context_menu_enabled());
        assert_eq!(
            app.notice(),
            Some(&Notice::ContextMenuFailed(MenuError::ExeDirWritable))
        );
        store.writable.store(false, Ordering::Relaxed);
        app.set_context_menu(true, &notify);
        wait(&mut app);
        assert!(app.context_menu_enabled());
        app.set_context_menu(false, &notify);
        wait(&mut app);
        assert!(!app.context_menu_enabled());
    }

    #[test]
    fn a_relative_worker_is_refused() {
        let mut app = Application::build(Settings::default()).expect("app");
        let setup = ExtractionSetup {
            worker_exe: PathBuf::from("lynkeus-searcher"),
            cache_dir: None,
            opened_entries_dir: None,
        };
        assert!(app.connect_extraction(&setup).is_err());
        assert!(app.document_source().is_none());
    }

    #[test]
    fn settings_include_form_and_layout() {
        let mut app = Application::build(Settings::default()).expect("app");
        app.set_terms(&["ar", "carta"]);
        app.layout.search_panel_width = 400.0;
        app.set_theme(ThemeChoice::Dark);
        let settings = app.settings();
        assert_eq!(settings.terms, ["ar", "carta"]);
        assert!((settings.layout.search_panel_width - 400.0).abs() < f32::EPSILON);
        assert_eq!(settings.theme, ThemeChoice::Dark);
        // Senza file il salvataggio non fa nulla e non fallisce.
        assert!(app.save_settings().is_ok());
    }
}
