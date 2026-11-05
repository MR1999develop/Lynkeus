// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Tabelle del pannello centrale: risultati e avvisi, virtualizzate (solo le righe
//! visibili si disegnano), e lo stato vuoto con il primo passo da fare.
//!
//! Le corrispondenze nel contesto sono evidenziate con un fondo, in SemiBold e
//! sottolineate: il colore non è l'unico segnale. Le righe selezionate hanno, oltre al
//! fondo, un segno sul bordo sinistro.
//!
//! Ogni riga è un nodo per la tastiera e i lettori di schermo (ruolo "riga", stato
//! "selezionata", nome con parola, file e posizione); le celle non prendono il focus, così
//! il Tab attraversa la tabella in un passo. La riga del cursore prende il focus dopo un
//! clic o una freccia, con un bordo di 2 px; le frecce, da lì, muovono il cursore.

use std::path::PathBuf;

use egui::text::LayoutJob;
use egui::{
    Button, Color32, EventFilter, FontId, Id, Label, Painter, Rect, RichText, Role, Sense, Stroke,
    StrokeKind, TextFormat, Ui, WidgetInfo, vec2,
};
use egui_extras::{Column, TableBuilder};
use lynkeus_core::Snippet;

use super::panes::{self, Pane};
use super::{Action, Command};
use crate::app::{Application, EmptyState};
use crate::format::{self, ELLIPSIS};
use crate::results::{ClickKind, SortColumn, SortOrder};
use crate::settings::ColumnWidths;
use crate::theme::{self, SELECTION_MARK_WIDTH, SPACE, Tokens};

/// Altezza di una riga della tabella oltre al testo.
const ROW_PADDING: f32 = 6.0;

/// Il nodo per la tastiera e i lettori di schermo della riga del risultato `hit` della
/// ricerca numero `generation`.
fn row_id(generation: u64, hit: usize) -> Id {
    Id::unique(("lynkeus-result-row", generation, hit))
}

/// Dove si ricorda se la riga del cursore era disegnata nel frame precedente.
fn cursor_drawn_id() -> Id {
    Id::unique("lynkeus-cursor-row-drawn")
}

