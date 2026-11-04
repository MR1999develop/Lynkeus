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
use std::time::{Duration, Instant};

use lynkeus_engine::{
    DocPath, Hit, Limits, Notify, Outcome, ProgressSnapshot, Received, SearchEvent, SearchHandle,
    SearchRequest, Summary, Warning, start_search,
};

use crate::form::{FormErrors, PathInputError, RootKind, SearchForm, TermRowError};
use crate::i18n::{I18n, Language};
use crate::results::{ResultsView, SortColumn, SortSpec};
use crate::settings::{LayoutSettings, Settings, SettingsError, SettingsFile, push_recent};
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
        let form = SearchForm::from_settings(&settings, RootKind::probe);
        let saved = settings_file.as_ref().map(|_| settings.clone());
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

    /// Aggiunge una radice; falso se c'era già.
    pub fn add_root(&mut self, root: PathBuf) -> bool {
        let kind = RootKind::probe(&root);
        self.form.add_root(root, kind)
    }

    /// Aggiunge il percorso scritto nel campo apposito, se esiste; altrimenti l'errore
    /// resta in [`SearchForm::path_error`].
    pub fn add_typed_path(&mut self) {
        let typed = self.form.path_input.trim().trim_matches('"');
        if typed.is_empty() {
            self.form.path_error = None;
            return;
        }
        let path = PathBuf::from(typed);
        match RootKind::probe(&path) {
            RootKind::Missing => self.form.path_error = Some(PathInputError::NotFound),
            kind => {
                self.form.add_root(path, kind);
                self.form.path_input.clear();
                self.form.path_error = None;
            }
        }
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
    /// `notify` è chiamata dal worker a ogni evento (la vista passa `request_repaint`).
    /// L'errore resta anche in [`Application::start_error`] e il modulo mostra da ora gli
    /// errori dei campi.
    pub fn start_search(&mut self, notify: Notify) -> Result<(), &StartError> {
        self.job = JobState::Idle;
        self.start_error = None;
        self.form.reveal_all_errors();
        let search = match self.form.validate() {
            Ok(search) => search,
            Err(errors) => return Err(self.start_error.insert(StartError::Form(errors))),
        };
        let request = SearchRequest {
            roots: search.roots.clone(),
            query: search.query,
            filters: search.filters,
            limits: self.limits,
        };
        let handle = match start_search(request, notify) {
            Ok(handle) => handle,
            Err(error) => return Err(self.start_error.insert(StartError::Spawn(error))),
        };
        self.results = Results {
            generation: self.results.generation.wrapping_add(1),
            roots: search.roots,
            terms: search.terms,
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

    #[test]
    fn typed_path_must_exist() {
        let mut app = Application::build(Settings::default()).expect("app");
        app.form.path_input = "  ".to_owned();
        app.add_typed_path();
        assert!(app.form.roots.is_empty() && app.form.path_error.is_none());

        let missing = std::env::temp_dir().join("lynkeus-non-esiste-7f3a9c");
        app.form.path_input = missing.display().to_string();
        app.add_typed_path();
        assert_eq!(app.form.path_error, Some(PathInputError::NotFound));
        assert!(app.form.roots.is_empty());

        let here = env!("CARGO_MANIFEST_DIR");
        app.form.path_input = format!("\"{here}\"");
        app.add_typed_path();
        assert_eq!(app.form.path_error, None);
        assert_eq!(app.form.roots.len(), 1);
        assert_eq!(app.form.roots[0].kind, RootKind::Folder);
        assert!(app.form.path_input.is_empty());
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
