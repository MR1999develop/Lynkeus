// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Comandi dei menu con le loro scorciatoie: una sola tabella per la barra dei menu e per
//! la tastiera, così le due cose non divergono.

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::app::{Application, Tab};

/// Un comando raggiungibile dal menu (e, se ha una scorciatoia, dalla tastiera).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    AddFolder,
    AddFile,
    Quit,
    SelectAll,
    SelectNone,
    FindTerms,
    AddTerm,
    Search,
    Cancel,
    ShowResults,
    ShowWarnings,
    ToggleSearchPanel,
    ToggleFilters,
    About,
}

impl Command {
    /// Ordine in cui si leggono le scorciatoie: le più specifiche prima (Ctrl+Maiusc+O
    /// prima di Ctrl+O), perché `consume_shortcut` ignora i modificatori in più.
    pub(crate) const SHORTCUT_ORDER: [Command; 11] = [
        Command::AddFile,
        Command::AddFolder,
        Command::SelectNone,
        Command::SelectAll,
        Command::Quit,
        Command::FindTerms,
        Command::Search,
        Command::Cancel,
        Command::ShowResults,
        Command::ShowWarnings,
        Command::ToggleSearchPanel,
    ];

    /// La scorciatoia, se c'è.
    pub(crate) fn shortcut(self) -> Option<KeyboardShortcut> {
        let ctrl = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        let ctrl_shift = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
        let plain = |key| KeyboardShortcut::new(Modifiers::NONE, key);
        Some(match self {
            Command::AddFolder => ctrl(Key::O),
            Command::AddFile => ctrl_shift(Key::O),
            Command::Quit => ctrl(Key::Q),
            Command::SelectAll => ctrl(Key::A),
            Command::SelectNone => ctrl_shift(Key::A),
            Command::FindTerms => ctrl(Key::F),
            Command::Search => plain(Key::F5),
            Command::Cancel => plain(Key::Escape),
            Command::ShowResults => ctrl(Key::Num1),
            Command::ShowWarnings => ctrl(Key::Num2),
            Command::ToggleSearchPanel => plain(Key::F9),
            Command::AddTerm | Command::ToggleFilters | Command::About => return None,
        })
    }

    /// ID Fluent della voce di menu.
    pub(crate) fn label_id(self) -> &'static str {
        match self {
            Command::AddFolder => "menu-add-folder",
            Command::AddFile => "menu-add-file",
            Command::Quit => "menu-quit",
            Command::SelectAll => "menu-select-all",
            Command::SelectNone => "menu-select-none",
            Command::FindTerms => "menu-find-terms",
            Command::AddTerm => "menu-add-term",
            Command::Search => "menu-search",
            Command::Cancel => "menu-cancel",
            Command::ShowResults => "menu-show-results",
            Command::ShowWarnings => "menu-show-warnings",
            Command::ToggleSearchPanel => "menu-search-panel",
            Command::ToggleFilters => "menu-filters",
            Command::About => "menu-about",
        }
    }

    /// Se il comando si può usare ora; altrimenti l'ID Fluent del motivo, mostrato nel
    /// tooltip della voce disabilitata.
    pub(crate) fn availability(self, app: &Application) -> Result<(), &'static str> {
        let has_rows = !app.view().is_empty();
        match self {
            Command::Search if app.is_searching() => Err("reason-search-running"),
            Command::Cancel if !app.is_searching() => Err("reason-no-search"),
            Command::SelectAll | Command::SelectNone if !has_rows => Err("reason-no-results"),
            Command::SelectAll | Command::SelectNone if app.view_state.tab != Tab::Results => {
                Err("reason-not-results-tab")
            }
            Command::AddTerm if !app.form.can_add_term() => Err("reason-too-many-terms"),
            _ => Ok(()),
        }
    }

    /// Vero se la scorciatoia va lasciata al campo di testo con il focus (Ctrl+A seleziona
    /// il testo del campo, Esc ne annulla la modifica).
    pub(crate) fn yields_to_text_fields(self) -> bool {
        matches!(
            self,
            Command::SelectAll | Command::SelectNone | Command::Cancel
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
