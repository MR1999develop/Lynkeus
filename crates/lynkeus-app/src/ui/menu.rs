// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Barra dei menu: File, Modifica, Ricerca, Visualizza, Strumenti, Aiuto. Ogni comando
//! ha una voce; le voci disabilitate dicono perché nel tooltip.

use egui::{Button, Ui};

use super::{Action, Command};
use crate::app::{Application, Tab};
use crate::i18n::Language;
use crate::results::Step;
use crate::theme::ThemeChoice;

pub(super) fn bar(app: &Application, ui: &mut Ui, actions: &mut Vec<Action>) {
    let i18n = app.i18n().clone();
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button(i18n.tr("menu-file"), |ui| {
            item(app, ui, Command::AddFolder, actions);
            item(app, ui, Command::AddFile, actions);
            recent_roots(app, ui, actions);
            ui.separator();
            item(app, ui, Command::Open, actions);
            item(app, ui, Command::Reveal, actions);
            ui.separator();
            item(app, ui, Command::Export, actions);
            item(app, ui, Command::ExportSettings, actions);
            ui.separator();
            item(app, ui, Command::Quit, actions);
        });
        ui.menu_button(i18n.tr("menu-edit"), |ui| {
            item(app, ui, Command::Copy, actions);
            ui.separator();
            item(app, ui, Command::SelectAll, actions);
            item(app, ui, Command::SelectNone, actions);
        });
        ui.menu_button(i18n.tr("menu-search-menu"), |ui| {
            item(app, ui, Command::Search, actions);
            item(app, ui, Command::Cancel, actions);
            ui.separator();
            item(app, ui, Command::FindTerms, actions);
            item(app, ui, Command::AddTerm, actions);
            recent_terms(app, ui, actions);
            ui.separator();
            item(app, ui, Command::ToggleFilters, actions);
            ui.separator();
            for step in [
                Step::NextInFile,
                Step::PreviousInFile,
                Step::NextFile,
                Step::PreviousFile,
            ] {
                item(app, ui, Command::Step(step), actions);
            }
        });
        ui.menu_button(i18n.tr("menu-view"), |ui| {
            check_item(
                app,
                ui,
                Command::ShowResults,
                app.view_state.tab == Tab::Results,
                actions,
            );
            check_item(
                app,
                ui,
                Command::ShowWarnings,
                app.view_state.tab == Tab::Warnings,
                actions,
            );
            ui.separator();
            check_item(
                app,
                ui,
                Command::ToggleSearchPanel,
                app.layout.search_panel_visible,
                actions,
            );
            check_item(
                app,
                ui,
                Command::TogglePreview,
                app.layout.preview_visible,
                actions,
            );
            ui.separator();
            ui.menu_button(i18n.tr("menu-theme"), |ui| {
                for choice in ThemeChoice::ALL {
                    let id = match choice {
                        ThemeChoice::System => "theme-system",
                        ThemeChoice::Light => "theme-light",
                        ThemeChoice::Dark => "theme-dark",
                    };
                    if ui.radio(app.theme() == choice, i18n.tr(id)).clicked() {
                        actions.push(Action::SetTheme(choice));
                        ui.close();
                    }
                }
            });
        });
        ui.menu_button(i18n.tr("menu-tools"), |ui| {
            ui.menu_button(i18n.tr("menu-language"), |ui| {
                let choices = [
                    (None, "language-system"),
                    (Some(Language::Italian), "language-italian"),
                    (Some(Language::English), "language-english"),
                ];
                for (language, id) in choices {
                    if ui
                        .radio(app.language_choice() == language, i18n.tr(id))
                        .clicked()
                    {
                        actions.push(Action::SetLanguage(language));
                        ui.close();
                    }
                }
            });
        });
        ui.menu_button(i18n.tr("menu-help"), |ui| {
            item(app, ui, Command::About, actions);
        });
    });
}

/// Una voce di comando, con la scorciatoia accanto e il motivo se disabilitata.
fn item(app: &Application, ui: &mut Ui, command: Command, actions: &mut Vec<Action>) {
    let i18n = app.i18n();
    let mut button = Button::new(i18n.tr(command.label_id()));
    if let Some(shortcut) = command.shortcut() {
        button = button.shortcut_text(ui.ctx().format_shortcut(&shortcut));
    }
    let availability = command.availability(app);
    let mut response = ui.add_enabled(availability.is_ok(), button);
    if let Err(reason) = availability {
        response = response.on_disabled_hover_text(i18n.tr(reason));
    }
    if response.clicked() {
        actions.push(Action::Command(command));
        ui.close();
    }
}

/// Una voce che attiva o disattiva qualcosa, con la spunta.
fn check_item(
    app: &Application,
    ui: &mut Ui,
    command: Command,
    checked: bool,
    actions: &mut Vec<Action>,
) {
    let i18n = app.i18n();
    let mut button = Button::selectable(checked, i18n.tr(command.label_id()));
    if let Some(shortcut) = command.shortcut() {
        button = button.shortcut_text(ui.ctx().format_shortcut(&shortcut));
    }
    if ui.add(button).clicked() {
        actions.push(Action::Command(command));
        ui.close();
    }
}

fn recent_roots(app: &Application, ui: &mut Ui, actions: &mut Vec<Action>) {
    let i18n = app.i18n();
    let recent = app.recent_roots();
    ui.add_enabled_ui(!recent.is_empty(), |ui| {
        ui.menu_button(i18n.tr("menu-recent-roots"), |ui| {
            for path in recent {
                if ui.button(path.display().to_string()).clicked() {
                    actions.push(Action::AddRoots(vec![path.clone()]));
                    ui.close();
                }
            }
        })
        .response
        .on_disabled_hover_text(i18n.tr("reason-no-recent"));
    });
}

fn recent_terms(app: &Application, ui: &mut Ui, actions: &mut Vec<Action>) {
    let i18n = app.i18n();
    let recent = app.recent_terms();
    ui.add_enabled_ui(!recent.is_empty(), |ui| {
        ui.menu_button(i18n.tr("menu-recent-terms"), |ui| {
            for term in recent {
                if ui.button(term.as_str()).clicked() {
                    actions.push(Action::InsertTerm(term.clone()));
                    ui.close();
                }
            }
        })
        .response
        .on_disabled_hover_text(i18n.tr("reason-no-recent"));
    });
}
