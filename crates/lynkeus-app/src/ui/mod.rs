// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! La finestra: vista umile sopra [`Application`].
//!
//! Struttura: menu in alto, banner (solo al limite dei risultati), stato in basso, modulo di
//! ricerca a sinistra, schede Risultati e Avvisi al centro. I gesti diventano [`Action`]
//! raccolte durante il disegno e applicate dopo, mai mentre si itera sui dati.
//!
//! Il worker si svuota in [`eframe::App::logic`], che gira anche a finestra nascosta, con
//! un limite di righe per frame; si ridisegna solo su richiesta (eventi del worker, timer
//! dello stato durante la ricerca, input).
//!
//! Nulla di lento sul thread dell'interfaccia: i dialoghi di sistema si aprono in un thread
//! (la finestra continua a ridisegnarsi e la ricerca a ricevere righe), la copia e il
//! salvataggio delle impostazioni li prepara l'app in un thread.

mod commands;
mod export_dialog;
mod menu;
mod notice;
mod panes;
mod preview;
mod search_panel;
mod status;
mod table;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use egui::{Align2, Id, RichText, Ui};
use lynkeus_engine::{DocPath, Notify};

use crate::app::{Application, FileChoice, MAX_HITS_PER_FRAME, Notice, Pump, Tab};
use crate::diagnostics::LogLevel;
use crate::export::ExportFormat;
use crate::form::{FormField, RootKind};
use crate::i18n::Language;
use crate::results::{ClickKind, SortColumn};
use crate::settings::LayoutSettings;
use crate::theme::{self, SPACE, ThemeChoice};

pub(crate) use commands::Command;

/// Ogni quanto si ridisegna lo stato durante una ricerca (tempo trascorso, contatori).
const STATUS_REFRESH: Duration = Duration::from_millis(250);

/// Un gesto dell'utente, applicato dopo il disegno del frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Action {
    Command(Command),
    AddRoots(Vec<PathBuf>),
    RemoveRoot(usize),
    AddTypedPath,
    RemoveTerm(usize),
    InsertTerm(String),
    TermsEdited,
    Sort(SortColumn),
    ClickRow(usize, ClickKind),
    MoveCursor(isize, bool),
    SetTheme(ThemeChoice),
    SetLanguage(Option<Language>),
    SetCacheEnabled(bool),
    SetCacheSize(u64),
    SetContextMenu(bool),
    SetLogLevel(LogLevel),
    ClearCache,
    ResetFilters,
    UndoFiltersReset,
    DismissNotice,
    Reveal(Arc<DocPath>),
    /// Chiude il dialogo delle impostazioni di esportazione; con `true` le applica ed
    /// esporta.
    CloseExportDialog(bool),
}

/// Chi avvisare quando un lavoro in background finisce: un nuovo frame.
fn repaint(ctx: &egui::Context) -> Notify {
    let ctx = ctx.clone();
    Arc::new(move || ctx.request_repaint())
}

/// Prepara il contesto: font, token del tema (o quelli del contrasto elevato del sistema,
/// se [`Application::set_system_contrast`] li ha dati), preferenza Sistema/Chiaro/Scuro. Da
/// chiamare una volta alla creazione della finestra (anche nei test dell'interfaccia).
pub fn setup_context(ctx: &egui::Context, app: &Application) {
    crate::platform::install_fonts(ctx);
    theme::apply(ctx, app.theme(), app.system_contrast());
}

