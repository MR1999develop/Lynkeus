// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Pannello di ricerca a sinistra: dove cercare, termini, opzioni, filtri e il bottone
//! primario Cerca (che diventa Annulla), fisso in fondo al pannello.
//!
//! Gli errori di un campo compaiono sotto il campo dopo la prima uscita dal campo o il
//! primo tentativo di ricerca, poi si aggiornano a ogni modifica. Quando la ricerca non
//! parte, il primo campo da correggere riceve il focus e va in vista.
//!
//! L'ordine del Tab è quello di lettura: luoghi, termini, opzioni, filtri, poi Cerca (il
//! bottone si disegna per ultimo, nello spazio riservato in fondo). I campi hanno un bordo
//! di focus di 2 px; il primario, con il fondo d'accento, ha l'anello all'esterno.

use egui::{
    Button, CollapsingHeader, CornerRadius, DragValue, Id, Label, Response, RichText, Stroke,
    StrokeKind, TextEdit, Ui, vec2,
};

use super::panes::{self, Pane};
use super::{Action, Command};
use crate::app::Application;
use crate::form::{FormField, PathInputStatus, RootKind, TermRowError};
use crate::format;
use crate::i18n::I18n;
use crate::settings::{ExtensionMode, MAX_FILE_SIZE_MIB_RANGE};
use crate::theme::{self, FOCUS_GAP, RADIUS, SPACE};

/// Larghezza dei bottoni quadrati "×" accanto alle righe.
const ICON_BUTTON_WIDTH: f32 = 28.0;
/// Altezza del bottone primario.
const PRIMARY_HEIGHT: f32 = 32.0;

pub(super) fn show(app: &mut Application, ui: &mut Ui, actions: &mut Vec<Action>) {
    let tokens = theme::current(ui.ctx());
    // I campi di testo disegnano il focus con il tratto della selezione.
    ui.visuals_mut().selection.stroke = theme::focus_stroke(&tokens);
    let panel = ui.max_rect();
    let focus = app.view_state.focus_field.take();
    // Lo spazio del bottone primario in fondo; il modulo scorre in quello che resta.
    let footer = PRIMARY_HEIGHT + 2.0 * SPACE[1] + 3.0 * ui.spacing().item_spacing.y;
    let form_height = (ui.available_height() - footer).max(0.0);
    let mut first_term = None;
    egui::ScrollArea::vertical()
        .id_salt("search_form")
        .auto_shrink([false, false])
        .max_height(form_height)
        .show(ui, |ui| {
            ui.add_space(SPACE[1]);
            roots_section(app, ui, focus, actions);
            ui.add_space(SPACE[4]);
            first_term = terms_section(app, ui, focus, actions);
            ui.add_space(SPACE[4]);
            options_section(app, ui, actions);
            ui.add_space(SPACE[3]);
            filters_section(app, ui, focus, actions);
            ui.add_space(SPACE[2]);
        });
    primary_button(app, ui, actions);
    if let Some(target) = first_term {
        panes::register(ui.ctx(), Pane::Search, panel, target);
    }
}

/// Se `field` è il campo richiesto, gli dà il focus e lo porta in vista.
fn focus_if_requested(response: &Response, field: FormField, requested: Option<FormField>) {
    if requested == Some(field) {
        response.request_focus();
        response.scroll_to_me(None);
    }
}

fn section_title(ui: &mut Ui, text: String) -> egui::Response {
    ui.label(
        RichText::new(text)
            .size(theme::FONT[3])
            .family(theme::semibold_family()),
    )
}

fn error_text(ui: &mut Ui, text: String) {
    let tokens = theme::current(ui.ctx());
    ui.add(Label::new(
        RichText::new(text)
            .size(theme::CAPTION_SIZE)
            .color(tokens.error),
    ));
}

fn caption(ui: &mut Ui, text: String) {
    ui.add(Label::new(
        RichText::new(text).size(theme::CAPTION_SIZE).weak(),
    ));
}

