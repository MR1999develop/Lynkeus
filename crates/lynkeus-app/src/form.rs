// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Modello del modulo di ricerca, senza egui: radici, righe dei termini, opzioni, OR/AND e
//! filtri come li scrive l'utente, con la validazione campo per campo.
//!
//! [`SearchForm::validate`] trasforma il modulo nei tipi validi del motore (radici,
//! [`CompiledQuery`], [`Filters`]) oppure in [`FormErrors`], un errore per campo da
//! mostrare sotto il campo stesso.

use std::path::{Path, PathBuf};

use lynkeus_core::{
    AccentMode, ByteSize, CaseMode, Combine, CompiledQuery, MatchOptions, Query, QueryError, Term,
    TermError, WildcardMode,
};
use lynkeus_engine::{
    Extension, ExtensionError, ExtensionFilter, Filters, HiddenPolicy, OnlineFilesPolicy,
};

use crate::settings::{ExtensionMode, FilterSettings, MAX_FILE_SIZE_MIB_RANGE, Settings};

/// Che cosa è una radice sul disco, visto quando è stata aggiunta (o all'avvio).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootKind {
    /// Da verificare: la risposta del disco non è ancora arrivata.
    Pending,
    /// Una cartella, cercata con le sottocartelle.
    Folder,
    /// Un file.
    File,
    /// Non trovata (spostata, disco scollegato): la ricerca la segnala negli avvisi.
    Missing,
}

impl RootKind {
    /// Guarda sul disco che cosa è `path` (una chiamata ai metadati, senza seguire i link
    /// oltre quanto fa il sistema). Può bloccare a lungo (rete scollegata): mai sul thread
    /// dell'interfaccia; l'app la chiama in un thread suo.
    pub fn probe(path: &Path) -> RootKind {
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_dir() => RootKind::Folder,
            Ok(_) => RootKind::File,
            Err(_) => RootKind::Missing,
        }
    }
}

/// `path` reso assoluto rispetto alla cartella corrente, senza accedere al disco (`.` e
/// `..` restano come li ha scritti l'utente su Linux; su Windows si risolvono). Una radice
/// assoluta vale lo stesso file per la ricerca, la cache e il processo estrattore, che ha
/// un'altra cartella corrente. Un percorso che non si può rendere assoluto (vuoto) resta
/// com'è: la verifica lo segnalerà come mancante.
pub fn absolute_root(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}

/// Una radice nell'elenco.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootEntry {
    /// Il percorso scelto dall'utente, reso assoluto ([`absolute_root`]).
    pub path: PathBuf,
    /// Cartella, file o non trovata.
    pub kind: RootKind,
}

/// Una riga dei termini.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermRow {
    id: u64,
    /// Il testo scritto.
    pub text: String,
    /// Vero dopo la prima uscita dal campo o il primo tentativo di ricerca: da lì l'errore
    /// si mostra e si aggiorna a ogni modifica.
    pub show_error: bool,
}

impl TermRow {
    /// Identità stabile della riga (per gli `Id` di egui quando si tolgono righe).
    pub fn id(&self) -> u64 {
        self.id
    }
}

/// Filtri come li scrive l'utente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterForm {
    /// Modalità del filtro sulle estensioni.
    pub extension_mode: ExtensionMode,
    /// Estensioni separate da virgole o spazi: "txt, md".
    pub extensions: String,
    /// Cartelle da saltare, una per riga.
    pub skip_dirs: String,
    /// Includere nascosti e di sistema.
    pub include_hidden: bool,
    /// Includere i file solo online.
    pub include_online_only: bool,
    /// Limitare la dimensione dei file.
    pub limit_file_size: bool,
    /// Dimensione massima in MiB.
    pub max_file_size_mib: u64,
    /// Vero dopo l'uscita dal campo estensioni o un tentativo di ricerca.
    pub show_extension_error: bool,
}

