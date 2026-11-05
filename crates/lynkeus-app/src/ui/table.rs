// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Tabelle del pannello centrale: risultati e avvisi, virtualizzate (solo le righe
//! visibili si disegnano), e lo stato vuoto con il primo passo da fare.
//!
//! Le corrispondenze nel contesto sono evidenziate con un fondo, in SemiBold e
//! sottolineate: il colore non è l'unico segnale. Le righe selezionate hanno, oltre al
//! fondo, un segno sul bordo sinistro.
//!
//! Ogni riga è un nodo per il mouse, la tastiera e i lettori di schermo, con l'id del
//! risultato e non della posizione: con un ordinamento scelto le righe nuove entrano al
//! loro posto durante la ricerca, e un clic (anche doppio) agisce sul risultato che era
//! sotto il puntatore, non su quello che ora occupa quella posizione. Il nodo nasce prima
//! del contenuto delle celle, così le etichette sopra di lui hanno ancora i loro tooltip;
//! le celle non sentono i clic.
//!
//! Per i lettori di schermo la tabella è una griglia a selezione multipla con il numero di
//! righe; ogni riga ne è figlia, con ruolo "riga", stato "selezionata", posizione e nome
//! (parola, file e posizione nel file). Le celle non prendono il focus, così il Tab
//! attraversa la tabella in un passo. La riga del cursore prende il focus dopo un clic o
//! una freccia, con un bordo di 2 px; le frecce, da lì, muovono il cursore.
//!
//! Mentre arrivano righe ordinate, la tabella scorre di quante righe sono entrate sopra la
//! riga da tenere ferma ([`crate::results::ResultsView::take_scroll_shift`]): ciò che
//! l'utente guarda o sta per cliccare non si sposta.

use std::path::PathBuf;

use egui::text::LayoutJob;
use egui::{
    Button, Color32, EventFilter, FontId, Id, Label, Painter, Rangef, Rect, Response, RichText,
    Role, Sense, Stroke, StrokeKind, TextFormat, Ui, UiBuilder, WidgetInfo, vec2,
};
use egui_extras::{Column, TableBuilder, TableRow};
use lynkeus_core::Snippet;
use lynkeus_engine::Hit;

use super::panes::{self, Pane};
use super::{Action, Command};
use crate::app::{Application, EmptyState, Results};
use crate::format::{self, ELLIPSIS};
use crate::i18n::I18n;
use crate::results::{ClickKind, ResultsView, SortColumn, SortOrder, SortSpec};
use crate::settings::ColumnWidths;
use crate::theme::{self, SELECTION_MARK_WIDTH, SPACE, Tokens};

/// Altezza di una riga della tabella oltre al testo.
const ROW_PADDING: f32 = 6.0;

/// Il nodo per il mouse, la tastiera e i lettori di schermo della riga del risultato `hit`
/// della ricerca numero `generation`.
fn row_id(generation: u64, hit: usize) -> Id {
    Id::unique(("lynkeus-result-row", generation, hit))
}

/// Dove si ricorda, da un frame all'altro, che cosa avevano le righe disegnate.
fn rows_memory_id() -> Id {
    Id::unique("lynkeus-result-rows-memory")
}

/// Il contenitore "griglia" delle righe, per i lettori di schermo.
fn grid_id() -> Id {
    Id::unique("lynkeus-results-grid")
}

/// Il nodo della riga con il cursore, se c'è.
pub(super) fn cursor_row_id(app: &Application) -> Option<Id> {
    let hit = app.view().cursor_hit()?;
    Some(row_id(app.results().generation, hit))
}

/// Ciò che il frame precedente lascia a questo: decide la riga nell'ordine del Tab, il
/// focus arrivato da fuori della tabella e lo scorrimento da cui compensare.
#[derive(Debug, Clone, Copy, Default)]
struct RowsMemory {
    /// La tabella a cui si riferisce: numero della ricerca e colonna Termine.
    table: (u64, bool),
    /// Vero se la riga del cursore era disegnata.
    cursor_drawn: bool,
    /// Vero se una riga disegnata aveva il focus della tastiera.
    focus_inside: bool,
    /// Lo scorrimento verticale della tabella, in punti.
    scroll_offset: f32,
}

/// Ciò che serve per disegnare le righe di un frame.
struct RowsFrame<'a> {
    i18n: &'a I18n,
    tokens: &'a Tokens,
    results: &'a Results,
    view: &'a ResultsView,
    with_terms: bool,
    /// Il risultato con il cursore della tastiera.
    cursor: Option<usize>,
    /// Il risultato la cui riga deve prendere il focus (dopo un clic o una freccia).
    focus_request: Option<usize>,
    previous: RowsMemory,
    /// Il contenitore delle righe, ritagliato sul corpo della tabella.
    grid: &'a Ui,
    /// Lo spazio orizzontale di una riga.
    row_x: Rangef,
}