fn roots_section(
    app: &mut Application,
    ui: &mut Ui,
    focus: Option<FormField>,
    actions: &mut Vec<Action>,
) {
    let i18n = app.i18n().clone();
    section_title(ui, i18n.tr("form-roots-title"));
    if app.form.roots.is_empty() {
        caption(ui, i18n.tr("form-roots-empty"));
        if app.form.show_roots_error {
            error_text(ui, i18n.tr("error-no-folder"));
        }
    }
    for (index, root) in app.form.roots.iter().enumerate() {
        ui.horizontal(|ui| {
            let full = root.path.display().to_string();
            let remove = ui
                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let remove = ui
                        .add_sized(
                            vec2(ICON_BUTTON_WIDTH, theme::INTERACT_HEIGHT),
                            Button::new("×"),
                        )
                        .on_hover_text(i18n.tr("form-root-remove-tooltip"))
                        .accessible_name(
                            i18n.tr_args("form-root-remove-name", &[("path", full.clone().into())]),
                        );
                    let kind = match root.kind {
                        RootKind::Folder => "root-kind-folder",
                        RootKind::File => "root-kind-file",
                        RootKind::Missing => "root-kind-missing",
                        RootKind::Pending => "root-kind-pending",
                    };
                    caption(ui, i18n.tr(kind));
                    ui.add(Label::new(short_name(&root.path)).truncate())
                        .on_hover_text(full);
                    remove
                })
                .inner;
            if remove.clicked() {
                actions.push(Action::RemoveRoot(index));
            }
        });
    }
    ui.horizontal_wrapped(|ui| {
        let add_folder = ui.button(i18n.tr("form-add-folder"));
        focus_if_requested(&add_folder, FormField::Roots, focus);
        if add_folder.clicked() {
            actions.push(Action::Command(Command::AddFolder));
        }
        if ui.button(i18n.tr("form-add-file")).clicked() {
            actions.push(Action::Command(Command::AddFile));
        }
    });
    ui.add_space(SPACE[0]);
    let label = ui.label(i18n.tr("form-path-label"));
    let field = ui
        .add(
            TextEdit::singleline(&mut app.form.path_input)
                .hint_text(i18n.tr("form-path-hint"))
                .desired_width(f32::INFINITY),
        )
        .labelled_by(label.id);
    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        actions.push(Action::AddTypedPath);
    }
    if field.changed() {
        app.form.path_status = PathInputStatus::Idle;
    }
    match &app.form.path_status {
        PathInputStatus::Idle => {}
        PathInputStatus::Checking(_) => caption(ui, i18n.tr("form-path-checking")),
        PathInputStatus::Error(error) => error_text(ui, format::path_error(&i18n, *error)),
    }
}

/// Nome breve di una radice: l'ultimo componente, o il percorso intero (es. "C:\").
fn short_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// I termini, una riga ciascuno; restituisce il campo della prima riga (la zona del
/// pannello per F6).
fn terms_section(
    app: &mut Application,
    ui: &mut Ui,
    focus: Option<FormField>,
    actions: &mut Vec<Action>,
) -> Option<Id> {
    let i18n = app.i18n().clone();
    section_title(ui, i18n.tr("form-terms-title"));
    let row_count = app.form.terms().len();
    let mut first_field = None;
    for index in 0..row_count {
        let number = index + 1;
        let Some(row_id) = app.form.terms().get(index).map(crate::form::TermRow::id) else {
            continue;
        };
        let removable = row_count > 1
            || app
                .form
                .terms()
                .first()
                .is_some_and(|row| !row.text.is_empty());
        let (field, remove) = ui
            .horizontal(|ui| {
                let width = (ui.available_width() - ICON_BUTTON_WIDTH - SPACE[1]).max(80.0);
                let hint = if index == 0 {
                    i18n.tr("form-term-hint")
                } else {
                    i18n.tr("form-term-hint-more")
                };
                let Some(text) = app.form.term_text_mut(index) else {
                    return (None, false);
                };
                let field = ui
                    .add(
                        TextEdit::singleline(text)
                            .id_salt(("term", row_id))
                            .hint_text(hint)
                            .desired_width(width),
                    )
                    .accessible_name(i18n.tr_args("form-term-name", &[("number", number.into())]));
                let remove = ui
                    .add_enabled(
                        removable,
                        Button::new("×").min_size(vec2(ICON_BUTTON_WIDTH, theme::INTERACT_HEIGHT)),
                    )
                    .on_hover_text(i18n.tr("form-term-remove-tooltip"))
                    .accessible_name(
                        i18n.tr_args("form-term-remove-name", &[("number", number.into())]),
                    )
                    .clicked();
                (Some(field), remove)
            })
            .inner;
        if let Some(field) = field {
            focus_if_requested(&field, FormField::Term(index), focus);
            if index == 0 {
                first_field = Some(field.id);
            }
            if field.changed() {
                actions.push(Action::TermsEdited);
            }
            if field.lost_focus() {
                app.form.reveal_term_error(index);
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !app.is_searching() {
                    actions.push(Action::Command(Command::Search));
                }
            }
        }
        if remove {
            actions.push(Action::RemoveTerm(index));
        }
        if let Some(message) = term_message(app, &i18n, index) {
            error_text(ui, message);
        }
    }
    let availability = Command::AddTerm.availability(app);
    let mut add = ui.add_enabled(availability.is_ok(), Button::new(i18n.tr("form-add-term")));
    if let Err(reason) = availability {
        add = add.on_disabled_hover_text(i18n.tr(reason));
    }
    if add.clicked() {
        actions.push(Action::Command(Command::AddTerm));
    }
    if let Some(crate::app::StartError::Form(errors)) = app.start_error()
        && let Some(error) = errors.query
    {
        error_text(ui, format::query_error(&i18n, error));
    }
    first_field
}

