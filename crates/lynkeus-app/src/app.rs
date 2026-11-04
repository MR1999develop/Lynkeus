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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use lynkeus_core::{ByteSize, CompiledQuery};
use lynkeus_engine::{
    DocPath, DocumentSource, Hit, Limits, Notify, Outcome, ProgressSnapshot, Received, SearchEvent,
    SearchHandle, SearchRequest, Summary, Warning, start_search,
};

use crate::export::{self, ExportScope, ExportSettings};
use crate::extraction::{self, ExtractionSetup, OpenedEntries, PoolDocumentUnits};
use crate::form::{
    FormErrors, PathInputError, PathInputStatus, RootKind, SearchForm, TermRowError,
};
use crate::i18n::{I18n, Language};
use crate::launch::{
    self, ArchiveEntryExtractor, LaunchOutcome, LaunchTask, NoArchiveExtraction, OpenPlan,
};
use crate::preview::{self, DocumentUnits, Loaded, NoDocumentUnits, PreviewError, PreviewRequest};
use crate::results::{ResultsView, SortColumn, SortSpec, Step};
use crate::root_check::{RootCheck, RootChecks};
use crate::settings::{
    CACHE_SIZE_MIB_RANGE, CacheSettings, LayoutSettings, Settings, SettingsError, SettingsFile,
    push_recent,
};
use crate::theme::ThemeChoice;

/// Righe di risultato inserite al massimo in una chiamata di
/// [`Application::pump_events`] fatta dalla vista: il frame resta veloce anche con
/// 200 000 risultati in arrivo.
pub const MAX_HITS_PER_FRAME: usize = 5_000;

/// Dopo quanto tempo la barra d'avanzamento compare.
pub const PROGRESS_DELAY: Duration = Duration::from_secs(1);

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
    /// Ricerca finita senza righe.
    NothingFound,
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
    /// Richiesta di portare il focus sul primo termine al prossimo frame.
    pub focus_first_term: bool,
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
    /// Le righe, nell'ordine di arrivo.
    pub hits: Vec<Hit>,
    /// Gli avvisi, nell'ordine di arrivo.
    pub warnings: Vec<Warning>,
}

