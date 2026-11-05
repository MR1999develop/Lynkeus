// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Punto d'ingresso di `lynkeus-searcher` e composition root: prepara le cartelle private,
//! installa log e rapporti di crash, legge le impostazioni, decide l'istanza unica per gli
//! avvii con `--add`, costruisce l'app e apre la finestra.
//!
//! Con l'opzione nascosta `--extract-worker` (seguita da `--memory-limit-bytes N`) lo stesso
//! eseguibile è il processo estrattore: legge le richieste da stdin e risponde su stdout
//! (`lynkeus_extract::worker::run`), senza finestre, senza log e senza leggere le
//! impostazioni.
//!
//! Con `--add <percorso>` (clic destro) o con percorsi senza opzione (cartella trascinata
//! sull'icona, "Invia a") la finestra si apre con quei luoghi; se un'altra istanza avviata
//! con dei percorsi è già aperta, i percorsi vanno a lei e questo processo esce.

// In release nessuna console accanto alla finestra su Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![deny(unsafe_code)]

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use lynkeus_app::cli::parse_added_paths;
use lynkeus_app::diagnostics::{self, LogLevel};
use lynkeus_app::extraction::ExtractionSetup;
use lynkeus_app::instance::{self, Startup};
use lynkeus_app::platform::{self, AppDirs};
use lynkeus_app::private_dirs::{self, PreparedDirs};
use lynkeus_app::settings::{FILE_NAME, LoadOutcome};
use lynkeus_app::{Application, Notice, Settings, SettingsFile, setup_context};
use lynkeus_engine::{EXTRACT_WORKER_FLAG, PrivateFs};
use lynkeus_extract::worker::{self, WorkerConfig};

/// Dimensione iniziale e minima della finestra, in punti: il layout regge 1024×600 meno
/// barre e dock.
const INITIAL_SIZE: [f32; 2] = [1100.0, 700.0];
const MIN_SIZE: [f32; 2] = [800.0, 480.0];
/// Nome dell'app per eframe (solo per il titolo; la cartella è quella di configurazione).
const APP_NAME: &str = "Lynkeus Searcher";
/// Icona della finestra (la stessa dell'eseguibile, generata da `assets/src`).
const WINDOW_ICON_PNG: &[u8] = include_bytes!("../../../assets/icon-256.png");
/// Il file in cui eframe ricorda posizione e dimensione della finestra, nella cartella di
/// configurazione accanto a `settings.toml`.
const WINDOW_STATE_FILE: &str = "window.ron";

fn main() -> ExitCode {
    // Prima di ogni altra cosa: le DLL caricate da qui in poi (grafica, dialoghi) solo dalla
    // cartella di sistema, mai dalla cartella dell'eseguibile o dal PATH. Quelle importate
    // dall'eseguibile sono già caricate, con la stessa regola (`/DEPENDENTLOADFLAG:0x800`,
    // verificato da `cargo xtask dist`). Se non riesce (sistema troppo vecchio) si continua
    // e lo si registra.
    let dll_search = platform::restrict_dll_search();
    let mut args = std::env::args_os().skip(1).peekable();
    if args.peek().map(OsString::as_os_str) == Some(EXTRACT_WORKER_FLAG.as_ref()) {
        args.next();
        return extract_worker(args);
    }
    let added = match parse_added_paths(args) {
        Ok(added) => added,
        Err(error) => {
            eprintln!("lynkeus-searcher: {error}; uso: lynkeus-searcher [[--add] <percorso>]…");
            return ExitCode::from(2);
        }
    };
    let private_fs = platform::private_fs();
    let dirs = AppDirs::for_user()
        .map(|dirs| private_dirs::prepare(&dirs, private_fs.as_ref()))
        .unwrap_or_default();
    // Log e rapporti di crash prima dei thread; il livello vero arriva con le impostazioni.
    let log_control = diagnostics::init_logging(dirs.logs.as_deref(), LogLevel::default());
    if let Some(dir) = &dirs.crash_reports {
        diagnostics::install_crash_reporter(dir.clone(), Arc::clone(&private_fs));
    }
    if let Err(error) = dll_search {
        tracing::warn!(%error, "ricerca delle DLL non ristretta");
    }
    let outcome = run(&dirs, &private_fs, &added, log_control);
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = format!("{error:#}"), "avvio non riuscito");
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

/// I percorsi di `--add` accettati dal guardiano unico, resi assoluti rispetto alla
/// cartella corrente; gli altri si scartano (e si contano nel log).
fn accepted_added_paths(added: &[PathBuf]) -> Vec<PathBuf> {
    let accepted: Vec<PathBuf> = added
        .iter()
        .filter_map(|path| std::path::absolute(path).ok())
        .filter_map(|path| instance::accept_path(&path).ok())
        .collect();
    let refused = added.len() - accepted.len();
    if refused > 0 {
        tracing::warn!(refused, "percorsi di --add scartati");
    }
    accepted
}

