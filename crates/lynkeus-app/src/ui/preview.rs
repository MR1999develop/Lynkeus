// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Anteprima sotto la tabella dei risultati: intestazione con il file, la posizione e i
//! comandi precedente/successiva, poi le unità di testo intorno al risultato.
//!
//! L'unità del risultato è segnata da "›" e da un fondo; le corrispondenze come nella
//! tabella (fondo, SemiBold, sottolineate): il colore non è mai l'unico segnale. Il
//! contenuto lo prepara [`Application::update_preview`] in un thread; qui si disegna
//! soltanto. Le unità lunghe (un paragrafo intero) vanno a capo: si scorre solo in
//! verticale.
//!
//! I bottoni dell'intestazione mostrano un testo breve ("« File") e hanno come nome
//! accessibile quello completo del comando ("File precedente").

use egui::text::LayoutJob;
use egui::{Button, Color32, FontId, Id, Label, RichText, ScrollArea, Stroke, TextFormat, Ui};

use super::panes::{self, Pane};
use super::{Action, Command};
use crate::app::{Application, PROGRESS_DELAY, PreviewState};
use crate::format;
use crate::preview::{Loaded, PreviewError, PreviewUnit};
use crate::results::Step;
use crate::theme::{self, SPACE, Tokens};

/// Segno davanti all'unità del risultato.
const FOCUS_MARK: &str = "›";

pub(super) fn show(app: &mut Application, ui: &mut Ui, actions: &mut Vec<Action>) {
    let pane = ui.max_rect();
    ui.add_space(SPACE[0]);
    if let Some(first_button) = header(app, ui, actions) {
        panes::register(ui.ctx(), Pane::Preview, pane, first_button);
    }
    ui.separator();
    let i18n = app.i18n().clone();
    let tokens = theme::current(ui.ctx());
    match app.preview().clone() {
        PreviewState::NoRow => {
            ui.label(RichText::new(i18n.tr("preview-empty")).color(tokens.text_weak));
        }
        PreviewState::Loading(_, started) => {
            if started.elapsed() >= PROGRESS_DELAY {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(i18n.tr("preview-loading"));
                });
            } else {
                // Lo spinner compare solo dopo un secondo: ridisegnare per allora.
                ui.ctx().request_repaint_after(PROGRESS_DELAY);
            }
        }
        PreviewState::Failed(_, error) => {
            ui.add(Label::new(RichText::new(error_text(app, error)).color(tokens.error)).wrap());
        }
        PreviewState::Ready(key, Loaded::ContextOnly) => {
            ui.add(
                Label::new(RichText::new(i18n.tr("preview-context-only")).color(tokens.text_weak))
                    .wrap(),
            );
            ui.add_space(SPACE[1]);
            if let Some(hit) = app.results().hits.get(key.hit) {
                let job = super::table::context_job(&hit.snippet, &tokens);
                ui.add(Label::new(job).wrap().selectable(false));
            }
        }
        PreviewState::Ready(key, Loaded::Units(units)) => {
            let scroll_now = app.view_state.preview_scrolled != Some(key);
            app.view_state.preview_scrolled = Some(key);
            ScrollArea::vertical()
                .id_salt("preview_units")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (index, unit) in units.units.iter().enumerate() {
                        let focused = index == units.focus;
                        let response = unit_row(app, ui, unit, focused, &tokens).response;
                        if focused && scroll_now {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                    }
                });
        }
    }
}

/// Intestazione: documento e posizione del risultato, poi i passi tra i risultati e Apri.
/// Restituisce il primo bottone attivo (dove va il focus arrivando all'anteprima con F6).
fn header(app: &Application, ui: &mut Ui, actions: &mut Vec<Action>) -> Option<Id> {
    let i18n = app.i18n();
    ui.horizontal(|ui| {
        let mut first = None;
        for (step, id) in [
            (Step::PreviousFile, "preview-previous-file"),
            (Step::PreviousInFile, "preview-previous"),
            (Step::NextInFile, "preview-next"),
            (Step::NextFile, "preview-next-file"),
        ] {
            let command = Command::Step(step);
            let button = step_button(app, ui, command, id, actions);
            first = first.or(button);
        }
        let open = Command::Open;
        let open = step_button(app, ui, open, "preview-open", actions);
        ui.add_space(SPACE[1]);
        if let Some(hit) = app.cursor_hit() {
            let root = app.results().roots.get(hit.root);
            let file = root.map_or_else(|| hit.doc.to_string(), |r| hit.doc.display_relative(r));
            let location = format::location(i18n, &hit.location);
            let title = i18n.tr_args(
                "preview-title",
                &[("file", file.into()), ("location", location.into())],
            );
            ui.add(Label::new(RichText::new(title).family(theme::semibold_family())).truncate())
                .on_hover_text(hit.doc.to_string());
        }
        first.or(open)
    })
    .inner
}