/// Il messaggio sotto la riga `index`, se va mostrato.
fn term_message(app: &Application, i18n: &I18n, index: usize) -> Option<String> {
    let row = app.form.terms().get(index)?;
    if row.show_error
        && let Some(error) = app.form.term_error(index)
    {
        return Some(format::term_error(i18n, TermRowError::Invalid(error)));
    }
    app.term_compile_error(index)
        .then(|| format::term_error(i18n, TermRowError::TooComplex))
}

fn options_section(app: &mut Application, ui: &mut Ui, actions: &mut Vec<Action>) {
    let i18n = app.i18n().clone();
    section_title(ui, i18n.tr("form-options-title"));
    let form = &mut app.form;
    let mut changed = ui
        .checkbox(&mut form.case_sensitive, i18n.tr("option-case"))
        .on_hover_text(i18n.tr("option-case-tooltip"))
        .changed();
    changed |= ui
        .checkbox(&mut form.ignore_accents, i18n.tr("option-accents"))
        .on_hover_text(i18n.tr("option-accents-tooltip"))
        .changed();
    changed |= ui
        .checkbox(&mut form.wildcards, i18n.tr("option-wildcards"))
        .on_hover_text(i18n.tr("option-wildcards-tooltip"))
        .changed();
    if changed {
        actions.push(Action::TermsEdited);
    }
    ui.add_space(SPACE[1]);
    let enabled = form.combine_enabled();
    let reason = i18n.tr("reason-one-term");
    ui.add_enabled_ui(enabled, |ui| {
        ui.label(i18n.tr("combine-title"))
            .on_disabled_hover_text(reason.clone());
        ui.radio_value(
            &mut form.combine,
            lynkeus_core::Combine::Any,
            i18n.tr("combine-any"),
        )
        .on_hover_text(i18n.tr("combine-any-tooltip"))
        .on_disabled_hover_text(reason.clone());
        ui.radio_value(
            &mut form.combine,
            lynkeus_core::Combine::All,
            i18n.tr("combine-all"),
        )
        .on_hover_text(i18n.tr("combine-all-tooltip"))
        .on_disabled_hover_text(reason);
    });
}

