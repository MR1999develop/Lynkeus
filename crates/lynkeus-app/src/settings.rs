// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Impostazioni dell'app, passate a [`Application::build`](crate::Application::build).
//!
//! Provvisorio (fase 0): solo in memoria. La fase 3 le salva in `settings.toml` (versione,
//! campi sconosciuti conservati, scrittura durevole).

use std::path::PathBuf;

use lynkeus_core::{Combine, MatchOptions};
use serde::{Deserialize, Serialize};

use crate::i18n::Language;

/// Preferenze dell'utente e ultimo stato del modulo di ricerca.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Lingua dell'interfaccia; `None` = quella del sistema.
    pub language: Option<Language>,
    /// Opzioni di confronto.
    pub options: MatchOptions,
    /// OR o AND tra i termini.
    pub combine: Combine,
    /// Ultima cartella usata.
    pub last_root: Option<PathBuf>,
    /// Ultimo termine cercato.
    pub last_term: String,
}
