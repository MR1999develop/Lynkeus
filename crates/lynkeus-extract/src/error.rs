// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Errori di lettura ed estrazione, uno per reazione diversa del chiamante.
//!
//! Il motore conta [`ExtractError::Binary`] e [`ExtractError::TooLarge`] tra i file saltati;
//! tutte le altre varianti diventano un avviso con il loro motivo. Il testo di `Display` è
//! per i log: i messaggi per l'utente li compone l'interfaccia con Fluent.

use std::io;

use crate::limits::LimitKind;

/// Perché un file non ha prodotto testo.
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    /// Il file sembra binario (NUL nei primi byte, firma nota): si salta e si conta.
    #[error("file binario")]
    Binary,
    /// Il file supera la dimensione massima ammessa.
    #[error("file di {size} byte, oltre il massimo di {max}")]
    TooLarge {
        /// Dimensione del file, in byte.
        size: u64,
        /// Massimo ammesso, in byte.
        max: u64,
    },
    /// Accesso negato dal sistema (permessi).
    #[error("accesso negato")]
    AccessDenied,
    /// File aperto da un altro programma in modo esclusivo.
    #[error("file in uso da un altro programma")]
    InUse,
    /// Altro errore di I/O (disco, rete, file sparito durante la ricerca).
    #[error("errore di lettura")]
    Io(#[source] io::Error),
    /// Documento o archivio protetto da password.
    #[error("protetto da password")]
    Encrypted,
    /// Struttura del formato non valida.
    #[error("file danneggiato: {reason}")]
    Corrupt {
        /// Descrizione tecnica breve, per i log; mai mostrata all'utente.
        reason: String,
    },
    /// L'estrazione ha superato il tempo massimo per file.
    #[error("tempo scaduto")]
    Timeout,
    /// Un limite sulle risorse è stato superato (archivi, memoria).
    #[error("limite superato: {0:?}")]
    LimitExceeded(LimitKind),
    /// Il contenuto non corrisponde all'estensione e non è un caso innocuo noto.
    #[error("contenuto diverso dall'estensione")]
    ContentMismatch,
    /// Documento senza testo estraibile (es. PDF scansionato).
    #[error("nessun testo estraibile")]
    NoText,
}

impl ExtractError {
    /// Classifica un errore di I/O: permessi e condivisione hanno un avviso proprio.
    pub fn from_io(error: io::Error) -> ExtractError {
        match error.kind() {
            io::ErrorKind::PermissionDenied => ExtractError::AccessDenied,
            io::ErrorKind::ResourceBusy => ExtractError::InUse,
            _ if is_sharing_violation(&error) => ExtractError::InUse,
            _ => ExtractError::Io(error),
        }
    }
}

/// `ERROR_SHARING_VIOLATION` (32) e `ERROR_LOCK_VIOLATION` (33) su Windows.
fn is_sharing_violation(error: &io::Error) -> bool {
    cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33))
}

impl From<io::Error> for ExtractError {
    fn from(error: io::Error) -> Self {
        ExtractError::from_io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_are_classified() {
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert!(matches!(
            ExtractError::from_io(denied),
            ExtractError::AccessDenied
        ));
        let other = io::Error::from(io::ErrorKind::UnexpectedEof);
        assert!(matches!(ExtractError::from_io(other), ExtractError::Io(_)));
    }
}
