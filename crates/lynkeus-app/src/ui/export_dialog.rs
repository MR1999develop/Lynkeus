// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Dialogo "Impostazioni di esportazione…": formato e righe. Modifica una copia delle
//! scelte: "Esporta…" le salva e chiede dove scrivere, "Annulla" (o Esc) le butta. Il
//! comando "Esporta" usa poi le ultime scelte senza passare di qui.

use egui::{Id, RichText, Ui};

use super::Action;
use crate::app::Application;
use crate::export::{ExportFormat, ExportScope, csv_separator};
use crate::platform::ACTION_BUTTON_FIRST;
use crate::theme::{self, SPACE};

/// Larghezza massima del dialogo.
const MAX_WIDTH: f32 = 440.0;

pub(super) fn show(app: &mut Application, ctx: &egui::Context, actions: &mut Vec<Action>) {
    let Some(mut draft) = app.view_state.export_dialog.clone() else {
        return;
    };
    let i18n = app.i18n().clone();
    let visible = app.export_row_count(ExportScope::Visible);
    let selected = app.export_row_count(ExportScope::Selected);
    let separator = csv_separator(i18n.language());
    let modal = egui::Modal::new(Id::unique("export_settings")).show(ctx, |ui| {
        ui.set_max_width(MAX_WIDTH);
        ui.heading(i18n.tr("export-dialog-title"));
        ui.add_space(SPACE[2]);

        section(ui, &i18n.tr("export-format-title"));
        ui.radio_value(
            &mut draft.format,
            ExportFormat::Csv,
            i18n.tr_args(
                "export-format-csv-choice",
                &[("separator", separator.to_string().into())],
            ),
        );
        ui.radio_value(
            &mut draft.format,
            ExportFormat::Txt,
            i18n.tr("export-format-txt"),
        );
        ui.add_space(SPACE[2]);

        section(ui, &i18n.tr("export-scope-title"));
        ui.radio_value(
            &mut draft.scope,
            ExportScope::Visible,
            i18n.tr_args("export-scope-visible", &[("count", visible.into())]),
        );
        ui.add_enabled_ui(selected > 0, |ui| {
            ui.radio_value(
                &mut draft.scope,
                ExportScope::Selected,
                i18n.tr_args("export-scope-selected", &[("count", selected.into())]),
            )
            .on_disabled_hover_text(i18n.tr("reason-no-selection"));
        });
        ui.add_space(SPACE[3]);

        let can_export = draft.scope == ExportScope::Visible || selected > 0;
        dialog_buttons(
            ui,
            &i18n.tr("export-dialog-confirm"),
            &i18n.tr("action-cancel"),
            can_export,
        )
    });
    // Nessuna riga selezionata: le righe visibili sono l'unica scelta possibile.
    if selected == 0 {
        draft.scope = ExportScope::Visible;
    }
    let choice = if modal.should_close() {
        Some(false)
    } else {
        modal.inner
    };
    match choice {
        Some(confirmed) => {
            app.view_state.export_dialog = Some(draft);
            actions.push(Action::CloseExportDialog(confirmed));
        }
        None => app.view_state.export_dialog = Some(draft),
    }
}

fn section(ui: &mut Ui, title: &str) {
    ui.label(RichText::new(title).family(theme::semibold_family()));
}

/// I bottoni del dialogo nell'ordine della piattaforma, allineati a destra; Invio conferma
/// (l'azione non è distruttiva). `Some(true)` = azione, `Some(false)` = Annulla.
fn dialog_buttons(ui: &mut Ui, action: &str, cancel: &str, enabled: bool) -> Option<bool> {
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    let (confirmed, cancelled) = ui
        .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Da destra a sinistra: il primo aggiunto è quello più a destra.
            let action_button =
                |ui: &mut Ui| ui.add_enabled(enabled, egui::Button::new(action)).clicked();
            if ACTION_BUTTON_FIRST {
                // Windows: [Azione] [Annulla].
                let cancelled = ui.button(cancel).clicked();
                (action_button(ui), cancelled)
            } else {
                // GNOME: [Annulla] [Azione].
                let confirmed = action_button(ui);
                (confirmed, ui.button(cancel).clicked())
            }
        })
        .inner;
    if cancelled {
        Some(false)
    } else if confirmed || (enter && enabled) {
        Some(true)
    } else {
        None
    }
}
