// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Eventi d'estrazione: ciò che un estrattore produce leggendo un file, nello stesso modo
//! nel processo estrattore (dove diventano messaggi del [`protocol`](crate::protocol)) e
//! nei test (dove finiscono in un `Vec`).
//!
//! Sequenza per un file, verificabile con [`SequenceCheck`]:
//!
//! - ogni documento letto è una coppia [`ExtractEvent::DocumentStart`] …
//!   [`ExtractEvent::DocumentEnd`], mai annidata; tra le due solo [`ExtractEvent::Unit`];
//! - un file semplice (docx, PDF) è un documento con `inner` vuoto; un archivio non è un
//!   documento: ogni sua voce lo è, con `inner` = nomi delle voci dall'esterno all'interno;
//! - [`ExtractEvent::Warning`] e [`ExtractEvent::Skipped`] stanno sempre fuori da una coppia
//!   e dicono a quale documento si riferiscono con il proprio `inner`. Un avviso può seguire
//!   la fine di un documento di cui sono già arrivate unità (per esempio un PDF con una
//!   pagina danneggiata): il chiamante decide se tenerle;
//! - un documento che fallisce prima di produrre testo dà solo l'avviso, senza coppia.
//!
//! **Righe dentro un'unità.** Il testo di un'unità può avere più righe (`\n`). Per
//! [`Location::Line`] e [`Location::PageLine`] la posizione è quella della prima riga e le
//! righe seguenti hanno numeri successivi ([`line_location`]); per le altre posizioni tutte
//! le righe dell'unità hanno la stessa posizione (un paragrafo con un a capo resta un
//! paragrafo).
//!
//! I nomi delle voci in `inner` e i nomi dei fogli vengono dal file: sono etichette con i
//! caratteri di controllo e di direzione del testo già sostituiti da U+FFFD, mai percorsi.

use std::fmt;

use lynkeus_core::{Location, TextUnit};
use serde::{Deserialize, Serialize};

use crate::error::ExtractError;
use crate::format::Format;
use crate::limits::LimitKind;

/// Un evento dell'estrazione di un file.
///
/// Viaggia nel protocollo e nella cache: nuove varianti solo in fondo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtractEvent {
    /// Inizia un documento.
    DocumentStart {
        /// Voci d'archivio dall'esterno all'interno; vuoto per il file stesso.
        inner: Vec<String>,
        /// Formato con cui il documento è letto (dopo la verifica del contenuto).
        format: Format,
    },
    /// Un pezzo di testo del documento aperto.
    Unit(TextUnit),
    /// Finisce il documento aperto.
    DocumentEnd,
    /// Un documento (o un archivio) che non si è potuto leggere, del tutto o in parte.
    Warning {
        /// Il documento: voci d'archivio dall'esterno all'interno.
        inner: Vec<String>,
        /// Il motivo.
        kind: WarningKind,
    },
    /// Un documento saltato senza avviso, da contare (come i binari tra i file di testo).
    Skipped {
        /// Il documento: voci d'archivio dall'esterno all'interno.
        inner: Vec<String>,
        /// Il motivo.
        reason: SkipReason,
    },
}

/// Motivo di un avviso.
///
/// Viaggia nel protocollo e nella cache: nuove varianti solo in fondo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WarningKind {
    /// File inesistente o sparito.
    NotFound,
    /// Accesso negato.
    AccessDenied,
    /// File in uso da un altro programma.
    InUse,
    /// Altro errore di lettura dal disco.
    Io,
    /// Protetto da password.
    Encrypted,
    /// Struttura del formato non valida (anche XML con DTD).
    Corrupt,
    /// Tempo scaduto (lo decide il processo principale, mai l'estrattore).
    Timeout,
    /// Un limite superato.
    LimitExceeded(LimitKind),
    /// Contenuto diverso dall'estensione.
    ContentMismatch,
    /// Nessun testo estraibile (PDF scansionato).
    NoText,
}

impl WarningKind {
    /// Il motivo dell'avviso per un errore, o `None` per gli errori che non sono avvisi ma
    /// documenti saltati ([`SkipReason::from_error`]).
    pub fn from_error(error: &ExtractError) -> Option<WarningKind> {
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

/// Perché un documento è stato saltato senza avviso.
///
/// Viaggia nel protocollo e nella cache: nuove varianti solo in fondo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SkipReason {
    /// Voce senza formato riconosciuto che sembra binaria.
    Binary,
    /// Oltre la dimensione massima di un file.
    TooLarge,
    /// Voce d'archivio che non è un file regolare (collegamento simbolico, dispositivo).
    SpecialEntry,
}

impl SkipReason {
    /// Il motivo per gli errori che fanno saltare un documento, o `None`.
    pub fn from_error(error: &ExtractError) -> Option<SkipReason> {
        match error {
            ExtractError::Binary => Some(SkipReason::Binary),
            ExtractError::TooLarge { .. } => Some(SkipReason::TooLarge),
            _ => None,
        }
    }
}

/// Chi riceve gli eventi ha chiesto di fermarsi (annullamento, canale chiuso): l'estrazione
/// termina subito senza altri eventi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopped;

impl fmt::Display for Stopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("estrazione fermata da chi riceve gli eventi")
    }
}

impl std::error::Error for Stopped {}

/// Destinatario degli eventi d'estrazione.
pub trait EventSink {
    /// Riceve un evento; [`Stopped`] ferma l'estrazione.
    fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped>;
}

impl EventSink for Vec<ExtractEvent> {
    fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped> {
        self.push(event);
        Ok(())
    }
}

impl<S: EventSink + ?Sized> EventSink for &mut S {
    fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped> {
        (**self).emit(event)
    }
}

