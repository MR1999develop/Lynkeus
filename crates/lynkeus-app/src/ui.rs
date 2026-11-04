// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! La finestra: vista umile sopra [`Application`].
//!
//! Provvisorio (fase 0): modulo in alto (cartella, termine, Cerca/Annulla), tabella dei
//! risultati al centro con solo le righe visibili, stato in basso. La fase 3 porta menu,
//! modulo a sinistra, tabella ordinabile, avvisi e `theme.rs`.
//!
//! I gesti diventano [`Action`] raccolte durante il disegno e applicate dopo.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use egui::text::LayoutJob;
use egui::{Align, Color32, FontId, Label, Layout, RichText, TextEdit, TextFormat, TextStyle, Ui};

use crate::app::{Application, MAX_HITS_PER_FRAME, Pump, SearchStatus, StartError};
use crate::format::{self, ELLIPSIS};

/// Ogni quanto si ridisegna lo stato durante una ricerca (tempo trascorso, contatori).
const STATUS_REFRESH: Duration = Duration::from_millis(250);

/// Larghezze delle colonne fisse, in punti.
const WORD_COLUMN_WIDTH: f32 = 160.0;
const FILE_COLUMN_WIDTH: f32 = 220.0;
const POSITION_COLUMN_WIDTH: f32 = 110.0;

/// Un gesto dell'utente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    StartSearch,
    CancelSearch,
}

impl eframe::App for Application {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.pump_events(MAX_HITS_PER_FRAME) == Pump::MorePending {
            ctx.request_repaint();
        }
        if self.is_searching() {
            ctx.request_repaint_after(STATUS_REFRESH);
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let mut actions = Vec::new();
        egui::Panel::top("search_form").show(ui, |ui| search_form(self, ui, &mut actions));
        egui::Panel::bottom("status_bar").show(ui, |ui| status_bar(self, ui));
        egui::CentralPanel::default().show(ui, |ui| results_table(self, ui));
        for action in actions {
            apply(self, ui.ctx(), action);
        }
    }
}

fn apply(app: &mut Application, ctx: &egui::Context, action: Action) {
    match action {
        Action::StartSearch => {
            let ctx = ctx.clone();
            // L'errore resta nell'app e il modulo lo mostra sotto il campo.
            if app
                .start_search(Arc::new(move || ctx.request_repaint()))
                .is_err()
            {}
        }
        Action::CancelSearch => app.cancel_search(),
    }
}

fn search_form(app: &mut Application, ui: &mut Ui, actions: &mut Vec<Action>) {
    ui.add_space(6.0);
    let searching = app.is_searching();
    let i18n = app.i18n().clone();
    let error = app
        .start_error()
        .map(|e| (e, format::start_error(&i18n, e)));
    let (root_error, term_error) = match error {
        Some((StartError::NoRoot, text)) => (Some(text), None),
        Some((_, text)) => (None, Some(text)),
        None => (None, None),
    };
    egui::Grid::new("search_form_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            let label = ui.label(i18n.tr("form-folder-label"));
            ui.add(
                TextEdit::singleline(&mut app.form.root)
                    .hint_text(i18n.tr("form-folder-hint"))
                    .desired_width(480.0),
            )
            .labelled_by(label.id);
            ui.end_row();
            if let Some(text) = root_error {
                ui.label("");
                ui.colored_label(ui.visuals().error_fg_color, text);
                ui.end_row();
            }

            let label = ui.label(i18n.tr("form-term-label"));
            let term = ui
                .add(
                    TextEdit::singleline(&mut app.form.term)
                        .hint_text(i18n.tr("form-term-hint"))
                        .desired_width(480.0),
                )
                .labelled_by(label.id);
            let enter = term.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter && !searching {
                actions.push(Action::StartSearch);
            }
            ui.end_row();
            if let Some(text) = term_error {
                ui.label("");
                ui.colored_label(ui.visuals().error_fg_color, text);
                ui.end_row();
            }

            ui.label("");
            let (text, action) = if searching {
                (i18n.tr("action-cancel"), Action::CancelSearch)
            } else {
                (i18n.tr("action-search"), Action::StartSearch)
            };
            if ui.button(text).clicked() {
                actions.push(action);
            }
            ui.end_row();
        });
    ui.add_space(6.0);
}