fn run(
    dirs: &PreparedDirs,
    private_fs: &Arc<dyn PrivateFs>,
    added: &[PathBuf],
    log_control: Option<diagnostics::LogControl>,
) -> anyhow::Result<()> {
    let added = accepted_added_paths(added);
    let launched_with_add = !added.is_empty();
    // Istanza unica solo per gli avvii con `--add` (clic destro).
    let server = match (&dirs.data, launched_with_add) {
        (Some(data), true) => match instance::start_or_deliver(data, private_fs, &added) {
            Startup::Delivered => return Ok(()),
            Startup::Server(server) => Some(server),
            Startup::Alone => None,
        },
        _ => None,
    };
    let mut app = match &dirs.config {
        Some(dir) => {
            let (file, settings, outcome) = SettingsFile::load(dir.join(FILE_NAME));
            report(&outcome);
            Application::build_persistent(settings, file)?
        }
        None => Application::build(Settings::default())?,
    };
    if let Some(control) = log_control {
        app.set_log_control(control);
    }
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "avvio");
    if !dirs.refused.is_empty() {
        tracing::warn!(
            count = dirs.refused.len(),
            "cartelle non private: funzioni spente"
        );
        app.set_notice(Notice::PrivateDirsRefused {
            count: dirs.refused.len(),
        });
    }
    if launched_with_add {
        app.open_with_roots(added);
    }
    if let Some(server) = server {
        app.attach_instance(server);
    }
    // Senza processo estrattore l'app cerca comunque nei file di testo.
    let connected =
        ExtractionSetup::for_this_program(dirs).and_then(|setup| app.connect_extraction(&setup));
    if let Err(error) = connected {
        tracing::warn!(%error, "documenti non disponibili");
    }
    connect_context_menu(&mut app);

    let title = app.i18n().tr("app-title");
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(title)
        .with_app_id("lynkeus-searcher")
        .with_inner_size(INITIAL_SIZE)
        .with_min_inner_size(MIN_SIZE);
    match eframe::icon_data::from_png_bytes(WINDOW_ICON_PNG) {
        Ok(icon) => viewport = viewport.with_icon(Arc::new(icon)),
        Err(error) => tracing::warn!(%error, "icona della finestra non letta"),
    }
    let options = native_options(viewport, dirs.config.as_deref());
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| {
            setup_context(&cc.egui_ctx, &app);
            // Con l'opzione accesa, la voce del clic destro segue l'eseguibile se è stato
            // spostato.
            let ctx = cc.egui_ctx.clone();
            app.refresh_context_menu(&(Arc::new(move || ctx.request_repaint()) as _));
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("finestra non avviata: {error}"))
}

/// Le opzioni della finestra: `viewport` e, con la cartella di configurazione privata
/// `config_dir`, posizione e dimensione ricordate da eframe nel file [`WINDOW_STATE_FILE`]
/// (eframe vuole il percorso di un file RON, non di una cartella). Senza la cartella la
/// finestra non si ricorda: nessun file fuori dalle cartelle private (eframe crea comunque
/// la sua cartella predefinita, e la lascia vuota).
fn native_options(
    viewport: egui::ViewportBuilder,
    config_dir: Option<&Path>,
) -> eframe::NativeOptions {
    let window_state = config_dir.map(|dir| dir.join(WINDOW_STATE_FILE));
    eframe::NativeOptions {
        viewport,
        persist_window: window_state.is_some(),
        persistence_path: window_state,
        ..eframe::NativeOptions::default()
    }
}

/// Collega la voce del clic destro (registro dell'utente o script di Nautilus) per questo
/// eseguibile; senza, la voce del menu resta disabilitata con il motivo.
fn connect_context_menu(app: &mut Application) {
    let store = platform::ContextMenu::for_current_user();
    let exe = std::env::current_exe();
    match (store, exe) {
        (Ok(store), Ok(exe)) => app.connect_context_menu(Arc::new(store), exe),
        (Err(error), _) | (_, Err(error)) => {
            tracing::warn!(%error, "voce del clic destro non disponibile");
        }
    }
}

/// Registra i problemi del file delle impostazioni: l'app parte comunque con le
/// predefinite.
fn report(outcome: &LoadOutcome) {
    match outcome {
        LoadOutcome::Missing | LoadOutcome::Loaded => {}
        LoadOutcome::Invalid { moved_to } => {
            let moved = moved_to.is_some();
            tracing::warn!(moved, "settings.toml non valido");
        }
        LoadOutcome::NewerVersion(version) => {
            tracing::warn!(
                version,
                "settings.toml di una versione più recente: non verrà riscritto"
            );
        }
        LoadOutcome::Unreadable(error) => {
            tracing::warn!(%error, "settings.toml non leggibile");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-D05: eframe riceve un file dentro la cartella di configurazione, non la cartella;
    /// senza cartella la finestra non si ricorda (e nulla si scrive altrove).
    #[test]
    fn window_state_file_is_inside_the_config_dir() {
        let config = std::env::temp_dir().join("lynkeus-config");
        let options = native_options(egui::ViewportBuilder::default(), Some(&config));
        assert!(options.persist_window);
        assert_eq!(
            options.persistence_path,
            Some(config.join(WINDOW_STATE_FILE))
        );
        let without = native_options(egui::ViewportBuilder::default(), None);
        assert!(!without.persist_window);
        assert_eq!(without.persistence_path, None);
    }
}
