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

mod commands;
mod menu;
mod search_panel;
mod status;
mod table;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use egui::{Align2, Id, RichText, Ui};

use crate::app::{Application, MAX_HITS_PER_FRAME, Pump, Tab};
use crate::form::RootKind;
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
    ResetFilters,
}

/// Prepara il contesto: font, token del tema, preferenza Sistema/Chiaro/Scuro. Da chiamare
/// una volta alla creazione della finestra (anche nei test dell'interfaccia).
pub fn setup_context(ctx: &egui::Context, app: &Application) {
    crate::platform::install_fonts(ctx);
    theme::apply(ctx, app.theme());
}

impl eframe::App for Application {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.has_root_check_waker() {
            let ctx = ctx.clone();
            self.set_root_check_waker(Arc::new(move || ctx.request_repaint()));
        }
        self.pump_root_checks();
        if self.pump_events(MAX_HITS_PER_FRAME) == Pump::MorePending {
            ctx.request_repaint();
        }
        if self.is_searching() {
            ctx.request_repaint_after(STATUS_REFRESH);
        }
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let mut actions = Vec::new();
        read_shortcuts(self, ui.ctx(), &mut actions);
        read_dropped_files(ui.ctx(), &mut actions);

        egui::Panel::top("menu").show(ui, |ui| menu::bar(self, ui, &mut actions));
        if self.limit_reached() {
            egui::Panel::top("limit_banner").show(ui, |ui| limit_banner(self, ui));
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
        // Fase 6: l'anteprima andrà in un pannello inferiore ridimensionabile dentro
        // quest'area, sotto la tabella.
        egui::CentralPanel::default().show(ui, |ui| central(self, ui, &mut actions));
        about_window(self, ui.ctx());
        drop_hint(self, ui.ctx());

        for action in actions {
            apply(self, ui.ctx(), frame, action);
        }
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        // eframe ricorda la finestra; le impostazioni vanno in settings.toml. Un errore
        // non ferma l'app: riproverà al prossimo salvataggio.
        if let Err(error) = self.save_settings() {
            eprintln!("lynkeus-searcher: impostazioni non salvate: {error}");
        }
    }

    fn on_exit(&mut self) {
        if let Err(error) = self.save_settings() {
            eprintln!("lynkeus-searcher: impostazioni non salvate: {error}");
        }
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }
}

/// Le scorciatoie, lette una volta per frame nell'ordine di [`Command::SHORTCUT_ORDER`].
fn read_shortcuts(app: &Application, ctx: &egui::Context, actions: &mut Vec<Action>) {
    let typing = ctx.egui_wants_keyboard_input();
    for command in Command::SHORTCUT_ORDER {
        let Some(shortcut) = command.shortcut() else {
            continue;
        };
        if typing && command.yields_to_text_fields() {
            continue;
        }
        if command.availability(app).is_err() {
            continue;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
            actions.push(Action::Command(command));
        }
    }
    // Frecce nella tabella, quando nessun campo ha il focus.
    if !typing && app.view_state.tab == Tab::Results && !app.view().is_empty() {
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
        Tab::Results => table::results(app, ui, actions),
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
        Action::ResetFilters => app.form.filters.reset(),
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
            let picked = dialog(app, frame, "dialog-add-folder").pick_folders();
            for path in picked.unwrap_or_default() {
                app.add_root(path);
            }
        }
        Command::AddFile => {
            let picked = dialog(app, frame, "dialog-add-file").pick_files();
            for path in picked.unwrap_or_default() {
                app.add_root(path);
            }
        }
        Command::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        Command::SelectAll => app.view_mut().select_all(),
        Command::SelectNone => app.view_mut().clear_selection(),
        Command::FindTerms => {
            app.layout.search_panel_visible = true;
            app.view_state.focus_first_term = true;
        }
        Command::AddTerm => {
            app.form.add_term();
        }
        Command::Search => {
            let ctx = ctx.clone();
            // L'errore resta nell'app e il modulo lo mostra sotto ogni campo.
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