fn status_bar(app: &Application, ui: &mut Ui) {
    let i18n = app.i18n();
    let status = app.status();
    ui.horizontal(|ui| {
        if matches!(status, SearchStatus::Running { .. }) {
            ui.spinner();
        }
        ui.label(format::status(i18n, &status, app.limits().max_results));
    });
    if let Some(doc) = app.current_file() {
        ui.add(
            Label::new(i18n.tr_args("status-current-file", &[("file", doc.to_string().into())]))
                .truncate(),
        );
    }
}

fn results_table(app: &Application, ui: &mut Ui) {
    let i18n = app.i18n();
    let results = app.results();
    if results.hits.is_empty() {
        let key = match app.status() {
            SearchStatus::Finished(_) => "results-empty-none",
            _ => "results-empty-idle",
        };
        if !app.is_searching() {
            ui.centered_and_justified(|ui| ui.label(i18n.tr(key)));
        }
        return;
    }

    ui.horizontal(|ui| {
        header(ui, &i18n.tr("column-word"), WORD_COLUMN_WIDTH);
        header(ui, &i18n.tr("column-file"), FILE_COLUMN_WIDTH);
        header(ui, &i18n.tr("column-position"), POSITION_COLUMN_WIDTH);
        ui.label(RichText::new(i18n.tr("column-context")).strong());
    });
    ui.separator();

    let row_height = ui.text_style_height(&TextStyle::Body) + ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical()
        .id_salt(("results", results.generation))
        .auto_shrink(false)
        .show_rows(ui, row_height, results.hits.len(), |ui, rows| {
            for hit in results.hits.get(rows).unwrap_or_default() {
                let root = results.roots.get(hit.root).map(PathBuf::as_path);
                let file =
                    root.map_or_else(|| hit.doc.to_string(), |r| hit.doc.display_relative(r));
                ui.horizontal(|ui| {
                    let mut word = hit.word.clone();
                    if hit.word_truncated {
                        word.push_str(ELLIPSIS);
                    }
                    cell(ui, Label::new(word).truncate(), WORD_COLUMN_WIDTH);
                    cell(ui, Label::new(file).truncate(), FILE_COLUMN_WIDTH)
                        .on_hover_text(hit.doc.to_string());
                    cell(
                        ui,
                        Label::new(format::location(i18n, &hit.location)).truncate(),
                        POSITION_COLUMN_WIDTH,
                    );
                    ui.add(Label::new(context_job(ui, &hit.snippet)).truncate());
                });
            }
        });
}

fn header(ui: &mut Ui, text: &str, width: f32) {
    cell(
        ui,
        Label::new(RichText::new(text).strong()).truncate(),
        width,
    );
}

/// Una cella a larghezza fissa, allineata a sinistra.
fn cell(ui: &mut Ui, label: Label, width: f32) -> egui::Response {
    let height = ui.text_style_height(&TextStyle::Body);
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_width(width);
            ui.add(label)
        },
    )
    .inner
}

/// Il contesto con le corrispondenze evidenziate, e "…" dove la riga continua.
fn context_job(ui: &Ui, snippet: &lynkeus_core::Snippet) -> LayoutJob {
    let font = FontId::default();
    let visuals = ui.visuals();
    let normal = TextFormat::simple(font.clone(), visuals.text_color());
    let highlight = TextFormat {
        background: visuals.selection.bg_fill,
        color: visuals.strong_text_color(),
        ..TextFormat::simple(font, Color32::PLACEHOLDER)
    };
    let mut job = LayoutJob::default();
    if snippet.truncated_start {
        job.append(ELLIPSIS, 0.0, normal.clone());
    }
    let mut cursor = 0;
    for range in &snippet.highlights {
        if let (Some(before), Some(marked)) = (
            snippet.text.get(cursor..range.start),
            snippet.text.get(range.clone()),
        ) {
            job.append(before, 0.0, normal.clone());
            job.append(marked, 0.0, highlight.clone());
            cursor = range.end;
        }
    }
    job.append(
        snippet.text.get(cursor..).unwrap_or_default(),
        0.0,
        normal.clone(),
    );
    if snippet.truncated_end {
        job.append(ELLIPSIS, 0.0, normal);
    }
    job
}
