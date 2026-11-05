// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Lynkeus Searcher, l'applicazione: stato e casi d'uso ([`Application`]), finestra egui,
//! impostazioni, testi Fluent, aspetto e integrazione con il sistema.
//!
//! Il binario `lynkeus-searcher` è solo il punto d'ingresso; i test d'accettazione usano
//! [`Application::build`] e l'API senza finestra, i test dell'interfaccia la stessa app
//! dentro `egui_kittest` dopo [`setup_context`].

#![deny(unsafe_code)]

mod app;
pub mod cli;
pub mod context_menu;
pub mod diagnostics;
pub mod export;
pub mod extraction;
pub mod form;
pub mod format;
pub mod i18n;
pub mod instance;
pub mod launch;
mod notices;
pub mod platform;
pub mod preview;
pub mod private_dirs;
pub mod results;
mod root_check;
pub mod settings;
pub mod theme;
mod ui;

pub use app::{
    Application, CountedAt, ESTIMATE_DELAY, EmptyState, FileChoice, MAX_HITS_PER_FRAME,
    PROGRESS_DELAY, PreviewKey, PreviewState, Progress, Pump, Results, SearchStatus, StartError,
    Tab, ViewState,
};
pub use notices::Notice;
pub use settings::{Settings, SettingsFile};
pub use ui::setup_context;
