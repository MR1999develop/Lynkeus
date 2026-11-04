// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Stato dell'applicazione e casi d'uso, senza finestra.
//!
//! [`Application`] possiede impostazioni, traduttore, modulo di ricerca, risultati e
//! ricerca in corso. La vista (`ui.rs`) la disegna e le passa i gesti; i test la usano
//! direttamente: impostano cartella e termine, avviano, pompano gli eventi e leggono
//! risultati e stato.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lynkeus_core::{Query, QueryError, Term, TermError};
use lynkeus_engine::{
    DocPath, Filters, Hit, Limits, Notify, ProgressSnapshot, Received, SearchEvent, SearchHandle,
    SearchRequest, Summary, Warning, start_search,
};

use crate::i18n::{I18n, Language};
use crate::settings::Settings;

/// Righe di risultato inserite al massimo in una chiamata di
/// [`Application::pump_events`] fatta dalla vista: il frame resta veloce anche con
/// 200 000 risultati in arrivo.
pub const MAX_HITS_PER_FRAME: usize = 5_000;

/// Perché una ricerca non è partita.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// Nessuna cartella indicata.
    #[error("nessuna cartella")]
    NoRoot,
    /// Termine non valido.
    #[error("termine non valido: {0}")]
    Term(#[from] TermError),
    /// Query non valida o non compilabile.
    #[error("query non valida: {0}")]
    Query(#[from] QueryError),
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

/// Campi del modulo di ricerca, come li ha scritti l'utente.
#[derive(Debug, Clone, Default)]
pub struct SearchForm {
    /// Percorso della cartella (o del file) in cui cercare.
    pub root: String,
    /// Il termine.
    pub term: String,
}

/// Risultati di una ricerca.
#[derive(Debug, Default)]
pub struct Results {
    /// Numero della ricerca che li ha prodotti; cambia a ogni avvio (es. per riportare
    /// la tabella in cima).
    pub generation: u64,
    /// Le radici della ricerca, per i percorsi relativi ([`Hit::root`] è un indice qui).
    pub roots: Vec<PathBuf>,
    /// Le righe, nell'ordine di arrivo.
    pub hits: Vec<Hit>,
    /// Gli avvisi, nell'ordine di arrivo.
    pub warnings: Vec<Warning>,
}

/// L'applicazione: radice dell'albero di proprietà dello stato.
pub struct Application {
    settings: Settings,
    i18n: I18n,
    limits: Limits,
    /// Campi del modulo, modificati dalla vista.
    pub form: SearchForm,
    start_error: Option<StartError>,
    results: Results,
    job: JobState,
    current_file: Option<Arc<DocPath>>,
}

impl Application {
    /// Composition root: costruisce l'app dalle impostazioni (lingua del sistema se non
    /// scelta). Usata da `main` e dai test d'accettazione.
    pub fn build(settings: Settings) -> anyhow::Result<Application> {
        let language = settings.language.unwrap_or_else(Language::from_system);
        let form = SearchForm {
            root: settings
                .last_root
                .as_deref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            term: settings.last_term.clone(),
        };
        Ok(Application {
            settings,
            i18n: I18n::new(language),
            limits: Limits::default(),
            form,
            start_error: None,
            results: Results::default(),
            job: JobState::Idle,
            current_file: None,
        })
    }

    /// Il traduttore.
    pub fn i18n(&self) -> &I18n {
        &self.i18n
    }

    /// Le impostazioni correnti.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// I limiti delle ricerche.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Imposta la cartella (o il file) in cui cercare.
    pub fn set_root(&mut self, root: &Path) {
        self.form.root = root.display().to_string();
    }

    /// Imposta il termine da cercare.
    pub fn set_term(&mut self, term: &str) {
        term.clone_into(&mut self.form.term);
    }

    /// L'errore dell'ultimo tentativo di avvio, da mostrare sotto il modulo.
    pub fn start_error(&self) -> Option<&StartError> {
        self.start_error.as_ref()
    }

    /// Vero se una ricerca è in corso.
    pub fn is_searching(&self) -> bool {
        matches!(self.job, JobState::Running(_))
    }

    /// Avvia una ricerca con i campi del modulo; una ricerca in corso viene annullata e i
    /// suoi risultati scartati.
    ///
    /// `notify` è chiamata dal worker a ogni evento (la vista passa `request_repaint`).
    /// L'errore resta anche in [`Application::start_error`].
    pub fn start_search(&mut self, notify: Notify) -> Result<(), &StartError> {
        self.job = JobState::Idle;
        self.start_error = None;
        let started = self.prepare_request().and_then(|request| {
            let roots = request.roots.clone();
            start_search(request, notify)
                .map(|handle| (handle, roots))
                .map_err(StartError::Spawn)
        });
        match started {
            Ok((handle, roots)) => {
                self.results = Results {
                    generation: self.results.generation.wrapping_add(1),
                    roots,
                    hits: Vec::new(),
                    warnings: Vec::new(),
                };
                self.current_file = None;
                self.job = JobState::Running(Job {
                    handle,
                    started: Instant::now(),
                });
                self.remember_form();
                Ok(())
            }
            Err(error) => Err(self.start_error.insert(error)),
        }
    }

    /// Chiede di annullare la ricerca in corso; il riepilogo arriverà con gli eventi.
    pub fn cancel_search(&mut self) {
        if let JobState::Running(job) = &self.job {
            job.handle.cancel();
        }
    }

    /// Legge gli eventi arrivati, al massimo circa `max_hits` righe di risultato.
    pub fn pump_events(&mut self, max_hits: usize) -> Pump {
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

    /// Il documento in lettura (l'ultimo visto), durante una ricerca.
    pub fn current_file(&self) -> Option<&DocPath> {
        self.current_file.as_deref()
    }

    /// I risultati dell'ultima ricerca.
    pub fn results(&self) -> &Results {
        &self.results
    }

    /// Valida il modulo e prepara la richiesta per il motore.
    fn prepare_request(&self) -> Result<SearchRequest, StartError> {
        let root = self.form.root.trim();
        if root.is_empty() {
            return Err(StartError::NoRoot);
        }
        let options = self.settings.options;
        let term = Term::parse(&self.form.term, options.wildcards)?;
        let query = Query::new(vec![term], options, self.settings.combine)?.compile()?;
        Ok(SearchRequest {
            roots: vec![PathBuf::from(root)],
            query,
            filters: Filters::default(),
            limits: self.limits,
        })
    }

    /// Ricorda cartella e termine nelle impostazioni.
    fn remember_form(&mut self) {
        self.settings.last_root = Some(PathBuf::from(self.form.root.trim()));
        self.settings.last_term.clone_from(&self.form.term);
    }
}