/// Le righe disegnate in un frame e ciò che è successo disegnandole.
#[derive(Debug, Default)]
struct DrawnRows {
    /// Posizioni della prima e dell'ultima riga disegnata.
    range: Option<(usize, usize)>,
    /// Vero se la riga del cursore è tra queste.
    cursor_drawn: bool,
    /// Vero se una di queste ha il focus della tastiera.
    focus_inside: bool,
    /// Vero se la riga chiesta dal modello ha preso il focus.
    focus_taken: bool,
    /// Il nodo a cui F6 porta il focus.
    pane_target: Option<Id>,
}

impl DrawnRows {
    /// Registra la riga in `position`; vero se è la prima disegnata.
    fn record(&mut self, position: usize) -> bool {
        match &mut self.range {
            Some((_, last)) => {
                *last = position;
                false
            }
            None => {
                self.range = Some((position, position));
                true
            }
        }
    }
}

pub(super) fn results(app: &mut Application, ui: &mut Ui, actions: &mut Vec<Action>) {
    let state = app.empty_state();
    if state != EmptyState::None {
        empty(app, ui, state, actions);
        return;
    }
    let i18n = app.i18n().clone();
    let tokens = theme::current(ui.ctx());
    let with_terms = app.results().terms.len() > 1;
    let table = (app.results().generation, with_terms);
    let rows = app.view().len();
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + ROW_PADDING;
    let row_stride = row_height + ui.spacing().item_spacing.y;
    let previous = ui
        .data(|data| data.get_temp::<RowsMemory>(rows_memory_id()))
        .filter(|memory| memory.table == table)
        .unwrap_or_default();
    let scroll_target = app.view_mut().take_scroll_target();
    let scroll_shift = app.view_mut().take_scroll_shift();
    let columns = columns(with_terms);
    let pane_rect = ui.max_rect();
    let mut grid = grid_container(ui, i18n.tr("table-results-name"), rows, columns.len());

    // Senza effetto elastico ai bordi (la ListView di Windows non lo ha): egui prenderebbe
    // lo scorrimento oltre il fondo del frame precedente, dovuto alle righe entrate sopra,
    // per un rimbalzo e lo tirerebbe indietro.
    let rubber_band = std::mem::replace(&mut ui.spacing_mut().scroll.kinetic.rubber_band, false);
    let mut builder = table_builder(ui, app.layout.columns, table);
    if let Some(position) = scroll_target {
        builder = builder.scroll_to_row(position, None);
    } else if scroll_shift > 0 {
        // Righe entrate sopra la riga ferma: la tabella scorre di altrettante righe.
        let shift = row_stride * scroll_shift as f32;
        builder = builder.vertical_scroll_offset(previous.scroll_offset + shift);
    }

    let app_ref: &Application = app;
    let sort = app_ref.view().sort();
    let mut new_widths = None;
    let mut drawn = DrawnRows::default();
    let output = builder
        .header(row_height, |header| {
            sort_headers(header, &columns, sort, &i18n, actions);
        })
        .body(|mut body| {
            new_widths = Some(body.widths().to_vec());
            grid.set_clip_rect(body.ui_mut().clip_rect());
            let frame = RowsFrame {
                i18n: &i18n,
                tokens: &tokens,
                results: app_ref.results(),
                view: app_ref.view(),
                with_terms,
                cursor: app_ref.view().cursor_hit(),
                focus_request: app_ref.view().focus_request(),
                previous,
                grid: &grid,
                row_x: body.max_rect().x_range(),
            };
            body.rows(row_height, rows, |row| {
                draw_row(&frame, row, &mut drawn, actions);
            });
        });
    ui.spacing_mut().scroll.kinetic.rubber_band = rubber_band;
    grid.expand_to_include_rect(output.inner_rect);

    if let Some(widths) = new_widths {
        store_widths(&mut app.layout.columns, &widths, with_terms);
    }
    ui.data_mut(|data| {
        let memory = RowsMemory {
            table,
            cursor_drawn: drawn.cursor_drawn,
            focus_inside: drawn.focus_inside,
            scroll_offset: output.state.clamped_offset().y,
        };
        data.insert_temp(rows_memory_id(), memory);
    });
    let view = app.view_mut();
    if drawn.focus_taken {
        view.focus_done();
    }
    if let Some((first, last)) = drawn.range {
        view.set_visible_rows(first, last);
    }
    if let Some(target) = drawn.pane_target {
        panes::register(ui.ctx(), Pane::Results, pane_rect, target);
    }
}

/// Le colonne mostrate, con l'ID del titolo: Termine solo con più termini.
fn columns(with_terms: bool) -> Vec<(SortColumn, &'static str)> {
    let mut columns = vec![(SortColumn::Word, "column-word")];
    if with_terms {
        columns.push((SortColumn::Term, "column-term"));
    }
    columns.extend([
        (SortColumn::File, "column-file"),
        (SortColumn::Position, "column-position"),
        (SortColumn::Context, "column-context"),
    ]);
    columns
}