impl eframe::App for Application {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.has_root_check_waker() {
            self.set_root_check_waker(repaint(ctx));
            // Anche l'istanza unica e i lavori senza vista, una volta sola, con lo stesso
            // contesto.
            self.set_instance_waker(repaint(ctx));
            self.set_waker(repaint(ctx));
        }
        self.pump_root_checks();
        if self.pump_instance() {
            // Percorsi da un'altra istanza: la finestra in primo piano (l'altra istanza ha
            // chiamato AllowSetForegroundWindow).
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if self.pump_events(MAX_HITS_PER_FRAME) == Pump::MorePending {
            ctx.request_repaint();
        }
        if self.is_searching() || self.is_copying() {
            ctx.request_repaint_after(STATUS_REFRESH);
        }
        self.pump_actions();
        self.update_preview(&repaint(ctx));
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        if let Some(text) = self.take_clipboard_text() {
            ui.ctx().copy_text(text);
        }
        // Le zone per F6 si registrano di nuovo mentre si disegnano.
        panes::forget(ui.ctx());
        let mut actions = Vec::new();
        read_shortcuts(self, ui.ctx(), &mut actions);
        read_dropped_files(ui.ctx(), &mut actions);

        egui::Panel::top("menu").show(ui, |ui| menu::bar(self, ui, &mut actions));
        if self.limit_reached() {
            egui::Panel::top("limit_banner").show(ui, |ui| limit_banner(self, ui));
        }
        if self.notice().is_some() {
            egui::Panel::top("notice").show(ui, |ui| notice::bar(self, ui, &mut actions));
        }
        egui::Panel::bottom("status").show(ui, |ui| status::bar(self, ui));
        if self.layout.search_panel_visible {
            let range = LayoutSettings::PANEL_RANGE;
            let shown = egui::Panel::left("search_panel")
                .resizable(true)
                .size_range(*range.start()..=*range.end())
                .default_size(self.layout.search_panel_width)
                .show(ui, |ui| search_panel::show(self, ui, &mut actions));
            self.layout.search_panel_width = shown.response.rect.width();
        }
        egui::CentralPanel::default().show(ui, |ui| central(self, ui, &mut actions));
        about_window(self, ui.ctx());
        export_dialog::show(self, ui.ctx(), &mut actions);
        drop_hint(self, ui.ctx());

        for action in actions {
            apply(self, ui.ctx(), frame, action);
        }
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        // eframe ricorda la finestra; le impostazioni vanno in settings.toml, scritte in un
        // thread. Un errore non ferma l'app: si dice una volta e si riprova al prossimo
        // salvataggio.
        self.save_settings_in_background();
    }

    fn on_exit(&mut self) {
        if let Err(error) = self.save_settings() {
            tracing::warn!(%error, "impostazioni non salvate");
        }
        self.cancel_search();
        self.shutdown_extraction();
        self.shutdown_instance();
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }
}

/// Le scorciatoie, lette una volta per frame nell'ordine di [`Command::SHORTCUT_ORDER`].
///
/// Con un dialogo aperto nessuna: Invio ed Esc sono del dialogo. Con un menu o un popup
/// aperto Esc e Invio restano a lui (Esc lo chiude, la ricerca continua). Invio apre il
/// risultato dalla riga con il focus o se nessun controllo ha il focus (altrimenti attiva
/// quel controllo). Ctrl+C arriva da eframe come evento "copia", non come tasto. Le frecce
/// muovono il cursore della tabella dalla riga con il focus, o se nessun controllo ce l'ha.
fn read_shortcuts(app: &Application, ctx: &egui::Context, actions: &mut Vec<Action>) {
    if app.view_state.export_dialog.is_some() || app.view_state.about_open {
        return;
    }
    let typing = ctx.text_edit_focused();
    let popup_open = egui::Popup::is_any_open(ctx);
    let focused = ctx.memory(egui::Memory::focused);
    let row_focused = focused.is_some() && focused == table::cursor_row_id(app);
    let other_focused = focused.is_some() && !row_focused;
    for command in Command::SHORTCUT_ORDER {
        let Some(shortcut) = command.shortcut() else {
            continue;
        };
        if typing && command.yields_to_text_fields() {
            continue;
        }
        if popup_open && command.yields_to_popups() {
            continue;
        }
        if other_focused && command == Command::Open {
            continue;
        }
        if command.availability(app).is_err() {
            continue;
        }
        let pressed = ctx.input_mut(|i| {
            let copy_event = command == Command::Copy && consume_copy_event(i);
            copy_event || i.consume_shortcut(&shortcut)
        });
        if pressed {
            actions.push(Action::Command(command));
        }
    }
    let table_keys = !typing && !popup_open && !other_focused;
    if table_keys && app.view_state.tab == Tab::Results && !app.view().is_empty() {
        let (down, up, shift) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowUp),
                i.modifiers.shift,
            )
        });
        if down {
            actions.push(Action::MoveCursor(1, shift));
        }
        if up {
            actions.push(Action::MoveCursor(-1, shift));
        }
    }
}