/// La posizione della riga `line_in_unit` (0-based, come
/// [`WordMatch::line`](lynkeus_core::WordMatch::line)) di un'unità in `location`.
///
/// Per [`Location::Line`] e [`Location::PageLine`] aggiunge `line_in_unit` al numero di riga
/// (saturando); le altre posizioni non cambiano.
pub fn line_location(location: &Location, line_in_unit: u32) -> Location {
    match location {
        Location::Line(line) => Location::Line(line.saturating_add(line_in_unit)),
        Location::PageLine { page, line } => Location::PageLine {
            page: *page,
            line: line.saturating_add(line_in_unit),
        },
        other => other.clone(),
    }
}

/// Una violazione della sequenza degli eventi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SequenceError {
    /// Inizio di un documento mentre un altro è aperto.
    #[error("documento annidato")]
    NestedDocument,
    /// Unità o fine documento senza un documento aperto.
    #[error("unità o fine fuori da un documento")]
    OutsideDocument,
    /// Avviso o documento saltato dentro un documento aperto.
    #[error("avviso dentro un documento")]
    NoticeInsideDocument,
    /// Fine dell'estrazione con un documento ancora aperto.
    #[error("documento non chiuso")]
    Unclosed,
}

/// Verifica che una serie di eventi rispetti la sequenza descritta nel modulo; per il
/// processo principale, che non accetta messaggi fuori sequenza, e per i test.
#[derive(Debug, Clone, Default)]
pub struct SequenceCheck {
    open: bool,
}

impl SequenceCheck {
    /// Un controllo all'inizio di un file.
    pub fn new() -> SequenceCheck {
        SequenceCheck::default()
    }

    /// Accetta il prossimo evento o dice quale regola viola.
    pub fn accept(&mut self, event: &ExtractEvent) -> Result<(), SequenceError> {
        match (event, self.open) {
            (ExtractEvent::DocumentStart { .. }, false) => self.open = true,
            (ExtractEvent::DocumentStart { .. }, true) => {
                return Err(SequenceError::NestedDocument);
            }
            (ExtractEvent::DocumentEnd, true) => self.open = false,
            (ExtractEvent::Unit(_) | ExtractEvent::DocumentEnd, false) => {
                return Err(SequenceError::OutsideDocument);
            }
            (ExtractEvent::Warning { .. } | ExtractEvent::Skipped { .. }, true) => {
                return Err(SequenceError::NoticeInsideDocument);
            }
            (ExtractEvent::Unit(_), true)
            | (ExtractEvent::Warning { .. } | ExtractEvent::Skipped { .. }, false) => {}
        }
        Ok(())
    }

    /// Fine del file: nessun documento deve restare aperto.
    pub fn finish(&self) -> Result<(), SequenceError> {
        if self.open {
            Err(SequenceError::Unclosed)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> ExtractEvent {
        ExtractEvent::DocumentStart {
            inner: Vec::new(),
            format: Format::Docx,
        }
    }

    fn unit() -> ExtractEvent {
        ExtractEvent::Unit(TextUnit {
            location: Location::Paragraph(1),
            text: "aragosta".to_owned(),
        })
    }

    fn warning() -> ExtractEvent {
        ExtractEvent::Warning {
            inner: Vec::new(),
            kind: WarningKind::Corrupt,
        }
    }

    fn check(events: &[ExtractEvent]) -> Result<(), SequenceError> {
        let mut check = SequenceCheck::new();
        for event in events {
            check.accept(event)?;
        }
        check.finish()
    }

    #[test]
    fn sequence_partitions() {
        // Valide: vuota, documento con unità, avviso dopo la fine, solo avviso.
        assert_eq!(check(&[]), Ok(()));
        assert_eq!(
            check(&[start(), unit(), ExtractEvent::DocumentEnd, warning()]),
            Ok(())
        );
        assert_eq!(check(&[warning()]), Ok(()));
        // Non valide.
        assert_eq!(
            check(&[start(), start()]),
            Err(SequenceError::NestedDocument)
        );
        assert_eq!(check(&[unit()]), Err(SequenceError::OutsideDocument));
        assert_eq!(
            check(&[ExtractEvent::DocumentEnd]),
            Err(SequenceError::OutsideDocument)
        );
        assert_eq!(
            check(&[start(), warning()]),
            Err(SequenceError::NoticeInsideDocument)
        );
        assert_eq!(check(&[start(), unit()]), Err(SequenceError::Unclosed));
    }

    #[test]
    fn line_location_partitions() {
        assert_eq!(line_location(&Location::Line(5), 2), Location::Line(7));
        assert_eq!(
            line_location(&Location::Line(u32::MAX), 1),
            Location::Line(u32::MAX)
        );
        assert_eq!(
            line_location(&Location::PageLine { page: 2, line: 1 }, 2),
            Location::PageLine { page: 2, line: 3 }
        );
        assert_eq!(
            line_location(&Location::Paragraph(4), 3),
            Location::Paragraph(4)
        );
    }

    #[test]
    fn errors_map_to_warning_or_skip() {
        assert_eq!(
            WarningKind::from_error(&ExtractError::Encrypted),
            Some(WarningKind::Encrypted)
        );
        assert_eq!(WarningKind::from_error(&ExtractError::Binary), None);
        assert_eq!(
            SkipReason::from_error(&ExtractError::Binary),
            Some(SkipReason::Binary)
        );
        assert_eq!(SkipReason::from_error(&ExtractError::NoText), None);
        let gone = ExtractError::Io(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert_eq!(WarningKind::from_error(&gone), Some(WarningKind::NotFound));
    }
}