impl FilterForm {
    /// Il modulo per le impostazioni `settings`.
    pub fn from_settings(settings: &FilterSettings) -> FilterForm {
        FilterForm {
            extension_mode: settings.extension_mode,
            extensions: settings
                .extensions
                .iter()
                .map(Extension::as_str)
                .collect::<Vec<_>>()
                .join(", "),
            skip_dirs: settings.skip_dirs.join("\n"),
            include_hidden: settings.include_hidden,
            include_online_only: settings.include_online_only,
            limit_file_size: settings.limit_file_size,
            max_file_size_mib: settings.max_file_size_mib,
            show_extension_error: false,
        }
    }

    /// Le impostazioni corrispondenti; le estensioni non valide non si salvano.
    pub fn to_settings(&self) -> FilterSettings {
        FilterSettings {
            extension_mode: self.extension_mode,
            extensions: split_extensions(&self.extensions)
                .filter_map(|e| Extension::parse(e).ok())
                .collect(),
            skip_dirs: self.skip_dir_names(),
            include_hidden: self.include_hidden,
            include_online_only: self.include_online_only,
            limit_file_size: self.limit_file_size,
            max_file_size_mib: self.max_file_size_mib.clamp(
                *MAX_FILE_SIZE_MIB_RANGE.start(),
                *MAX_FILE_SIZE_MIB_RANGE.end(),
            ),
        }
    }

    /// Torna ai filtri predefiniti.
    pub fn reset(&mut self) {
        *self = FilterForm::from_settings(&FilterSettings::default());
    }

    /// Vero se i filtri sono diversi dai predefiniti (per segnalarlo vicino al titolo).
    pub fn is_customized(&self) -> bool {
        self.to_settings() != FilterSettings::default()
    }

    /// L'errore del campo estensioni, se il filtro è attivo.
    pub fn extension_error(&self) -> Option<ExtensionInputError> {
        if self.extension_mode == ExtensionMode::All {
            return None;
        }
        self.parse_extensions().err()
    }

    /// I filtri per il motore.
    pub fn to_filters(&self) -> Result<Filters, ExtensionInputError> {
        let extensions = match self.extension_mode {
            ExtensionMode::All => ExtensionFilter::All,
            ExtensionMode::Include => ExtensionFilter::Include(self.parse_extensions()?),
            ExtensionMode::Exclude => ExtensionFilter::Exclude(self.parse_extensions()?),
        };
        let settings = self.to_settings();
        Ok(Filters {
            extensions,
            skip_dirs: settings.skip_dirs,
            hidden: if self.include_hidden {
                HiddenPolicy::Include
            } else {
                HiddenPolicy::Skip
            },
            max_file_size: self
                .limit_file_size
                .then(|| ByteSize::from_mib(settings.max_file_size_mib)),
            online_only: if self.include_online_only {
                OnlineFilesPolicy::Include
            } else {
                OnlineFilesPolicy::Skip
            },
        })
    }

    fn parse_extensions(&self) -> Result<Vec<Extension>, ExtensionInputError> {
        let mut parsed = Vec::new();
        for raw in split_extensions(&self.extensions) {
            match Extension::parse(raw) {
                Ok(extension) if !parsed.contains(&extension) => parsed.push(extension),
                Ok(_) => {}
                Err(error) => {
                    return Err(ExtensionInputError::Invalid {
                        text: raw.chars().take(Extension::MAX_CHARS).collect(),
                        error,
                    });
                }
            }
        }
        if parsed.is_empty() {
            return Err(ExtensionInputError::Missing);
        }
        Ok(parsed)
    }

    fn skip_dir_names(&self) -> Vec<String> {
        self.skip_dirs
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

/// Le parti di un elenco di estensioni separato da virgole, punti e virgola o spazi.
fn split_extensions(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|part| !part.is_empty())
}

/// Errore del campo estensioni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionInputError {
    /// Filtro attivo senza estensioni.
    Missing,
    /// Un'estensione non valida: il testo (accorciato) e il motivo.
    Invalid {
        /// L'estensione scritta, al massimo [`Extension::MAX_CHARS`] caratteri.
        text: String,
        /// Il motivo.
        error: ExtensionError,
    },
}