/// Toglie dagli eventi del frame la richiesta "copia" (Ctrl+C); vero se c'era.
fn consume_copy_event(input: &mut egui::InputState) -> bool {
    let before = input.events.len();
    input
        .events
        .retain(|event| !matches!(event, egui::Event::Copy));
    input.events.len() != before
}

/// Cartelle e file trascinati nella finestra.
fn read_dropped_files(ctx: &egui::Context, actions: &mut Vec<Action>) {
    let dropped: Vec<PathBuf> = ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|file| file.path().to_owned())
            .filter(|path| !path.as_os_str().is_empty())
            .collect()
    });
    if !dropped.is_empty() {
        actions.push(Action::AddRoots(dropped));
    }
}

/// Mentre si trascinano file sopra la finestra, un velo dice che cosa succede al rilascio.
fn drop_hint(app: &Application, ctx: &egui::Context) {
    if ctx.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let tokens = theme::current(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        Id::unique("drop_hint"),
    ));
    let screen = ctx.content_rect();
    painter.rect_filled(screen, 0.0, tokens.bg.gamma_multiply(0.9));
    painter.text(
        screen.center(),
        Align2::CENTER_CENTER,
        app.i18n().tr("drop-hint"),
        egui::FontId::new(theme::FONT[3], theme::semibold_family()),
        tokens.text_strong,
    );
}

fn limit_banner(app: &Application, ui: &mut Ui) {
    let i18n = app.i18n();
    let tokens = theme::current(ui.ctx());
    ui.add_space(SPACE[0]);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(i18n.tr("banner-limit-title"))
                .color(tokens.warning)
                .family(theme::semibold_family()),
        );
        ui.label(i18n.tr_args(
            "banner-limit-text",
            &[("max", app.limits().max_results.into())],
        ));
    });
    ui.add_space(SPACE[0]);
}

fn central(app: &mut Application, ui: &mut Ui, actions: &mut Vec<Action>) {
    let i18n = app.i18n().clone();
    let results = app.results();
    let hit_count = results.hits.len();
    let warning_count = results.warnings.len();
    ui.horizontal(|ui| {
        let tab = &mut app.view_state.tab;
        ui.selectable_value(
            tab,
            Tab::Results,
            i18n.tr_args("tab-results", &[("count", hit_count.into())]),
        );
        ui.selectable_value(
            tab,
            Tab::Warnings,
            i18n.tr_args("tab-warnings", &[("count", warning_count.into())]),
        );
    });
    ui.separator();
    match app.view_state.tab {
        Tab::Results => {
            if app.layout.preview_visible && !app.view().is_empty() {
                let range = LayoutSettings::PREVIEW_RANGE;
                let shown = egui::Panel::bottom("preview")
                    .resizable(true)
                    .size_range(*range.start()..=*range.end())
                    .default_size(app.layout.preview_height)
                    .show(ui, |ui| preview::show(app, ui, actions));
                let height = shown.response.rect.height();
                if height.is_finite() {
                    app.layout.preview_height = height.clamp(*range.start(), *range.end());
                }
            }
            table::results(app, ui, actions);
        }
        Tab::Warnings => table::warnings(app, ui),
    }
}

