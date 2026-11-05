// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Comandi dei menu con le loro scorciatoie: una sola tabella per la barra dei menu e per
//! la tastiera, così le due cose non divergono.

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::app::{Application, Tab};
use crate::export::ExportScope;
use crate::results::Step;

/// Un comando raggiungibile dal menu (e, se ha una scorciatoia, dalla tastiera).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    AddFolder,
    AddFile,
    Open,
    Reveal,
    Export,
    ExportSettings,
    Quit,
    Copy,
    SelectAll,
    SelectNone,
    FindTerms,
    AddTerm,
    Search,
    Cancel,
    Step(Step),
    ShowResults,
    ShowWarnings,
    ToggleSearchPanel,
    TogglePreview,
    ToggleFilters,
    NextPane,
    PreviousPane,
    About,
}

impl Command {
    /// Ordine in cui si leggono le scorciatoie: le più specifiche prima (Ctrl+Maiusc+O
    /// prima di Ctrl+O, Maiusc+F3 prima di F3), perché `consume_shortcut` ignora Maiusc e
    /// Alt in più.
    pub(crate) const SHORTCUT_ORDER: [Command; 23] = [
        Command::AddFile,
        Command::AddFolder,
        Command::Reveal,
        Command::ExportSettings,
        Command::Export,
        Command::Open,
        Command::Copy,
        Command::SelectNone,
        Command::SelectAll,
        Command::Quit,
        Command::FindTerms,
        Command::Search,
        Command::Cancel,
        Command::Step(Step::PreviousFile),
        Command::Step(Step::NextFile),
        Command::Step(Step::PreviousInFile),
        Command::Step(Step::NextInFile),
        Command::ShowResults,
        Command::ShowWarnings,
        Command::TogglePreview,
        Command::ToggleSearchPanel,
        Command::PreviousPane,
        Command::NextPane,
    ];

    /// La scorciatoia, se c'è.
    pub(crate) fn shortcut(self) -> Option<KeyboardShortcut> {
        let ctrl = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        let ctrl_shift = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
        let plain = |key| KeyboardShortcut::new(Modifiers::NONE, key);
        Some(match self {
            Command::AddFolder => ctrl(Key::O),
            Command::AddFile => ctrl_shift(Key::O),
            Command::Open => plain(Key::Enter),
            Command::Reveal => ctrl_shift(Key::R),
            Command::Export => ctrl(Key::E),
            Command::ExportSettings => ctrl_shift(Key::E),
            Command::Quit => ctrl(Key::Q),
            Command::Copy => ctrl(Key::C),
            Command::SelectAll => ctrl(Key::A),
            Command::SelectNone => ctrl_shift(Key::A),
            Command::FindTerms => ctrl(Key::F),
            Command::Search => plain(Key::F5),
            Command::Cancel => plain(Key::Escape),
            Command::Step(Step::NextInFile) => plain(Key::F3),
            Command::Step(Step::PreviousInFile) => KeyboardShortcut::new(Modifiers::SHIFT, Key::F3),
            Command::Step(Step::NextFile) => ctrl(Key::F3),
            Command::Step(Step::PreviousFile) => ctrl_shift(Key::F3),
            Command::ShowResults => ctrl(Key::Num1),
            Command::ShowWarnings => ctrl(Key::Num2),
            Command::TogglePreview => ctrl(Key::Num3),
            Command::ToggleSearchPanel => plain(Key::F9),
            Command::NextPane => plain(Key::F6),
            Command::PreviousPane => KeyboardShortcut::new(Modifiers::SHIFT, Key::F6),
            Command::AddTerm | Command::ToggleFilters | Command::About => return None,
        })
    }