/// Errore di una riga dei termini.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermRowError {
    /// Il termine non è valido.
    Invalid(TermError),
    /// Il termine è valido ma troppo complesso da compilare.
    TooComplex,
}

/// Errore del percorso scritto a mano.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathInputError {
    /// Nessun file o cartella con quel percorso.
    NotFound,
    /// L'elenco dei luoghi ha già [`MAX_ROOTS`] voci.
    TooManyRoots,
}

/// Luoghi di ricerca al più: un elenco più lungo non si legge più e non viene da un uso
/// normale (vale per ogni via d'ingresso: dialoghi, trascinamento, campo, clic destro,
/// impostazioni salvate).
pub const MAX_ROOTS: usize = 256;

/// Esito di [`SearchForm::add_root`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootAdded {
    /// Aggiunta in fondo.
    Added,
    /// C'era già.
    Duplicate,
    /// Non aggiunta: l'elenco ha già [`MAX_ROOTS`] voci.
    LimitReached,
}

/// Stato del campo del percorso scritto a mano.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PathInputStatus {
    /// Niente da mostrare.
    #[default]
    Idle,
    /// Verifica in corso del percorso indicato (quello scritto, ripulito).
    Checking(PathBuf),
    /// Il percorso non va: l'errore sotto il campo.
    Error(PathInputError),
}

/// Errori del modulo, uno per campo.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FormErrors {
    /// Nessuna cartella o file in cui cercare.
    pub no_roots: bool,
    /// Errore per riga dei termini, stesso indice delle righe.
    pub terms: Vec<Option<TermRowError>>,
    /// Errore della query che non riguarda una riga (troppi termini).
    pub query: Option<QueryError>,
    /// Errore del campo estensioni.
    pub extensions: Option<ExtensionInputError>,
}

impl FormErrors {
    /// Vero se non c'è nessun errore.
    pub fn is_empty(&self) -> bool {
        !self.no_roots
            && self.terms.iter().all(Option::is_none)
            && self.query.is_none()
            && self.extensions.is_none()
    }
}

/// Una ricerca pronta da avviare.
#[derive(Debug, Clone)]
pub struct ValidSearch {
    /// Le radici, nell'ordine dell'elenco.
    pub roots: Vec<PathBuf>,
    /// I termini validati (senza spazi esterni), nell'ordine delle righe: per la colonna
    /// Termine e i recenti.
    pub terms: Vec<String>,
    /// La query compilata.
    pub query: CompiledQuery,
    /// I filtri.
    pub filters: Filters,
}

/// Il modulo di ricerca.
#[derive(Debug, Clone)]
pub struct SearchForm {
    /// Cartelle e file in cui cercare.
    pub roots: Vec<RootEntry>,
    /// Percorso scritto o incollato, da aggiungere con Invio.
    pub path_input: String,
    /// Verifica o errore del percorso scritto, mostrato sotto il campo.
    pub path_status: PathInputStatus,
    /// Vero dopo un tentativo di ricerca senza radici.
    pub show_roots_error: bool,
    terms: Vec<TermRow>,
    next_row_id: u64,
    /// Distinguere maiuscole e minuscole.
    pub case_sensitive: bool,
    /// Ignorare gli accenti.
    pub ignore_accents: bool,
    /// `*` e `?` come caratteri jolly.
    pub wildcards: bool,
    /// OR o AND, usato solo con almeno due termini.
    pub combine: Combine,
    /// Filtri.
    pub filters: FilterForm,
}

impl Default for SearchForm {
    fn default() -> Self {
        SearchForm::from_settings(&Settings::default(), |_| RootKind::Missing)
    }
}