/// Il nodo della riga con il cursore, se c'è.
pub(super) fn cursor_row_id(app: &Application) -> Option<Id> {
    let hit = app.view().cursor_hit()?;
    Some(row_id(app.results().generation, hit))
}

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
    let focus_request = app.view().focus_request();
    let pane_rect = ui.max_rect();
    // Una sola riga è nell'ordine del Tab: quella del cursore se era disegnata nel frame
    // precedente, altrimenti la prima disegnata. F6 porta il focus lì.
    let cursor = app.view().cursor_hit();
    let cursor_was_drawn = ui
        .data(|data| data.get_temp::<bool>(cursor_drawn_id()))
        .unwrap_or(false);

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
    // Le celle sentono i clic ma non prendono il focus: lo prende la riga.
    let mut builder = TableBuilder::new(ui)
        .id_salt(("results", generation, with_terms))
        .striped(true)
        .resizable(true)
        .sense(Sense::CLICK)
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

    let mut cursor_drawn = false;
    let mut any_drawn = false;
    let app_ref: &Application = app;
    let mut new_widths = None;
    let mut focus_taken = false;
    let mut pane_target = None;
    builder
        .header(row_height, |mut header| {
            for (index, (column, id)) in columns.iter().enumerate() {
                header.col(|ui| {
                    if index == 0 {
                        // Allineata al testo delle righe, che lascia posto al segno della
                        // selezione.
                        ui.add_space(SELECTION_MARK_WIDTH + SPACE[0]);
                    }
                    let mut text = i18n.tr(id);
                    let sorted = sort.filter(|s| s.column == *column);
                    if let Some(spec) = sorted {
                        text.push_str(match spec.order {
                            SortOrder::Ascending => " ↑",
                            SortOrder::Descending => " ↓",
                        });
                    }
                    // Senza cornice a riposo, con la cornice (e il bordo di focus) quando
                    // il puntatore o la tastiera ci arrivano.
                    let button = Button::selectable(
                        sorted.is_some(),
                        RichText::new(text).family(theme::semibold_family()),
                    )
                    .frame_when_inactive(false);
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
                let selected = view.is_selected(hit_index);
                row.set_selected(selected);
                let (_, first_cell) = row.col(|ui| {
                    if selected {
                        paint_selection_mark(ui, &tokens);
                    }
                    ui.add_space(SELECTION_MARK_WIDTH + SPACE[0]);
                    let word = format::word(hit);
                    ui.add(Label::new(word).truncate().selectable(false));
                });
                let first_clip = Rect::from_x_y_ranges(
                    first_cell.rect.x_range(),
                    first_cell.interact_rect.y_range(),
                );
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
                    // Il nodo della riga, largo quanto la riga: focus, ruolo, selezione.
                    let rect = first_cell.rect.union(ui.max_rect());
                    let id = row_id(generation, hit_index);
                    let first_drawn = !std::mem::replace(&mut any_drawn, true);
                    let tab_stop = if cursor_was_drawn {
                        cursor == Some(hit_index)
                    } else {
                        first_drawn
                    };
                    let sense = if tab_stop || focus_request == Some(hit_index) {
                        Sense::FOCUSABLE
                    } else {
                        Sense::hover()
                    };
                    let node = ui.interact(rect, id, sense);
                    let name = row_name(&i18n, results, hit);
                    node.widget_info(|| WidgetInfo::labeled(Role::Row, true, &name));
                    ui.ctx()
                        .accesskit_node_builder(id, |builder| builder.set_selected(selected));
                    if focus_request == Some(hit_index) {
                        node.request_focus();
                        focus_taken = true;
                    }
                    if node.has_focus() {
                        // Le frecce muovono il cursore della tabella, non il focus di egui.
                        let arrows = EventFilter {
                            vertical_arrows: true,
                            ..EventFilter::default()
                        };
                        ui.memory_mut(|memory| memory.set_focus_lock_filter(id, arrows));
                        let clip = rect
                            .intersect(Rect::from_x_y_ranges(rect.x_range(), first_clip.y_range()));
                        Painter::new(ui.ctx().clone(), ui.layer_id(), clip).rect_stroke(
                            rect,
                            0.0,
                            theme::focus_stroke(&tokens),
                            StrokeKind::Inside,
                        );
                    }
                    // Il focus arrivato da Tab o F6 porta lì anche il cursore: chi apre o
                    // copia agisce sulla riga con il focus.
                    if node.gained_focus() && cursor != Some(hit_index) {
                        actions.push(Action::ClickRow(position, ClickKind::Replace));
                    }
                    if tab_stop {
                        pane_target = Some(id);
                    }
                    cursor_drawn |= cursor == Some(hit_index);
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
    ui.data_mut(|data| data.insert_temp(cursor_drawn_id(), cursor_drawn));
    if focus_taken {
        app.view_mut().focus_done();
    }
    if let Some(target) = pane_target {
        panes::register(ui.ctx(), Pane::Results, pane_rect, target);
    }
}

/// Il segno della selezione sul bordo sinistro della cella: la riga selezionata si
/// riconosce anche senza il colore del fondo.
fn paint_selection_mark(ui: &Ui, tokens: &Tokens) {
    let cell = ui.max_rect();
    let mark = Rect::from_min_size(cell.left_top(), vec2(SELECTION_MARK_WIDTH, cell.height()));
    ui.painter().rect_filled(mark, 0.0, tokens.selection_mark);
}

/// Il nome della riga per i lettori di schermo: parola, file e posizione.
fn row_name(
    i18n: &crate::i18n::I18n,
    results: &crate::app::Results,
    hit: &lynkeus_engine::Hit,
) -> String {
    let root = results.roots.get(hit.root).map(PathBuf::as_path);
    let file = root.map_or_else(|| hit.doc.to_string(), |r| hit.doc.display_relative(r));
    i18n.tr_args(
        "table-row-name",
        &[
            ("word", format::word(hit).into()),
            ("file", file.into()),
            ("location", format::location(i18n, &hit.location).into()),
        ],
    )
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
        EmptyState::Cancelled => ("empty-cancelled-title", "empty-cancelled-text"),
        EmptyState::Interrupted => ("empty-interrupted-title", "empty-interrupted-text"),
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
            EmptyState::Cancelled | EmptyState::Interrupted => {
                if ui.button(i18n.tr("empty-search-again")).clicked() {
                    actions.push(Action::Command(Command::Search));
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