fn about_window(app: &mut Application, ctx: &egui::Context) {
    if !app.view_state.about_open {
        return;
    }
    let i18n = app.i18n().clone();
    let modal = egui::Modal::new(Id::unique("about")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.heading(i18n.tr("app-title"));
        ui.label(i18n.tr_args(
            "about-version",
            &[("version", env!("CARGO_PKG_VERSION").into())],
        ));
        ui.add_space(SPACE[1]);
        ui.label(i18n.tr("about-description"));
        ui.label(i18n.tr("about-license"));
        ui.add_space(SPACE[2]);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.button(i18n.tr("action-close")).clicked()
        })
        .inner
    });
    if modal.inner || modal.should_close() {
        app.view_state.about_open = false;
    }
}

fn apply(app: &mut Application, ctx: &egui::Context, frame: &eframe::Frame, action: Action) {
    match action {
        Action::Command(command) => run_command(app, ctx, frame, command),
        Action::AddRoots(paths) => {
            for path in paths {
                app.add_root(path);
            }
        }
        Action::RemoveRoot(index) => app.form.remove_root(index),
        Action::AddTypedPath => app.add_typed_path(),
        Action::RemoveTerm(index) => {
            app.form.remove_term(index);
            app.clear_start_error();
        }
        Action::InsertTerm(term) => {
            app.form.insert_term(&term);
            app.clear_start_error();
        }
        Action::TermsEdited => app.clear_start_error(),
        Action::Sort(column) => app.sort_by(column),
        Action::ClickRow(position, kind) => app.view_mut().click(position, kind),
        Action::MoveCursor(delta, extend) => app.view_mut().move_cursor(delta, extend),
        Action::SetTheme(choice) => {
            app.set_theme(choice);
            theme::set_choice(ctx, choice);
        }
        Action::SetLanguage(language) => app.set_language(language),
        Action::SetCacheEnabled(enabled) => app.set_cache_enabled(enabled),
        Action::SetCacheSize(mib) => app.set_cache_size_mib(mib),
        Action::ClearCache => app.clear_cache(&repaint(ctx)),
        Action::SetContextMenu(enabled) => app.set_context_menu(enabled, &repaint(ctx)),
        Action::SetLogLevel(level) => app.set_log_level(level),
        Action::ResetFilters => app.reset_filters(),
        Action::UndoFiltersReset => app.undo_filters_reset(),
        Action::DismissNotice => app.dismiss_notice(),
        Action::Reveal(doc) => {
            app.dismiss_notice();
            app.reveal(&doc, &repaint(ctx));
        }
        Action::CloseExportDialog(confirmed) => {
            let draft = app.view_state.export_dialog.take();
            if let (true, Some(settings)) = (confirmed, draft) {
                app.set_export_settings(settings);
                if Command::Export.availability(app).is_ok() {
                    export(app, ctx, frame);
                }
            }
        }
    }
}

/// Chiede dove salvare (dialogo di sistema, che conferma la sovrascrittura di un file
/// esistente) ed esporta con le ultime scelte, quando l'utente ha scelto.
fn export(app: &mut Application, ctx: &egui::Context, frame: &eframe::Frame) {
    let i18n = app.i18n();
    let settings = app.export_settings();
    let extension = settings.format.extension();
    let filter_id = match settings.format {
        ExportFormat::Csv => "export-format-csv",
        ExportFormat::Txt => "export-format-txt",
    };
    let mut dialog = rfd::FileDialog::new()
        .set_title(i18n.tr("dialog-export"))
        .set_file_name(format!("{}.{extension}", i18n.tr("export-file-name")))
        .add_filter(i18n.tr(filter_id), &[extension]);
    if let Some(dir) = &settings.last_dir {
        dialog = dialog.set_directory(dir);
    }
    let dialog = dialog.set_parent(frame);
    open_dialog(app, ctx, move || {
        dialog.save_file().map(FileChoice::ExportTo)
    });
}