impl SearchForm {
    /// Il modulo per le impostazioni salvate; `probe` dice che cosa è ogni radice (l'app
    /// passa `|_| RootKind::Pending` e verifica dopo, fuori dal thread dell'interfaccia).
    pub fn from_settings(settings: &Settings, probe: impl Fn(&Path) -> RootKind) -> SearchForm {
        let mut form = SearchForm {
            roots: Vec::new(),
            path_input: String::new(),
            path_status: PathInputStatus::Idle,
            show_roots_error: false,
            terms: Vec::new(),
            next_row_id: 0,
            case_sensitive: settings.options.case == CaseMode::Sensitive,
            ignore_accents: settings.options.accents == AccentMode::Ignored,
            wildcards: settings.options.wildcards == WildcardMode::Enabled,
            combine: settings.combine,
            filters: FilterForm::from_settings(&settings.filters),
        };
        for path in settings.roots.iter().take(MAX_ROOTS) {
            form.add_root(path.clone(), probe(path));
        }
        for term in settings.terms.iter().take(Query::MAX_TERMS) {
            let id = form.add_term();
            if let Some(row) = form.terms.iter_mut().find(|r| r.id == id) {
                row.text.clone_from(term);
            }
        }
        if form.terms.is_empty() {
            form.add_term();
        }
        form
    }

    /// Copia nel modulo le impostazioni salvate: radici, termini, opzioni, filtri.
    pub fn write_settings(&self, settings: &mut Settings) {
        settings.roots = self.roots.iter().map(|r| r.path.clone()).collect();
        settings.terms = self.terms.iter().map(|r| r.text.clone()).collect();
        settings.options = self.match_options();
        settings.combine = self.combine;
        settings.filters = self.filters.to_settings();
    }

    /// Aggiunge una radice in fondo, resa assoluta ([`absolute_root`]), se non c'è già e se
    /// l'elenco non ha già [`MAX_ROOTS`] voci.
    pub fn add_root(&mut self, path: PathBuf, kind: RootKind) -> RootAdded {
        let path = absolute_root(path);
        if self.roots.iter().any(|r| r.path == path) {
            return RootAdded::Duplicate;
        }
        if self.roots_full() {
            return RootAdded::LimitReached;
        }
        self.roots.push(RootEntry { path, kind });
        self.show_roots_error = false;
        RootAdded::Added
    }

    /// Vero se l'elenco dei luoghi ha già [`MAX_ROOTS`] voci.
    pub fn roots_full(&self) -> bool {
        self.roots.len() >= MAX_ROOTS
    }

    /// Registra che cosa è `path` sul disco, per tutte le radici con quel percorso.
    pub fn set_root_kind(&mut self, path: &Path, kind: RootKind) {
        for root in self.roots.iter_mut().filter(|r| r.path == path) {
            root.kind = kind;
        }
    }

    /// Toglie la radice in posizione `index` (nessun effetto fuori dai limiti).
    pub fn remove_root(&mut self, index: usize) {
        if index < self.roots.len() {
            self.roots.remove(index);
        }
    }

    /// Le righe dei termini (sempre almeno una).
    pub fn terms(&self) -> &[TermRow] {
        &self.terms
    }

    /// Il testo della riga `index`, da modificare.
    pub fn term_text_mut(&mut self, index: usize) -> Option<&mut String> {
        self.terms.get_mut(index).map(|row| &mut row.text)
    }

    /// Da ora mostra l'errore della riga `index` (uscita dal campo).
    pub fn reveal_term_error(&mut self, index: usize) {
        if let Some(row) = self.terms.get_mut(index) {
            row.show_error = !row.text.is_empty();
        }
    }

    /// Vero se si può aggiungere un'altra riga ([`Query::MAX_TERMS`] al massimo).
    pub fn can_add_term(&self) -> bool {
        self.terms.len() < Query::MAX_TERMS
    }

    /// Aggiunge una riga vuota in fondo e ne restituisce l'identità; al massimo
    /// [`Query::MAX_TERMS`] righe (oltre restituisce l'ultima).
    pub fn add_term(&mut self) -> u64 {
        if !self.can_add_term()
            && let Some(last) = self.terms.last()
        {
            return last.id;
        }
        let id = self.next_row_id;
        self.next_row_id += 1;
        self.terms.push(TermRow {
            id,
            text: String::new(),
            show_error: false,
        });
        id
    }