fn filters_section(
    app: &mut Application,
    ui: &mut Ui,
    focus: Option<FormField>,
    actions: &mut Vec<Action>,
) {
    let i18n = app.i18n().clone();
    let open = std::mem::take(&mut app.view_state.open_filters).then_some(true);
    let filters = &mut app.form.filters;
    let title = if filters.is_customized() {
        i18n.tr("filters-title-customized")
    } else {
        i18n.tr("filters-title")
    };
    CollapsingHeader::new(RichText::new(title).family(theme::semibold_family()))
        .id_salt("filters")
        .default_open(false)
        .open(open)
        .show(ui, |ui| {
            let label = ui.label(i18n.tr("filters-extensions"));
            ui.horizontal_wrapped(|ui| {
                ui.radio_value(
                    &mut filters.extension_mode,
                    ExtensionMode::All,
                    i18n.tr("filters-extensions-all"),
                );
                ui.radio_value(
                    &mut filters.extension_mode,
                    ExtensionMode::Include,
                    i18n.tr("filters-extensions-include"),
                );
                ui.radio_value(
                    &mut filters.extension_mode,
                    ExtensionMode::Exclude,
                    i18n.tr("filters-extensions-exclude"),
                );
            });
            let active = filters.extension_mode != ExtensionMode::All;
            let field = ui
                .add_enabled(
                    active,
                    TextEdit::singleline(&mut filters.extensions)
                        .hint_text(i18n.tr("filters-extensions-hint"))
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(label.id);
            focus_if_requested(&field, FormField::Extensions, focus);
            if field.lost_focus() {
                filters.show_extension_error = true;
            }
            if filters.show_extension_error
                && let Some(error) = filters.extension_error()
            {
                error_text(ui, format::extension_error(&i18n, &error));
            }

            ui.add_space(SPACE[1]);
            let label = ui.label(i18n.tr("filters-skip-dirs"));
            ui.add(
                TextEdit::multiline(&mut filters.skip_dirs)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(label.id);
            caption(ui, i18n.tr("filters-skip-dirs-caption"));

            ui.add_space(SPACE[1]);
            ui.checkbox(&mut filters.include_hidden, i18n.tr("filters-hidden"));
            ui.checkbox(&mut filters.include_online_only, i18n.tr("filters-online"))
                .on_hover_text(i18n.tr("filters-online-tooltip"));
            ui.horizontal(|ui| {
                let limit = ui.checkbox(&mut filters.limit_file_size, i18n.tr("filters-max-size"));
                ui.add_enabled(
                    filters.limit_file_size,
                    DragValue::new(&mut filters.max_file_size_mib)
                        .range(MAX_FILE_SIZE_MIB_RANGE)
                        .suffix(i18n.tr("filters-max-size-unit")),
                )
                .labelled_by(limit.id);
            });
            ui.add_space(SPACE[1]);
            let customized = filters.is_customized();
            if ui
                .add_enabled(customized, Button::new(i18n.tr("filters-reset")))
                .on_disabled_hover_text(i18n.tr("reason-filters-default"))
                .clicked()
            {
                actions.push(Action::ResetFilters);
            }
        });
}

fn primary_button(app: &Application, ui: &mut Ui, actions: &mut Vec<Action>) {
    let i18n = app.i18n();
    let tokens = theme::current(ui.ctx());
    let (label, command) = if app.is_searching() {
        (i18n.tr("action-cancel"), Command::Cancel)
    } else {
        (i18n.tr("action-search"), Command::Search)
    };
    ui.add_space(SPACE[1]);
    // Lascia posto all'anello di focus esterno, ai lati.
    let width = ui.available_width() - 2.0 * (FOCUS_GAP + theme::FOCUS_WIDTH);
    let button = Button::new(
        RichText::new(label)
            .color(tokens.on_accent)
            .family(theme::semibold_family()),
    )
    .fill(tokens.accent)
    .min_size(vec2(width, PRIMARY_HEIGHT));
    let shortcut = command
        .shortcut()
        .map(|s| ui.ctx().format_shortcut(&s))
        .unwrap_or_default();
    let response = ui
        .horizontal(|ui| {
            ui.add_space(FOCUS_GAP + theme::FOCUS_WIDTH);
            // Sul fondo d'accento il bordo resta dell'accento: il focus è l'anello fuori.
            let widgets = &mut ui.visuals_mut().widgets;
            widgets.active.bg_stroke = Stroke::new(theme::FOCUS_WIDTH, tokens.accent);
            ui.add(button)
        })
        .inner;
    if response.has_focus() {
        let radius = f32::from(RADIUS[0]) + FOCUS_GAP;
        ui.painter().rect_stroke(
            response.rect.expand(FOCUS_GAP),
            CornerRadius::from(radius),
            theme::focus_stroke(&tokens),
            StrokeKind::Outside,
        );
    }
    if response.on_hover_text(shortcut).clicked() {
        actions.push(Action::Command(command));
    }
    ui.add_space(SPACE[1]);
}