/// L'applicazione: radice dell'albero di proprietà dello stato.
pub struct Application {
    settings: Settings,
    settings_file: Option<SettingsFile>,
    /// Le impostazioni come sono sul disco, per scrivere solo se cambiano.
    saved: Option<Settings>,
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
    opened_entries_dir: Option<PathBuf>,
    cache_job: Option<Receiver<io::Result<ByteSize>>>,
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
            notice: None,
            document_source: None,
            opened_entries_dir: None,
            cache_job: None,
            settings,
            settings_file,
            saved,
        }
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
    /// c'era già.
    pub fn add_root(&mut self, root: PathBuf) -> bool {
        let added = self.form.add_root(root.clone(), RootKind::Pending);
        if added {
            self.root_checks.request(root);
        }
        added
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
        let path = PathBuf::from(typed);
        if self.form.roots.iter().any(|r| r.path == path) {
            self.form.path_input.clear();
            self.form.path_status = PathInputStatus::Idle;
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
            } else {
                self.form.add_root(path, kind);
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
    /// errori dei campi.
    pub fn start_search(&mut self, notify: Notify) -> Result<(), &StartError> {
        self.start_error = None;
        self.form.reveal_all_errors();
        let search = match self.form.validate() {
            Ok(search) => search,
            Err(errors) => return Err(self.start_error.insert(StartError::Form(errors))),
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
            hits: Vec::new(),
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

    /// Chiede di annullare la ricerca in corso; il riepilogo arriverà con gli eventi.
    pub fn cancel_search(&mut self) {
        if let JobState::Running(job) = &self.job {
            job.handle.cancel();
        }
    }

    /// Legge gli eventi arrivati, al massimo circa `max_hits` righe di risultato.
    pub fn pump_events(&mut self, max_hits: usize) -> Pump {
        let pump = self.receive(max_hits);
        self.view.extend_to(self.results.hits.len());
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
        let mut inserted = 0usize;
        loop {
            if inserted >= max_hits {
                return Pump::MorePending;
            }
            match job.handle.try_recv() {
                Received::Event(SearchEvent::Hits(hits)) => {
                    inserted += hits.len();
                    self.results.hits.extend(hits);
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
        self.view.step_target(&self.results.hits, step)
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
            OpenPlan::ArchiveEntry => LaunchTask::OpenArchiveEntry((*doc).clone()),
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
            (LaunchOutcome::Failed, LaunchTask::Reveal(_)) => Some(Notice::RevealFailed { file }),
            (LaunchOutcome::Failed, _) => Some(Notice::OpenFailed { file }),
        };
        self.launch_job = None;
    }

    // --- Copia ed esportazione --------------------------------------------------------

    /// Le righe selezionate come TSV per gli appunti, con il loro numero; `None` senza
    /// selezione.
    pub fn selection_tsv(&self) -> Option<(String, usize)> {
        let indices: Vec<usize> = self.view.selected_hits().collect();
        if indices.is_empty() {
            return None;
        }
        let results = &self.results;
        let table = export::results_table(&self.i18n, &results.hits, &results.terms, indices);
        Some((export::to_tsv(&table.rows), table.rows.len()))
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
    /// salvataggio; la scrittura è atomica e il file precedente resta se fallisce.
    pub fn export_to(&mut self, path: PathBuf, notify: &Notify) {
        let settings = self.settings.export.clone();
        let indices: Vec<usize> = match settings.scope {
            ExportScope::Visible => (0..self.view.len())
                .filter_map(|p| self.view.hit_at(p))
                .collect(),
            ExportScope::Selected => self.view.selected_hits().collect(),
        };
        let results = &self.results;
        let table = export::results_table(&self.i18n, &results.hits, &results.terms, indices);
        let rows = table.rows.len();
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
    /// leggono anche PDF, Office e archivi; l'anteprima e "Apri" li usano. La cartella dei
    /// file aperti si svuota subito (avanzi di un'uscita anomala), in un thread.
    ///
    /// Errore se il lanciatore non si può creare (eseguibile senza percorso assoluto): l'app
    /// resta senza documenti.
    pub fn connect_extraction(&mut self, setup: &ExtractionSetup) -> io::Result<()> {
        let source = extraction::document_source(setup, self.settings.cache)?;
        self.documents = Arc::new(PoolDocumentUnits::new(Arc::clone(&source), self.limits));
        if let Some(dir) = &setup.opened_entries_dir {
            self.archive_extractor = Arc::new(OpenedEntries::new(
                Arc::clone(&source),
                dir.clone(),
                self.limits,
            ));
            let dir = dir.clone();
            // Se il thread non parte, si ripulirà all'uscita.
            let _cleaning = thread::Builder::new()
                .name("pulizia".to_owned())
                .spawn(move || extraction::clean_opened_entries(&dir));
        }
        self.opened_entries_dir = setup.opened_entries_dir.clone();
        self.document_source = Some(source);
        Ok(())
    }

    /// La fonte dei documenti collegata, se c'è.
    pub fn document_source(&self) -> Option<&Arc<DocumentSource>> {
        self.document_source.as_ref()
    }

    /// Chiude i processi estrattori liberi e svuota la cartella dei file aperti (all'uscita).
    pub fn shutdown_extraction(&mut self) {
        if let Some(source) = &self.document_source {
            source.pool().shutdown();
        }
        if let Some(dir) = &self.opened_entries_dir {
            extraction::clean_opened_entries(dir);
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

    /// Raccoglie, senza attendere, l'esito delle aperture e delle esportazioni finite.
    pub fn pump_actions(&mut self) {
        self.poll_launch();
        self.poll_export();
        self.poll_cache();
    }

    /// Salva le impostazioni se sono cambiate dall'ultimo salvataggio. Senza file (test)
    /// non fa nulla.
    pub fn save_settings(&mut self) -> Result<(), SettingsError> {
        let current = self.settings();
        let Some(file) = self.settings_file.as_mut() else {
            return Ok(());
        };
        if self.saved.as_ref() == Some(&current) {
            return Ok(());
        }
        file.save(&current)?;
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
        JobState::Finished(_) | JobState::WorkerLost => EmptyState::NothingFound,
        JobState::Idle if has_roots => EmptyState::WriteTerm,
        JobState::Idle => EmptyState::AddRoot,
    }
}

#[cfg(test)]
mod tests {
    use lynkeus_engine::Outcome;

    use super::*;

    #[test]
    fn empty_state_partitions() {
        let finished = JobState::Finished(Summary {
            outcome: Outcome::Completed,
            elapsed: Duration::ZERO,
            counts: ProgressSnapshot::default(),
        });
        assert_eq!(
            empty_state(false, false, &JobState::Idle),
            EmptyState::AddRoot
        );
        assert_eq!(
            empty_state(false, true, &JobState::Idle),
            EmptyState::WriteTerm
        );
        assert_eq!(
            empty_state(false, true, &finished),
            EmptyState::NothingFound
        );
        assert_eq!(
            empty_state(false, true, &JobState::WorkerLost),
            EmptyState::NothingFound
        );
        assert_eq!(empty_state(true, true, &finished), EmptyState::None);
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