/// La tabella della ricerca `generation`, con le larghezze ricordate. Le celle non
/// sentono i clic: li sente il nodo della riga ([`draw_row`]).
fn table_builder(
    ui: &mut Ui,
    widths: ColumnWidths,
    (generation, with_terms): (u64, bool),
) -> TableBuilder<'_> {
    let resizable = |width: f32| {
        Column::initial(width)
            .range(*ColumnWidths::RANGE.start()..=*ColumnWidths::RANGE.end())
            .clip(true)
    };
    let mut builder = TableBuilder::new(ui)
        .id_salt(("results", generation, with_terms))
        .striped(true)
        .resizable(true)
        .auto_shrink([false, false])
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(resizable(widths.word));
    if with_terms {
        builder = builder.column(resizable(widths.term));
    }
    builder
        .column(resizable(widths.file))
        .column(resizable(widths.position))
        .column(Column::remainder().clip(true))
}

/// Il contenitore delle righe per i lettori di schermo: una griglia a selezione multipla
/// con il numero di righe (tutte, anche quelle non disegnate) e di colonne. Le righe nate
/// da lui con [`Ui::interact`] ne sono figlie.
fn grid_container(ui: &mut Ui, name: String, rows: usize, columns: usize) -> Ui {
    let grid = ui.new_child(
        UiBuilder::new()
            .scope_id(grid_id())
            .accessibility_role(Role::Grid)
            .accessibility_label(name),
    );
    ui.ctx().accesskit_node_builder(grid.unique_id(), |node| {
        node.set_row_count(rows);
        node.set_column_count(columns);
        node.set_size_of_set(rows);
        node.set_multiselectable();
    });
    grid
}

