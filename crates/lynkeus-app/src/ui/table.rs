// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Tabelle del pannello centrale: risultati e avvisi, virtualizzate (solo le righe
//! visibili si disegnano), e lo stato vuoto con il primo passo da fare.
//!
//! Le corrispondenze nel contesto sono evidenziate con un fondo, in SemiBold e
//! sottolineate: il colore non è l'unico segnale.

use std::path::PathBuf;

use egui::text::LayoutJob;
use egui::{Button, Color32, FontId, Label, RichText, Sense, Stroke, TextFormat, Ui};
use egui_extras::{Column, TableBuilder};
use lynkeus_core::Snippet;

use super::{Action, Command};
use crate::app::{Application, EmptyState};
use crate::format::{self, ELLIPSIS};
use crate::results::{ClickKind, SortColumn, SortOrder};
use crate::settings::ColumnWidths;
use crate::theme::{self, SPACE, Tokens};

/// Altezza di una riga della tabella oltre al testo.
const ROW_PADDING: f32 = 6.0;

pub(super) fn results(app: &mut Application, ui: &mut Ui, actions: &mut Vec<Action>) {
    let state = app.empty_state();
    if state != EmptyState::None {
        empty(app, ui, state, actions);
        return;
    }
    let i18n = app.i18n().clone();
    let tokens = theme::current(ui.ctx());
    let results = app.results();
    let with_terms = results.terms.len() > 1;
    let generation = results.generation;
    let widths = app.layout.columns;
    let sort = app.view().sort();
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + ROW_PADDING;
    let scroll_target = app.view_mut().take_scroll_target();

    let mut columns: Vec<(SortColumn, &'static str)> = vec![(SortColumn::Word, "column-word")];
    if with_terms {
        columns.push((SortColumn::Term, "column-term"));
    }
    columns.extend([
        (SortColumn::File, "column-file"),
        (SortColumn::Position, "column-position"),
        (SortColumn::Context, "column-context"),
    ]);

    let resizable = |width: f32| {
        Column::initial(width)
            .range(*ColumnWidths::RANGE.start()..=*ColumnWidths::RANGE.end())
            .clip(true)
    };
    let mut builder = TableBuilder::new(ui)
        .id_salt(("results", generation, with_terms))
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .auto_shrink([false, false])
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(resizable(widths.word));
    if with_terms {
        builder = builder.column(resizable(widths.term));
    }
    builder = builder
        .column(resizable(widths.file))
        .column(resizable(widths.position))
        .column(Column::remainder().clip(true));
    if let Some(position) = scroll_target {
        builder = builder.scroll_to_row(position, None);
    }

    let app_ref: &Application = app;
    let mut new_widths = None;
    builder
        .header(row_height, |mut header| {
            for (column, id) in &columns {
                header.col(|ui| {
                    let mut text = i18n.tr(id);
                    let sorted = sort.filter(|s| s.column == *column);
                    if let Some(spec) = sorted {
                        text.push_str(match spec.order {
                            SortOrder::Ascending => " ↑",
                            SortOrder::Descending => " ↓",
                        });
                    }
                    let button = Button::selectable(
                        sorted.is_some(),
                        RichText::new(text).family(theme::semibold_family()),
                    )
                    .frame(false);
                    if ui
                        .add(button)
                        .on_hover_text(i18n.tr("column-sort-tooltip"))
                        .clicked()
                    {
                        actions.push(Action::Sort(*column));
                    }
                });
            }
        })
        .body(|body| {
            let view = app_ref.view();
            let results = app_ref.results();
            new_widths = Some(body.widths().to_vec());
            body.rows(row_height, view.len(), |mut row| {
                let position = row.index();
                let Some(hit_index) = view.hit_at(position) else {
                    return;
                };
                let Some(hit) = results.hits.get(hit_index) else {
                    return;
                };
                row.set_selected(view.is_selected(hit_index));
                row.col(|ui| {
                    let word = format::word(hit);
                    ui.add(Label::new(word).truncate().selectable(false));
                });
                if with_terms {
                    row.col(|ui| {
                        let terms = format::matched_terms(&results.terms, hit.terms);
                        ui.add(Label::new(terms).truncate().selectable(false));
                    });
                }
                row.col(|ui| {
                    let root = results.roots.get(hit.root).map(PathBuf::as_path);
                    let file =
                        root.map_or_else(|| hit.doc.to_string(), |r| hit.doc.display_relative(r));
                    ui.add(Label::new(file).truncate().selectable(false))
                        .on_hover_text(hit.doc.to_string());
                });
                row.col(|ui| {
                    let location = format::location(&i18n, &hit.location);
                    ui.add(Label::new(location).truncate().selectable(false));
                });
                row.col(|ui| {
                    let job = context_job(&hit.snippet, &tokens);
                    ui.add(Label::new(job).truncate().selectable(false));
                });
                let response = row.response();
                if response.clicked() {
                    let kind = ui_click_kind(&response);
                    actions.push(Action::ClickRow(position, kind));
                }
                if response.double_clicked() {
                    // Il clic ha già portato il cursore su questa riga.
                    actions.push(Action::Command(Command::Open));
                }
                if response.secondary_clicked() && !view.is_selected(hit_index) {
                    actions.push(Action::ClickRow(position, ClickKind::Replace));
                }
                response.context_menu(|ui| row_menu(app_ref, ui, actions));
            });
        });
    if let Some(widths) = new_widths {
        store_widths(&mut app.layout.columns, &widths, with_terms);
    }
}

/// Menu contestuale di una riga: gli stessi comandi dei menu File e Modifica.
///
/// Il clic destro su una riga non selezionata la seleziona e ci porta il cursore, come in
/// Esplora file; su una riga già selezionata lascia la selezione com'è (per copiarla).
fn row_menu(app: &Application, ui: &mut Ui, actions: &mut Vec<Action>) {
    let i18n = app.i18n();
    for command in [Command::Open, Command::Reveal, Command::Copy] {
        let mut button = Button::new(i18n.tr(command.label_id()));
        if let Some(shortcut) = command.shortcut() {
            button = button.shortcut_text(ui.ctx().format_shortcut(&shortcut));
        }
        if ui.add(button).clicked() {
            actions.push(Action::Command(command));
            ui.close();
        }
    }
}

/// Clic, Ctrl+clic o Maiusc+clic, dai modificatori del clic.
fn ui_click_kind(response: &egui::Response) -> ClickKind {
    let modifiers = response.ctx.input(|i| i.modifiers);
    if modifiers.shift {
        ClickKind::Range
    } else if modifiers.command {
        ClickKind::Toggle
    } else {
        ClickKind::Replace
    }
}

/// Ricorda le larghezze delle colonne ridimensionabili (il contesto prende il resto).
fn store_widths(columns: &mut ColumnWidths, widths: &[f32], with_terms: bool) {
    let mut it = widths.iter().copied();
    let mut next = |slot: &mut f32| {
        if let Some(width) = it.next().filter(|w| w.is_finite()) {
            *slot = width.clamp(*ColumnWidths::RANGE.start(), *ColumnWidths::RANGE.end());
        }
    };
    next(&mut columns.word);
    if with_terms {
        next(&mut columns.term);
    }
    next(&mut columns.file);
    next(&mut columns.position);
}

/// Il contesto con le corrispondenze evidenziate, e "…" dove la riga continua.
pub(super) fn context_job(snippet: &Snippet, tokens: &Tokens) -> LayoutJob {
    let normal = TextFormat::simple(FontId::proportional(theme::BODY_SIZE), tokens.text_strong);
    let highlight = TextFormat {
        font_id: FontId::new(theme::BODY_SIZE, theme::semibold_family()),
        color: tokens.text_strong,
        background: tokens.highlight_bg,
        underline: Stroke::new(1.0, tokens.text_strong),
        ..TextFormat::simple(FontId::default(), Color32::PLACEHOLDER)
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

/// Lo stato vuoto: che cosa manca e il comando per farlo.
fn empty(app: &Application, ui: &mut Ui, state: EmptyState, actions: &mut Vec<Action>) {
    let i18n = app.i18n();
    let (title, text) = match state {
        EmptyState::AddRoot => ("empty-add-root-title", "empty-add-root-text"),
        EmptyState::WriteTerm => ("empty-write-term-title", "empty-write-term-text"),
        EmptyState::Searching => ("empty-searching-title", "empty-searching-text"),
        EmptyState::NothingFound | EmptyState::None => ("empty-none-title", "empty-none-text"),
    };
    ui.vertical_centered(|ui| {
        ui.set_max_width(460.0);
        ui.add_space(SPACE[6]);
        ui.label(
            RichText::new(i18n.tr(title))
                .size(theme::FONT[4])
                .family(theme::semibold_family()),
        );
        ui.add_space(SPACE[1]);
        ui.add(Label::new(i18n.tr(text)).wrap());
        ui.add_space(SPACE[3]);
        match state {
            EmptyState::AddRoot => {
                ui.horizontal(|ui| {
                    if ui.button(i18n.tr("form-add-folder")).clicked() {
                        actions.push(Action::Command(Command::AddFolder));
                    }
                    if ui.button(i18n.tr("form-add-file")).clicked() {
                        actions.push(Action::Command(Command::AddFile));
                    }
                });
            }
            EmptyState::WriteTerm => {
                if ui.button(i18n.tr("menu-find-terms")).clicked() {
                    actions.push(Action::Command(Command::FindTerms));
                }
            }
            EmptyState::Searching | EmptyState::NothingFound | EmptyState::None => {}
        }
    });
}

pub(super) fn warnings(app: &Application, ui: &mut Ui) {
    let i18n = app.i18n();
    let results = app.results();
    if results.warnings.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(SPACE[6]);
            ui.label(i18n.tr("warnings-empty"));
        });
        return;
    }
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + ROW_PADDING;
    TableBuilder::new(ui)
        .id_salt(("warnings", results.generation))
        .striped(true)
        .resizable(true)
        .auto_shrink([false, false])
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(420.0).range(80.0..=2000.0).clip(true))
        .column(Column::remainder().clip(true))
        .header(row_height, |mut header| {
            for id in ["column-file", "column-reason"] {
                header.col(|ui| {
                    ui.label(RichText::new(i18n.tr(id)).family(theme::semibold_family()));
                });
            }
        })
        .body(|body| {
            body.rows(row_height, results.warnings.len(), |mut row| {
                let Some(warning) = results.warnings.get(row.index()) else {
                    return;
                };
                row.col(|ui| {
                    let full = warning.doc.to_string();
                    ui.add(Label::new(full.clone()).truncate())
                        .on_hover_text(full);
                });
                row.col(|ui| {
                    ui.add(Label::new(i18n.tr(format::warning_id(warning.kind))).truncate());
                });
            });
        });
}
