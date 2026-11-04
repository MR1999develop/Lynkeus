// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Punto d'ingresso di `lynkeus-searcher`: legge le impostazioni, costruisce l'app e apre
//! la finestra.
//!
//! Con l'opzione nascosta `--extract-worker` (seguita da `--memory-limit-bytes N`) lo stesso
//! eseguibile è il processo estrattore: legge le richieste da stdin e risponde su stdout
//! (`lynkeus_extract::worker::run`), senza finestre e senza leggere le impostazioni.

// In release nessuna console accanto alla finestra su Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![deny(unsafe_code)]

use std::ffi::OsString;
use std::io;
use std::process::ExitCode;

use lynkeus_app::extraction::ExtractionSetup;
use lynkeus_app::settings::{FILE_NAME, LoadOutcome};
use lynkeus_app::{Application, Settings, SettingsFile, platform, setup_context};
use lynkeus_engine::EXTRACT_WORKER_FLAG;
use lynkeus_extract::worker::{self, WorkerConfig};

/// Dimensione iniziale e minima della finestra, in punti: il layout regge 1024×600 meno
/// barre e dock.
const INITIAL_SIZE: [f32; 2] = [1100.0, 700.0];
const MIN_SIZE: [f32; 2] = [800.0, 480.0];
/// Nome dell'app per eframe (solo per il titolo; la cartella è quella di configurazione).
const APP_NAME: &str = "Lynkeus Searcher";

fn main() -> ExitCode {
    // Prima di ogni altra cosa: nessuna DLL dalla cartella dell'eseguibile o dal PATH. Se
    // non riesce (sistema troppo vecchio) si continua con la ricerca predefinita.
    let dll_search = platform::restrict_dll_search();
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(EXTRACT_WORKER_FLAG.as_ref()) {
        return extract_worker(args);
    }
    if let Err(error) = dll_search {
        eprintln!("lynkeus-searcher: ricerca delle DLL non ristretta: {error}");
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lynkeus-searcher: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Il processo estrattore: nessuna finestra, nessun file scritto; esce quando il processo
/// principale chiude lo stdin.
fn extract_worker(args: impl Iterator<Item = OsString>) -> ExitCode {
    let Ok(config) = WorkerConfig::from_args(args) else {
        return ExitCode::from(2);
    };
    match worker::run(io::stdin().lock(), io::stdout().lock(), &config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

fn run() -> anyhow::Result<()> {
    let config_dir = platform::config_dir();
    let mut app = match &config_dir {
        Some(dir) => {
            let (file, settings, outcome) = SettingsFile::load(dir.join(FILE_NAME));
            report(&outcome);
            Application::build_persistent(settings, file)?
        }
        None => Application::build(Settings::default())?,
    };
    // Senza processo estrattore l'app cerca comunque nei file di testo.
    let connected =
        ExtractionSetup::for_this_program().and_then(|setup| app.connect_extraction(&setup));
    if let Err(error) = connected {
        eprintln!("lynkeus-searcher: documenti non disponibili: {error}");
    }
    let title = app.i18n().tr("app-title");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_app_id("lynkeus-searcher")
            .with_inner_size(INITIAL_SIZE)
            .with_min_inner_size(MIN_SIZE),
        // eframe ricorda solo la finestra, accanto a settings.toml.
        persist_window: true,
        persistence_path: config_dir,
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| {
            setup_context(&cc.egui_ctx, &app);
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("finestra non avviata: {error}"))
}

/// Segnala sullo standard error i problemi del file delle impostazioni: l'app parte
/// comunque con le predefinite.
fn report(outcome: &LoadOutcome) {
    match outcome {
        LoadOutcome::Missing | LoadOutcome::Loaded => {}
        LoadOutcome::Invalid { moved_to } => match moved_to {
            Some(path) => eprintln!(
                "lynkeus-searcher: settings.toml non valido, spostato in {}",
                path.display()
            ),
            None => eprintln!("lynkeus-searcher: settings.toml non valido e non spostato"),
        },
        LoadOutcome::NewerVersion(version) => eprintln!(
            "lynkeus-searcher: settings.toml della versione {version}: non verrà riscritto"
        ),
        LoadOutcome::Unreadable(error) => {
            eprintln!("lynkeus-searcher: settings.toml non leggibile: {error}");
        }
    }
}