/// I titoli delle colonne: un clic ordina per quella colonna, un altro inverte il verso.
fn sort_headers(
    mut header: TableRow<'_, '_>,
    columns: &[(SortColumn, &'static str)],
    sort: Option<SortSpec>,
    i18n: &I18n,
    actions: &mut Vec<Action>,
) {
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
            // Senza cornice a riposo, con la cornice (e il bordo di focus) quando il
            // puntatore o la tastiera ci arrivano.
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
}

/// Disegna la riga `row` e raccoglie i gesti fatti su di lei.
fn draw_row(
    frame: &RowsFrame<'_>,
    mut row: TableRow<'_, '_>,
    drawn: &mut DrawnRows,
    actions: &mut Vec<Action>,
) {
    let position = row.index();
    let Some(hit_index) = frame.view.hit_at(position) else {
        return;
    };
    let Some(hit) = frame.results.hits.get(hit_index) else {
        return;
    };
    let selected = frame.view.is_selected(hit_index);
    row.set_selected(selected);
    let first_drawn = drawn.record(position);
    let is_cursor = frame.cursor == Some(hit_index);
    drawn.cursor_drawn |= is_cursor;
    // Una sola riga è nell'ordine del Tab: quella del cursore se era disegnata nel frame
    // precedente, altrimenti la prima disegnata. F6 porta il focus lì.
    let tab_stop = if frame.previous.cursor_drawn {
        is_cursor
    } else {
        first_drawn
    };
    let id = row_id(frame.results.generation, hit_index);
    if tab_stop {
        drawn.pane_target = Some(id);
    }
    let clip = frame.grid.clip_rect();

    let mut node = None;
    let mut hovered = false;
    row.col(|ui| {
        // Il nodo nasce prima del contenuto delle celle: le etichette restano sopra.
        let rect = Rect::from_x_y_ranges(frame.row_x, ui.max_rect().y_range());
        let sense = if tab_stop || frame.focus_request == Some(hit_index) {
            Sense::CLICK | Sense::FOCUSABLE
        } else {
            Sense::CLICK
        };
        let response = frame.grid.interact(rect, id, sense);
        describe_row(frame, &response, hit, position, selected);
        hovered = response.hovered() && !selected;
        paint_hover(ui, clip, hovered);
        if selected {
            paint_selection_mark(ui, frame.tokens);
        }
        ui.add_space(SELECTION_MARK_WIDTH + SPACE[0]);
        ui.add(Label::new(format::word(hit)).truncate().selectable(false));
        node = Some(response);
    });
    if frame.with_terms {
        row.col(|ui| {
            paint_hover(ui, clip, hovered);
            let terms = format::matched_terms(&frame.results.terms, hit.terms);
            ui.add(Label::new(terms).truncate().selectable(false));
        });
    }
    row.col(|ui| {
        paint_hover(ui, clip, hovered);
        let root = frame.results.roots.get(hit.root).map(PathBuf::as_path);
        let file = root.map_or_else(|| hit.doc.to_string(), |r| hit.doc.display_relative(r));
        ui.add(Label::new(file).truncate().selectable(false))
            .on_hover_text(hit.doc.to_string());
    });
    row.col(|ui| {
        paint_hover(ui, clip, hovered);
        let location = format::location(frame.i18n, &hit.location);
        ui.add(Label::new(location).truncate().selectable(false));
    });
    row.col(|ui| {
        paint_hover(ui, clip, hovered);
        let job = context_job(&hit.snippet, frame.tokens);
        ui.add(Label::new(job).truncate().selectable(false));
    });
    if let Some(node) = node {
        row_focus(frame, &node, hit_index, drawn, actions);
        row_gestures(frame, &node, hit_index, selected, actions);
    }
}

/// Il nodo della riga per i lettori di schermo: ruolo, nome, selezione e posizione nella
/// griglia.
fn describe_row(
    frame: &RowsFrame<'_>,
    node: &Response,
    hit: &Hit,
    position: usize,
    selected: bool,
) {
    node.widget_info(|| {
        WidgetInfo::labeled(Role::Row, true, row_name(frame.i18n, frame.results, hit))
    });
    node.ctx.accesskit_node_builder(node.id, |builder| {
        builder.set_selected(selected);
        builder.set_row_index(position);
        builder.set_position_in_set(position);
    });
}

/// Il focus della riga del risultato `hit_index`: quello chiesto dal modello (dopo un clic
/// o una freccia), il bordo di 2 px, le frecce che muovono il cursore, e il cursore che
/// segue il focus arrivato da fuori della tabella (Tab, F6) senza toccare la selezione,
/// come nella ListView di Windows.
fn row_focus(
    frame: &RowsFrame<'_>,
    node: &Response,
    hit_index: usize,
    drawn: &mut DrawnRows,
    actions: &mut Vec<Action>,
) {
    if frame.focus_request == Some(hit_index) {
        node.request_focus();
        drawn.focus_taken = true;
    }
    if !node.has_focus() {
        return;
    }
    drawn.focus_inside = true;
    // Le frecce muovono il cursore della tabella, non il focus di egui.
    let arrows = EventFilter {
        vertical_arrows: true,
        ..EventFilter::default()
    };
    node.ctx
        .memory_mut(|memory| memory.set_focus_lock_filter(node.id, arrows));
    // Dopo tutte le celle, sopra i loro fondi; solo sulla parte visibile della riga.
    Painter::new(node.ctx.clone(), frame.grid.layer_id(), node.interact_rect).rect_stroke(
        node.rect,
        0.0,
        theme::focus_stroke(frame.tokens),
        StrokeKind::Inside,
    );
    // F6 chiede il focus dopo il disegno: egui non lo segnala come appena preso, quindi si
    // guarda se nel frame precedente il focus era fuori dalle righe.
    let arrived = !frame.previous.focus_inside && frame.focus_request.is_none();
    if arrived && frame.cursor != Some(hit_index) {
        actions.push(Action::FocusRow(hit_index));
    }
}

/// Clic, doppio clic e menu contestuale sulla riga del risultato `hit_index`.
///
/// Il clic destro su una riga non selezionata la seleziona e ci porta il cursore, come in
/// Esplora file; su una riga già selezionata lascia la selezione com'è (per copiarla).
fn row_gestures(
    frame: &RowsFrame<'_>,
    node: &Response,
    hit_index: usize,
    selected: bool,
    actions: &mut Vec<Action>,
) {
    if node.double_clicked() {
        actions.push(Action::DoubleClickRow(hit_index));
    } else if node.clicked() {
        actions.push(Action::ClickRow(hit_index, ui_click_kind(node)));
    }
    if node.secondary_clicked() && !selected {
        actions.push(Action::ClickRow(hit_index, ClickKind::Replace));
    }
    node.context_menu(|ui| row_menu(frame.i18n, ui, actions));
}

/// Il fondo della riga sotto il puntatore, in una cella. Le celle non sentono il puntatore
/// (lo sente il nodo della riga), quindi la tabella non lo disegna da sé; come lei copre
/// anche metà dello spazio tra le celle. `clip` è il corpo della tabella.
fn paint_hover(ui: &Ui, clip: Rect, hovered: bool) {
    if hovered {
        let rect = ui.max_rect().expand2(0.5 * ui.spacing().item_spacing);
        let fill = ui.visuals().widgets.hovered.bg_fill;
        Painter::new(ui.ctx().clone(), ui.layer_id(), clip).rect_filled(rect, 0.0, fill);
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
fn row_name(i18n: &I18n, results: &Results, hit: &Hit) -> String {
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
fn row_menu(i18n: &I18n, ui: &mut Ui, actions: &mut Vec<Action>) {
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
