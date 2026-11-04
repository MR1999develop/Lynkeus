// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Eventi che il worker manda al chiamante: risultati, avvisi, fine.

use std::sync::Arc;
use std::time::Duration;

use lynkeus_core::{Location, Snippet, TermSet};
use lynkeus_extract::{ExtractError, LimitKind};

use crate::doc::DocPath;
use crate::progress::ProgressSnapshot;

/// Una riga dei risultati: una parola che contiene almeno un termine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Il documento, condiviso tra tutte le righe dello stesso file.
    pub doc: Arc<DocPath>,
    /// Indice della radice in [`SearchRequest::roots`](crate::SearchRequest::roots) da cui
    /// si è arrivati al documento (per il percorso relativo).
    pub root: usize,
    /// Posizione nel documento.
    pub location: Location,
    /// La parola, eventualmente troncata (senza "…").
    pub word: String,
    /// Vero se `word` è troncata.
    pub word_truncated: bool,
    /// Il contesto con le corrispondenze evidenziate.
    pub snippet: Snippet,
    /// Termini che corrispondono nella parola.
    pub terms: TermSet,
}

/// Motivo di un avviso: un documento che non si è potuto leggere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WarningKind {
    /// Radice inesistente o sparita.
    NotFound,
    /// Accesso negato.
    AccessDenied,
    /// File in uso da un altro programma.
    InUse,
    /// Altro errore di lettura.
    Io,
    /// Protetto da password.
    Encrypted,
    /// Danneggiato.
    Corrupt,
    /// Tempo scaduto.
    Timeout,
    /// Limite superato.
    LimitExceeded(LimitKind),
    /// Contenuto diverso dall'estensione.
    ContentMismatch,
    /// Nessun testo estraibile (probabile scansione).
    NoText,
}

impl WarningKind {
    /// Motivo dell'avviso per un errore di estrazione; `None` per gli errori che non sono
    /// avvisi ma file saltati ([`ExtractError::Binary`], [`ExtractError::TooLarge`]).
    pub fn from_extract_error(error: &ExtractError) -> Option<WarningKind> {
        Some(match error {
            ExtractError::Binary | ExtractError::TooLarge { .. } => return None,
            ExtractError::AccessDenied => WarningKind::AccessDenied,
            ExtractError::InUse => WarningKind::InUse,
            ExtractError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => {
                WarningKind::NotFound
            }
            ExtractError::Io(_) => WarningKind::Io,
            ExtractError::Encrypted => WarningKind::Encrypted,
            ExtractError::Corrupt { .. } => WarningKind::Corrupt,
            ExtractError::Timeout => WarningKind::Timeout,
            ExtractError::LimitExceeded(kind) => WarningKind::LimitExceeded(*kind),
            ExtractError::ContentMismatch => WarningKind::ContentMismatch,
            ExtractError::NoText => WarningKind::NoText,
        })
    }
}

/// Un documento che non si è potuto leggere, con il motivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// Il documento.
    pub doc: Arc<DocPath>,
    /// Il motivo.
    pub kind: WarningKind,
}

/// Come è finita una ricerca.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// Tutte le radici sono state lette.
    Completed,
    /// Annullata dall'utente (o dalla chiusura del [`SearchHandle`](crate::SearchHandle)).
    Cancelled,
    /// Raggiunto [`Limits::max_results`](crate::Limits::max_results).
    LimitReached,
}

/// Riepilogo finale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// Come è finita.
    pub outcome: Outcome,
    /// Tempo trascorso dall'avvio (orologio monotono).
    pub elapsed: Duration,
    /// Conteggi finali.
    pub counts: ProgressSnapshot,
}

/// Un messaggio del worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchEvent {
    /// Risultati di un documento, già filtrati da OR/AND; mai vuoto.
    Hits(Vec<Hit>),
    /// Un documento non letto.
    Warning(Warning),
    /// Fine della ricerca: sempre l'ultimo evento.
    Finished(Summary),
}
