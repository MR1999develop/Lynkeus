// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Punto d'ingresso di `lynkeus-searcher`: costruisce l'app e apre la finestra.

// In release nessuna console accanto alla finestra su Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![deny(unsafe_code)]

use std::process::ExitCode;

use lynkeus_app::{Application, Settings, platform};

/// Dimensione iniziale e minima della finestra, in punti.
const INITIAL_SIZE: [f32; 2] = [1024.0, 680.0];
const MIN_SIZE: [f32; 2] = [640.0, 400.0];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lynkeus-searcher: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let app = Application::build(Settings::default())?;
    let title = app.i18n().tr("app-title");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title.clone())
            .with_inner_size(INITIAL_SIZE)
            .with_min_inner_size(MIN_SIZE),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        &title,
        options,
        Box::new(|cc| {
            platform::install_fonts(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("finestra non avviata: {error}"))
}