/// Apre un dialogo di sistema in un thread: intanto la finestra si ridisegna e la ricerca
/// continua; la scelta arriva all'app, che la applica ([`Application::wait_for_dialog`]).
/// Un dialogo alla volta: con uno già aperto non fa nulla.
fn open_dialog(
    app: &mut Application,
    ctx: &egui::Context,
    choose: impl FnOnce() -> Option<FileChoice> + Send + 'static,
) {
    if app.is_dialog_open() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    let spawned = thread::Builder::new()
        .name("dialogo".to_owned())
        .spawn(move || {
            let choice = choose();
            if tx.send(choice).is_ok() {
                ctx.request_repaint();
            }
        });
    match spawned {
        Ok(_) => app.wait_for_dialog(rx),
        Err(_) => app.set_notice(Notice::NoThread),
    }
}

fn run_command(
    app: &mut Application,
    ctx: &egui::Context,
    frame: &eframe::Frame,
    command: Command,
) {
    match command {
        Command::AddFolder => {
            let dialog = dialog(app, frame, "dialog-add-folder");
            open_dialog(app, ctx, move || {
                dialog.pick_folders().map(FileChoice::AddRoots)
            });
        }
        Command::AddFile => {
            let dialog = dialog(app, frame, "dialog-add-file");
            open_dialog(app, ctx, move || {
                dialog.pick_files().map(FileChoice::AddRoots)
            });
        }
        Command::Open => app.open_current(&repaint(ctx)),
        Command::Reveal => app.reveal_current(&repaint(ctx)),
        Command::Export => export(app, ctx, frame),
        Command::ExportSettings => {
            app.view_state.export_dialog = Some(app.export_settings().clone());
        }
        Command::Copy => app.copy_selection(&repaint(ctx)),
        Command::Step(step) => {
            app.step(step);
        }
        Command::TogglePreview => {
            app.layout.preview_visible = !app.layout.preview_visible;
        }
        Command::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        Command::SelectAll => app.view_mut().select_all(),
        Command::SelectNone => app.view_mut().clear_selection(),
        Command::FindTerms => {
            app.layout.search_panel_visible = true;
            app.view_state.focus_field = Some(FormField::Term(0));
        }
        Command::AddTerm => {
            app.form.add_term();
        }
        Command::Search => {
            let ctx = ctx.clone();
            // L'errore resta nell'app e il modulo lo mostra sotto ogni campo; l'app riapre
            // pannello e Filtri e chiede il focus per il primo campo da correggere.
            if app
                .start_search(Arc::new(move || ctx.request_repaint()))
                .is_ok()
            {
                app.view_state.tab = Tab::Results;
            }
        }
        Command::Cancel => app.cancel_search(),
        Command::ShowResults => app.view_state.tab = Tab::Results,
        Command::ShowWarnings => app.view_state.tab = Tab::Warnings,
        Command::ToggleSearchPanel => {
            app.layout.search_panel_visible = !app.layout.search_panel_visible;
        }
        Command::ToggleFilters => {
            app.layout.search_panel_visible = true;
            app.view_state.open_filters = true;
        }
        Command::NextPane => panes::cycle_focus(ctx, false),
        Command::PreviousPane => panes::cycle_focus(ctx, true),
        Command::About => app.view_state.about_open = true,
    }
}

/// Il dialogo di sistema per scegliere cartelle o file, figlio della finestra e aperto
/// accanto all'ultima radice.
fn dialog(app: &Application, frame: &eframe::Frame, title_id: &str) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new().set_title(app.i18n().tr(title_id));
    if let Some(last) = app.form.roots.last() {
        // Dal tipo già verificato: mai una chiamata al disco sul thread dell'interfaccia.
        let folder = match last.kind {
            RootKind::Folder => Some(last.path.as_path()),
            RootKind::File => last.path.parent(),
            RootKind::Missing | RootKind::Pending => None,
        };
        if let Some(folder) = folder {
            dialog = dialog.set_directory(folder);
        }
    }
    dialog.set_parent(frame)
}
