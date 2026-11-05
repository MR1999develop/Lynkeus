// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Barra dei messaggi sopra la tabella: esito di copia ed esportazione, aperture non
//! riuscite o rifiutate, filtri ripristinati (con "Annulla"), impostazioni che non si
//! salvano, cartelle, clic destro e cache. Una riga per messaggio, dal più recente, ognuna
//! con il suo "Chiudi": l'esito di un gesto resta finché arriva un altro messaggio, i
//! problemi che restano finché l'utente li chiude ([`Notice::is_lasting`]). Gli errori
//! iniziano con una parola che lo dice, non solo con il colore. Ogni testo è una regione
//! "live": i lettori di schermo lo annunciano senza spostare il focus.

use egui::accesskit::Live;
use egui::{Label, RichText, Ui};

use super::{Action, Command};
use crate::app::{Application, Notice};
use crate::context_menu::MenuError;
use crate::i18n::I18n;
use crate::launch::EntryFailure;
use crate::settings::SettingsProblem;
use crate::theme::{self, SPACE};

/// Tutti i messaggi, uno per riga, dal più recente.
pub(super) fn bar(app: &Application, ui: &mut Ui, actions: &mut Vec<Action>) {
    ui.add_space(SPACE[0]);
    for (index, notice) in app.notices().iter().enumerate() {
        ui.push_id(index, |ui| row(app, notice, index, ui, actions));
    }
    ui.add_space(SPACE[0]);
}

/// Il testo di un messaggio e se è un errore.
fn text(i18n: &I18n, notice: &Notice) -> (String, bool) {
    match notice {
        Notice::Executable { .. } => (i18n.tr("notice-executable"), false),
        Notice::ArchiveEntry => (i18n.tr("notice-archive-entry"), false),
        Notice::OpenFailed { file } => (
            i18n.tr_args("notice-open-failed", &[("file", file.clone().into())]),
            true,
        ),
        Notice::RevealFailed { file } => (
            i18n.tr_args("notice-reveal-failed", &[("file", file.clone().into())]),
            true,
        ),
        Notice::Copied { rows } => (
            i18n.tr_args("notice-copied", &[("count", (*rows).into())]),
            false,
        ),
        Notice::CopyTooLarge { rows } => (
            i18n.tr_args("notice-copy-too-large", &[("count", (*rows).into())]),
            false,
        ),
        Notice::Exported { rows, file } => (
            i18n.tr_args(
                "notice-exported",
                &[("count", (*rows).into()), ("file", file.clone().into())],
            ),
            false,
        ),
        Notice::ExportFailed { file } => (
            i18n.tr_args("notice-export-failed", &[("file", file.clone().into())]),
            true,
        ),
        Notice::NoThread => (i18n.tr("notice-no-thread"), true),
        Notice::CacheCleared { freed_mib } => (
            i18n.tr_args("notice-cache-cleared", &[("mib", (*freed_mib).into())]),
            false,
        ),
        Notice::CacheClearFailed => (i18n.tr("notice-cache-clear-failed"), true),
        Notice::EntryFailed { file, reason } => {
            let id = match reason {
                EntryFailure::Encrypted => "notice-entry-encrypted",
                EntryFailure::LimitExceeded => "notice-entry-limit",
                EntryFailure::Corrupt => "notice-entry-corrupt",
                EntryFailure::Unreadable => "notice-entry-unreadable",
            };
            (i18n.tr_args(id, &[("file", file.clone().into())]), true)
        }
        Notice::ContextMenuFailed(error) => {
            let id = match error {
                MenuError::ExeNotAbsolute | MenuError::ExeUnsafeChars => {
                    "notice-context-menu-exe-path"
                }
                MenuError::CommandTooLong => "notice-context-menu-too-long",
                MenuError::ExeDirWritable => "notice-context-menu-exe-dir-off",
                MenuError::Foreign => "notice-context-menu-foreign",
                MenuError::Io(_) => "notice-context-menu-io",
            };
            (i18n.tr(id), true)
        }
        Notice::PrivateDirsRefused { count } => (
            i18n.tr_args("notice-private-dirs", &[("count", (*count).into())]),
            true,
        ),
        Notice::RootsLimit { max } => (
            i18n.tr_args("notice-roots-limit", &[("max", (*max).into())]),
            false,
        ),
        Notice::FiltersReset { .. } => (i18n.tr("notice-filters-reset"), false),
        Notice::SettingsNotSaved(problem) => {
            let id = match problem {
                SettingsProblem::NewerVersion => "notice-settings-newer",
                SettingsProblem::Unreadable => "notice-settings-unreadable",
                SettingsProblem::WriteFailed => "notice-settings-write-failed",
            };
            (i18n.tr(id), true)
        }
        Notice::SettingsReplaced => (i18n.tr("notice-settings-replaced"), true),
        Notice::CacheNotWritten => (i18n.tr("notice-cache-not-written"), true),
    }
}

/// Una riga: da destra Chiudi, l'eventuale azione, poi il testo nello spazio che resta.
fn row(app: &Application, notice: &Notice, index: usize, ui: &mut Ui, actions: &mut Vec<Action>) {
    let i18n = app.i18n();
    let tokens = theme::current(ui.ctx());
    let (text, is_error) = text(i18n, notice);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button(i18n.tr("action-close")).clicked() {
            actions.push(Action::DismissNotice(index));
        }
        if let Notice::Executable { doc } = notice
            && ui.button(i18n.tr("menu-reveal")).clicked()
        {
            actions.push(Action::DismissNotice(index));
            actions.push(Action::Reveal(doc.clone()));
        }
        if let Notice::FiltersReset { .. } = notice
            && ui.button(i18n.tr("action-undo")).clicked()
        {
            actions.push(Action::UndoFiltersReset);
        }
        if let Notice::CopyTooLarge { .. } = notice
            && export_button(app, ui).clicked()
        {
            actions.push(Action::DismissNotice(index));
            actions.push(Action::ExportSelection);
        }
        let text = if is_error {
            RichText::new(text).color(tokens.error)
        } else {
            RichText::new(text)
        };
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let response = ui.add(Label::new(text).wrap());
            ui.ctx()
                .accesskit_node_builder(response.id, |node| node.set_live(Live::Polite));
        });
    });
}

/// "Esporta…" per le righe che non stanno negli appunti: apre le impostazioni di
/// esportazione con le sole righe selezionate. Spento, con il motivo, quando Esporta lo è.
fn export_button(app: &Application, ui: &mut Ui) -> egui::Response {
    let i18n = app.i18n();
    let availability = Command::ExportSettings.availability(app);
    let response = ui.add_enabled(
        availability.is_ok(),
        egui::Button::new(i18n.tr(Command::Export.label_id())),
    );
    match availability {
        Ok(()) => response,
        Err(reason) => response.on_disabled_hover_text(i18n.tr(reason)),
    }
}
