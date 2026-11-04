// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Lynkeus Searcher, l'applicazione: stato e casi d'uso ([`Application`]), finestra egui,
//! impostazioni, testi Fluent e integrazione con il sistema.
//!
//! Il binario `lynkeus-searcher` è solo il punto d'ingresso; i test d'accettazione usano
//! [`Application::build`] e l'API senza finestra.

#![deny(unsafe_code)]

mod app;
pub mod format;
pub mod i18n;
pub mod platform;
mod settings;
mod ui;

pub use app::{
    Application, MAX_HITS_PER_FRAME, Pump, Results, SearchForm, SearchStatus, StartError,
};
pub use settings::Settings;