/// Un bottone dell'intestazione legato a un comando: testo breve, nome accessibile del
/// comando, disabilitato con il motivo nel tooltip, la scorciatoia nel tooltip quando è
/// attivo. Restituisce il suo `Id` se è attivo.
fn step_button(
    app: &Application,
    ui: &mut Ui,
    command: Command,
    label_id: &str,
    actions: &mut Vec<Action>,
) -> Option<Id> {
    let i18n = app.i18n();
    let availability = command.availability(app);
    let response = ui
        .add_enabled(availability.is_ok(), Button::new(i18n.tr(label_id)))
        .accessible_name(i18n.tr(command.label_id()));
    let id = availability.is_ok().then_some(response.id);
    let response = match availability {
        Err(reason) => response.on_disabled_hover_text(i18n.tr(reason)),
        Ok(()) => {
            let mut tip = i18n.tr(command.label_id());
            if let Some(shortcut) = command.shortcut() {
                tip.push_str(&format!(" ({})", ui.ctx().format_shortcut(&shortcut)));
            }
            response.on_hover_text(tip)
        }
    };
    if response.clicked() {
        actions.push(Action::Command(command));
    }
    id
}

/// Una riga dell'anteprima: posizione, segno del risultato, testo con le corrispondenze,
/// che va a capo nello spazio della riga.
fn unit_row(
    app: &Application,
    ui: &mut Ui,
    unit: &PreviewUnit,
    focused: bool,
    tokens: &Tokens,
) -> egui::InnerResponse<()> {
    let i18n = app.i18n();
    ui.horizontal_top(|ui| {
        let location = format::location(i18n, &unit.location);
        let mark = if focused { FOCUS_MARK } else { " " };
        ui.add_sized(
            [LOCATION_WIDTH, ui.spacing().interact_size.y],
            Label::new(
                RichText::new(format!("{mark} {location}"))
                    .size(theme::CAPTION_SIZE)
                    .color(if focused {
                        tokens.text_strong
                    } else {
                        tokens.text_weak
                    }),
            )
            .selectable(false),
        );
        let job = unit_job(unit, focused, tokens);
        ui.add(Label::new(job).wrap().selectable(false));
    })
}

/// Larghezza della colonna delle posizioni.
const LOCATION_WIDTH: f32 = 96.0;

/// Il testo di un'unità con le corrispondenze evidenziate; l'unità del risultato ha il
/// fondo della selezione.
fn unit_job(unit: &PreviewUnit, focused: bool, tokens: &Tokens) -> LayoutJob {
    let background = if focused {
        tokens.selection_bg
    } else {
        Color32::TRANSPARENT
    };
    let normal = TextFormat {
        background,
        ..TextFormat::simple(FontId::proportional(theme::BODY_SIZE), tokens.text_strong)
    };
    let highlight = TextFormat {
        font_id: FontId::new(theme::BODY_SIZE, theme::semibold_family()),
        color: tokens.text_strong,
        background: tokens.highlight_bg,
        underline: Stroke::new(1.0, tokens.text_strong),
        ..TextFormat::simple(FontId::default(), Color32::PLACEHOLDER)
    };
    let mut job = LayoutJob::default();
    let mut cursor = 0;
    for range in &unit.highlights {
        if let (Some(before), Some(marked)) = (
            unit.text.get(cursor..range.start),
            unit.text.get(range.clone()),
        ) {
            job.append(before, 0.0, normal.clone());
            job.append(marked, 0.0, highlight.clone());
            cursor = range.end;
        }
    }
    job.append(unit.text.get(cursor..).unwrap_or_default(), 0.0, normal);
    job
}

/// Il messaggio di un'anteprima non riuscita, con il rimedio.
fn error_text(app: &Application, error: PreviewError) -> String {
    let i18n = app.i18n();
    match error {
        PreviewError::Unreadable(kind) => i18n.tr_args(
            "preview-unreadable",
            &[("reason", i18n.tr(format::warning_id(kind)).into())],
        ),
        PreviewError::NotText | PreviewError::Changed => i18n.tr("preview-changed"),
        PreviewError::NoThread => i18n.tr("preview-no-thread"),
    }
}