    /// ID Fluent della voce di menu.
    pub(crate) fn label_id(self) -> &'static str {
        match self {
            Command::AddFolder => "menu-add-folder",
            Command::AddFile => "menu-add-file",
            Command::Open => "menu-open",
            Command::Reveal => "menu-reveal",
            Command::Export => "menu-export",
            Command::ExportSettings => "menu-export-settings",
            Command::Quit => "menu-quit",
            Command::Copy => "menu-copy",
            Command::SelectAll => "menu-select-all",
            Command::SelectNone => "menu-select-none",
            Command::FindTerms => "menu-find-terms",
            Command::AddTerm => "menu-add-term",
            Command::Search => "menu-search",
            Command::Cancel => "menu-cancel",
            Command::Step(Step::NextInFile) => "menu-next-in-file",
            Command::Step(Step::PreviousInFile) => "menu-previous-in-file",
            Command::Step(Step::NextFile) => "menu-next-file",
            Command::Step(Step::PreviousFile) => "menu-previous-file",
            Command::ShowResults => "menu-show-results",
            Command::ShowWarnings => "menu-show-warnings",
            Command::ToggleSearchPanel => "menu-search-panel",
            Command::TogglePreview => "menu-preview",
            Command::ToggleFilters => "menu-filters",
            Command::NextPane => "menu-next-pane",
            Command::PreviousPane => "menu-previous-pane",
            Command::About => "menu-about",
        }
    }

    /// Se il comando si può usare ora; altrimenti l'ID Fluent del motivo, mostrato nel
    /// tooltip della voce disabilitata.
    pub(crate) fn availability(self, app: &Application) -> Result<(), &'static str> {
        let has_rows = !app.view().is_empty();
        let on_results = app.view_state.tab == Tab::Results;
        let has_cursor = app.cursor_hit().is_some();
        match self {
            Command::AddFolder | Command::AddFile | Command::Export if app.is_dialog_open() => {
                Err("reason-dialog-open")
            }
            Command::Search if app.is_searching() => Err("reason-search-running"),
            Command::Cancel if !app.is_searching() => Err("reason-no-search"),
            Command::SelectAll | Command::SelectNone if !has_rows => Err("reason-no-results"),
            Command::SelectAll
            | Command::SelectNone
            | Command::Copy
            | Command::Open
            | Command::Reveal
            | Command::Step(_)
                if !on_results =>
            {
                Err("reason-not-results-tab")
            }
            Command::Copy if app.view().selected_count() == 0 => Err("reason-no-selection"),
            Command::Copy if app.is_copying() => Err("reason-copy-running"),
            Command::Open | Command::Reveal | Command::Step(_) if !has_cursor => {
                Err("reason-no-current-row")
            }
            Command::Step(step) if app.step_target(step).is_none() => Err(match step {
                Step::NextInFile => "reason-last-in-file",
                Step::PreviousInFile => "reason-first-in-file",
                Step::NextFile => "reason-last-file",
                Step::PreviousFile => "reason-first-file",
            }),
            Command::Export | Command::ExportSettings if !has_rows => {
                Err("reason-nothing-to-export")
            }
            Command::Export | Command::ExportSettings if app.is_searching() => {
                Err("reason-search-running")
            }
            Command::Export | Command::ExportSettings if app.is_exporting() => {
                Err("reason-export-running")
            }
            Command::Export
                if app.export_settings().scope == ExportScope::Selected
                    && app.view().selected_count() == 0 =>
            {
                Err("reason-export-no-selection")
            }
            Command::AddTerm if !app.form.can_add_term() => Err("reason-too-many-terms"),
            _ => Ok(()),
        }
    }

    /// Vero se la scorciatoia va lasciata a un menu o a un popup aperto: Esc lo chiude,
    /// Invio ne attiva la voce; non annullano la ricerca né aprono il risultato.
    pub(crate) fn yields_to_popups(self) -> bool {
        matches!(self, Command::Cancel | Command::Open)
    }

    /// Vero se la scorciatoia va lasciata al campo di testo con il focus (Ctrl+A seleziona
    /// il testo del campo, Ctrl+C lo copia, Esc ne annulla la modifica, Invio lo conferma).
    pub(crate) fn yields_to_text_fields(self) -> bool {
        matches!(
            self,
            Command::SelectAll
                | Command::SelectNone
                | Command::Cancel
                | Command::Copy
                | Command::Open
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_are_unique_and_specific_first() {
        let shortcuts: Vec<KeyboardShortcut> = Command::SHORTCUT_ORDER
            .iter()
            .filter_map(|c| c.shortcut())
            .collect();
        assert_eq!(shortcuts.len(), Command::SHORTCUT_ORDER.len());
        for (i, a) in shortcuts.iter().enumerate() {
            for (j, b) in shortcuts.iter().enumerate() {
                if i == j || a.logical_key != b.logical_key {
                    continue;
                }
                assert_ne!(a.modifiers, b.modifiers, "{a:?} doppia");
                // A parità di tasto, quella con più modificatori va letta prima.
                let a_more = a.modifiers.contains(b.modifiers);
                if a_more {
                    assert!(i < j, "{a:?} va prima di {b:?}");
                }
            }
        }
    }
}