    /// Toglie la riga `index`; l'ultima riga rimasta si svuota invece di sparire.
    pub fn remove_term(&mut self, index: usize) {
        if self.terms.len() > 1 {
            if index < self.terms.len() {
                self.terms.remove(index);
            }
        } else if let Some(row) = self.terms.first_mut() {
            row.text.clear();
            row.show_error = false;
        }
    }

    /// Sostituisce tutte le righe con `terms` (almeno una riga resta).
    pub fn set_terms<S: AsRef<str>>(&mut self, terms: &[S]) {
        self.terms.clear();
        for term in terms.iter().take(Query::MAX_TERMS) {
            let id = self.add_term();
            if let Some(row) = self.terms.iter_mut().find(|r| r.id == id) {
                term.as_ref().clone_into(&mut row.text);
            }
        }
        if self.terms.is_empty() {
            self.add_term();
        }
    }

    /// Mette `term` nella prima riga vuota, o in una nuova riga.
    pub fn insert_term(&mut self, term: &str) {
        if let Some(row) = self.terms.iter_mut().find(|r| r.text.trim().is_empty()) {
            term.clone_into(&mut row.text);
            return;
        }
        if self.can_add_term() {
            let id = self.add_term();
            if let Some(row) = self.terms.iter_mut().find(|r| r.id == id) {
                term.clone_into(&mut row.text);
            }
        }
    }

    /// L'errore attuale della riga `index`, a prescindere da quando mostrarlo.
    pub fn term_error(&self, index: usize) -> Option<TermError> {
        let row = self.terms.get(index)?;
        Term::parse(&row.text, self.wildcard_mode()).err()
    }

    /// Vero se la scelta qualsiasi/tutti i termini ha senso: almeno due righe.
    pub fn combine_enabled(&self) -> bool {
        self.terms.len() >= 2
    }

    /// Le opzioni di confronto.
    pub fn match_options(&self) -> MatchOptions {
        MatchOptions {
            case: if self.case_sensitive {
                CaseMode::Sensitive
            } else {
                CaseMode::Insensitive
            },
            accents: if self.ignore_accents {
                AccentMode::Ignored
            } else {
                AccentMode::Distinct
            },
            wildcards: self.wildcard_mode(),
        }
    }

    fn wildcard_mode(&self) -> WildcardMode {
        if self.wildcards {
            WildcardMode::Enabled
        } else {
            WildcardMode::Literal
        }
    }

    /// Mostra da ora tutti gli errori (tentativo di ricerca).
    pub fn reveal_all_errors(&mut self) {
        for row in &mut self.terms {
            row.show_error = true;
        }
        self.show_roots_error = true;
        self.filters.show_extension_error = true;
    }

