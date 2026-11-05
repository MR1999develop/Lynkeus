// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Barra di stato: contatori, tempo trascorso, file in lettura e, dopo un secondo di
//! ricerca, l'avanzamento. Finché la scansione delle cartelle non finisce il totale dei file
//! non è noto: l'avanzamento è uno spinner con i conteggi ("30 di 40 file trovati finora"),
//! mai una percentuale che resterebbe ferma vicino al 100%. Poi è una barra con la frazione
//! ("30 di 40 file") e, oltre dieci secondi, il tempo restante stimato.
//!
//! Il testo dello stato è una regione "live" per i lettori di schermo quando la ricerca non
//! è in corso: l'esito (completata, annullata, errore) si annuncia senza spostare il focus;
//! durante la ricerca i contatori cambiano ogni 250 ms e non si annunciano.

use egui::accesskit::Live;
use egui::{Label, ProgressBar, RichText, Spinner, Ui};

use crate::app::{Application, PROGRESS_DELAY, SearchStatus, StartError};
use crate::format;
use crate::theme::{self, PROGRESS_WIDTH, SPACE};

pub(super) fn bar(app: &Application, ui: &mut Ui) {
    let i18n = app.i18n();
    let status = app.status();
    let tokens = theme::current(ui.ctx());
    ui.add_space(SPACE[0]);
    ui.horizontal(|ui| {
        if let Some(progress) = status.progress() {
            match progress.fraction() {
                Some(fraction) => ui.add(ProgressBar::new(fraction).desired_width(PROGRESS_WIDTH)),
                None => ui.add(Spinner::new()),
            };
            ui.label(format::progress(i18n, progress));
        }
        let text = match (&status, app.start_error()) {
            (_, Some(StartError::Spawn(_))) => {
                RichText::new(i18n.tr("error-start-failed")).color(tokens.error)
            }
            (_, Some(StartError::Form(_))) => {
                RichText::new(i18n.tr("error-form-invalid")).color(tokens.error)
            }
            (SearchStatus::WorkerLost, _) => {
                RichText::new(format::status(i18n, &status, app.limits().max_results))
                    .color(tokens.error)
            }
            _ => RichText::new(format::status(i18n, &status, app.limits().max_results)),
        };
        let response = ui.add(Label::new(text).truncate());
        let live = if matches!(status, SearchStatus::Running { .. }) {
            Live::Off
        } else {
            Live::Polite
        };
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_live(live));
        let selected = app.view().selected_count();
        if selected > 0 {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(i18n.tr_args("status-selected", &[("count", selected.into())]));
            });
        }
    });
    if let Some(elapsed) = app.copy_elapsed()
        && elapsed >= PROGRESS_DELAY
    {
        ui.horizontal(|ui| {
            ui.add(Spinner::new());
            ui.label(i18n.tr("status-copying"));
        });
    }
    if let Some(doc) = app.current_file() {
        ui.add(
            Label::new(
                RichText::new(
                    i18n.tr_args("status-current-file", &[("file", doc.to_string().into())]),
                )
                .weak(),
            )
            .truncate(),
        );
    }
    ui.add_space(SPACE[0]);
}
