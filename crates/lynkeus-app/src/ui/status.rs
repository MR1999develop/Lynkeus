// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Barra di stato: contatori, tempo trascorso, file in lettura e, dopo un secondo di
//! ricerca, la barra d'avanzamento (file letti su file trovati finora).

use egui::{Label, ProgressBar, RichText, Ui};

use crate::app::{Application, PROGRESS_DELAY, SearchStatus, StartError};
use crate::format;
use crate::theme::{self, SPACE};

/// Larghezza della barra d'avanzamento.
const PROGRESS_WIDTH: f32 = 180.0;

pub(super) fn bar(app: &Application, ui: &mut Ui) {
    let i18n = app.i18n();
    let status = app.status();
    ui.add_space(SPACE[0]);
    ui.horizontal(|ui| {
        if let SearchStatus::Running { counts, elapsed } = &status
            && *elapsed >= PROGRESS_DELAY
        {
            let done = counts
                .files_searched
                .saturating_add(counts.files_skipped)
                .saturating_add(counts.binary_skipped)
                .saturating_add(counts.warnings);
            let seen = counts.files_seen.max(done).max(1);
            #[expect(
                clippy::cast_precision_loss,
                reason = "frazione per una barra: la precisione oltre 2^24 file non conta"
            )]
            let fraction = done as f32 / seen as f32;
            ui.add(
                ProgressBar::new(fraction)
                    .desired_width(PROGRESS_WIDTH)
                    .text(i18n.tr_args(
                        "status-progress",
                        &[("done", done.into()), ("seen", counts.files_seen.into())],
                    )),
            );
        }
        let text = match (&status, app.start_error()) {
            (_, Some(StartError::Spawn(_))) => {
                let tokens = theme::current(ui.ctx());
                RichText::new(i18n.tr("error-start-failed")).color(tokens.error)
            }
            (SearchStatus::WorkerLost, _) => {
                let tokens = theme::current(ui.ctx());
                RichText::new(format::status(i18n, &status, app.limits().max_results))
                    .color(tokens.error)
            }
            _ => RichText::new(format::status(i18n, &status, app.limits().max_results)),
        };
        ui.add(Label::new(text).truncate());
        let selected = app.view().selected_count();
        if selected > 0 {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(i18n.tr_args("status-selected", &[("count", selected.into())]));
            });
        }
    });
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