    /// Valida tutto il modulo: la ricerca pronta, o un errore per campo.
    pub fn validate(&self) -> Result<ValidSearch, FormErrors> {
        let mut errors = FormErrors {
            no_roots: self.roots.is_empty(),
            terms: vec![None; self.terms.len()],
            ..FormErrors::default()
        };
        let mut terms = Vec::with_capacity(self.terms.len());
        for (slot, row) in errors.terms.iter_mut().zip(&self.terms) {
            match Term::parse(&row.text, self.wildcard_mode()) {
                Ok(term) => terms.push(term),
                Err(error) => *slot = Some(TermRowError::Invalid(error)),
            }
        }
        let filters = self.filters.to_filters();
        if let Err(error) = &filters {
            errors.extensions = Some(error.clone());
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let combine = if self.combine_enabled() {
            self.combine
        } else {
            Combine::Any
        };
        // I termini validati, senza spazi esterni: per la colonna Termine e i recenti.
        let term_texts = terms.iter().map(|t| t.as_str().to_owned()).collect();
        let compiled = Query::new(terms, self.match_options(), combine).and_then(|q| q.compile());
        let query = match compiled {
            Ok(query) => query,
            Err(QueryError::TooComplex { term_index }) => {
                if let Some(slot) = errors.terms.get_mut(term_index) {
                    *slot = Some(TermRowError::TooComplex);
                }
                return Err(errors);
            }
            Err(error) => {
                errors.query = Some(error);
                return Err(errors);
            }
        };
        match filters {
            Ok(filters) => Ok(ValidSearch {
                roots: self.roots.iter().map(|r| r.path.clone()).collect(),
                terms: term_texts,
                query,
                filters,
            }),
            Err(error) => {
                errors.extensions = Some(error);
                Err(errors)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form_with(roots: &[&str], terms: &[&str]) -> SearchForm {
        let mut form = SearchForm::default();
        for root in roots {
            form.add_root(PathBuf::from(root), RootKind::Folder);
        }
        form.set_terms(terms);
        form
    }

    #[test]
    fn valid_form_becomes_a_search() {
        let form = form_with(&["dati"], &["ar"]);
        let search = form.validate().expect("valido");
        assert_eq!(search.roots, [absolute_root(PathBuf::from("dati"))]);
        assert_eq!(search.terms, ["ar"]);
        assert_eq!(search.filters, Filters::default());
    }

    #[test]
    fn search_terms_are_the_validated_ones_without_outer_spaces() {
        let form = form_with(&["dati"], &["  ar ", "	c?rta"]);
        let search = form.validate().expect("valido");
        assert_eq!(search.terms, ["ar", "c?rta"]);
    }

    #[test]
    fn every_field_reports_its_own_error() {
        let mut form = form_with(&[], &["ar", "  ", "**"]);
        form.filters.extension_mode = ExtensionMode::Include;
        form.filters.extensions = "txt, tar.gz".to_owned();
        let errors = form.validate().expect_err("non valido");
        assert!(errors.no_roots);
        assert_eq!(
            errors.terms,
            [
                None,
                Some(TermRowError::Invalid(TermError::Empty)),
                Some(TermRowError::Invalid(TermError::OnlyWildcards)),
            ]
        );
        assert_eq!(
            errors.extensions,
            Some(ExtensionInputError::Invalid {
                text: "tar.gz".to_owned(),
                error: ExtensionError::InvalidChar,
            })
        );
    }

    #[test]
    fn wildcards_off_make_stars_literal() {
        let mut form = form_with(&["dati"], &["**"]);
        assert_eq!(form.term_error(0), Some(TermError::OnlyWildcards));
        form.wildcards = false;
        assert_eq!(form.term_error(0), None);
        assert!(form.validate().is_ok());
    }

    #[test]
    fn rows_partitions() {
        let mut form = SearchForm::default();
        assert_eq!(form.terms().len(), 1, "sempre almeno una riga");
        assert!(!form.combine_enabled());
        form.add_term();
        assert!(form.combine_enabled());
        form.remove_term(0);
        form.remove_term(0);
        assert_eq!(
            form.terms().len(),
            1,
            "l'ultima riga si svuota, non sparisce"
        );
        for _ in 0..(Query::MAX_TERMS + 5) {
            form.add_term();
        }
        assert_eq!(form.terms().len(), Query::MAX_TERMS);
        assert!(!form.can_add_term());
        let ids: std::collections::HashSet<u64> = form.terms().iter().map(TermRow::id).collect();
        assert_eq!(ids.len(), Query::MAX_TERMS, "identità distinte");
    }

    #[test]
    fn insert_term_fills_the_first_empty_row() {
        let mut form = form_with(&[], &["ar", ""]);
        form.insert_term("carta");
        let texts: Vec<&str> = form.terms().iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["ar", "carta"]);
        form.insert_term("mare");
        assert_eq!(form.terms().len(), 3);
    }

    #[test]
    fn one_term_searches_with_any_even_if_all_was_chosen() {
        let mut form = form_with(&["dati"], &["ar"]);
        form.combine = Combine::All;
        let search = form.validate().expect("valido");
        assert_eq!(search.query.combine(), Combine::Any);
    }

    #[test]
    fn roots_are_unique_and_removable() {
        let mut form = form_with(&["a", "b"], &["x"]);
        assert_eq!(
            form.add_root(PathBuf::from("a"), RootKind::Folder),
            RootAdded::Duplicate
        );
        form.remove_root(0);
        form.remove_root(7);
        let paths: Vec<&Path> = form.roots.iter().map(|r| r.path.as_path()).collect();
        assert_eq!(paths, [absolute_root(PathBuf::from("b")).as_path()]);
    }

    /// Al più [`MAX_ROOTS`] luoghi: l'ultimo entra, il successivo no (un doppione resta un
    /// doppione); anche dalle impostazioni salvate.
    #[test]
    fn roots_stop_at_the_limit() {
        let mut form = form_with(&[], &["x"]);
        for i in 0..MAX_ROOTS {
            assert_eq!(
                form.add_root(PathBuf::from(format!("r{i}")), RootKind::Folder),
                RootAdded::Added
            );
        }
        assert!(form.roots_full());
        assert_eq!(
            form.add_root(PathBuf::from("oltre"), RootKind::Folder),
            RootAdded::LimitReached
        );
        assert_eq!(
            form.add_root(PathBuf::from("r0"), RootKind::Folder),
            RootAdded::Duplicate
        );
        assert_eq!(form.roots.len(), MAX_ROOTS);
        form.remove_root(0);
        assert_eq!(
            form.add_root(PathBuf::from("oltre"), RootKind::Folder),
            RootAdded::Added
        );

        let settings = Settings {
            roots: (0..MAX_ROOTS + 10)
                .map(|i| PathBuf::from(format!("s{i}")))
                .collect(),
            ..Settings::default()
        };
        let loaded = SearchForm::from_settings(&settings, |_| RootKind::Pending);
        assert_eq!(loaded.roots.len(), MAX_ROOTS);
    }

    #[test]
    fn filters_to_engine() {
        let mut filters = FilterForm::from_settings(&FilterSettings::default());
        assert!(!filters.is_customized());
        filters.extension_mode = ExtensionMode::Exclude;
        filters.extensions = ".LOG; tmp  log".to_owned();
        filters.skip_dirs = " build \n\n.cache".to_owned();
        filters.include_hidden = true;
        filters.limit_file_size = false;
        let engine = filters.to_filters().expect("validi");
        let excluded: Vec<&str> = match &engine.extensions {
            ExtensionFilter::Exclude(list) => list.iter().map(Extension::as_str).collect(),
            other => panic!("atteso Exclude: {other:?}"),
        };
        assert_eq!(excluded, ["log", "tmp"]);
        assert_eq!(engine.skip_dirs, ["build", ".cache"]);
        assert_eq!(engine.hidden, HiddenPolicy::Include);
        assert_eq!(engine.max_file_size, None);
        assert!(filters.is_customized());

        filters.extensions = " , ".to_owned();
        assert_eq!(
            filters.extension_error(),
            Some(ExtensionInputError::Missing)
        );
        filters.extension_mode = ExtensionMode::All;
        assert_eq!(filters.extension_error(), None);
        filters.reset();
        assert!(!filters.is_customized());
    }

    #[test]
    fn settings_round_trip_through_the_form() {
        let settings = Settings {
            roots: vec![std::env::temp_dir().join("a")],
            terms: vec!["ar".to_owned(), "c?rt*".to_owned()],
            combine: Combine::All,
            options: MatchOptions {
                case: CaseMode::Sensitive,
                ..MatchOptions::default()
            },
            ..Settings::default()
        };
        let form = SearchForm::from_settings(&settings, |_| RootKind::Folder);
        let mut back = Settings::default();
        form.write_settings(&mut back);
        assert_eq!(back, settings);
    }
}
