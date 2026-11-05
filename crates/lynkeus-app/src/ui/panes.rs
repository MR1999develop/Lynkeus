// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Le zone della finestra tra cui F6 e Maiusc+F6 spostano il focus della tastiera
//! (egui non lo fa da sé): pannello di ricerca, tabella dei risultati, anteprima.
//!
//! Ogni zona, quando si disegna, registra il suo rettangolo e il controllo che riceve il
//! focus quando ci si arriva con F6; il comando del frame successivo legge le zone
//! registrate nel frame precedente. L'ordine è quello di lettura.

use egui::{Context, Id, Rect};

/// Una zona della finestra raggiungibile con F6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Pane {
    /// Il pannello di ricerca a sinistra (focus sul primo termine).
    Search,
    /// La tabella dei risultati (focus sulla riga del cursore).
    Results,
    /// L'anteprima sotto la tabella (focus sul primo comando).
    Preview,
}

impl Pane {
    /// Tutte le zone, nell'ordine di lettura.
    const ALL: [Pane; 3] = [Pane::Search, Pane::Results, Pane::Preview];
}

/// Dove sta una zona e quale controllo prende il focus quando ci si arriva.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PaneEntry {
    rect: Rect,
    target: Id,
}

/// Registra che `pane` è visibile in `rect` e che con F6 il focus va a `target`.
pub(super) fn register(ctx: &Context, pane: Pane, rect: Rect, target: Id) {
    ctx.data_mut(|data| data.insert_temp(entry_id(pane), PaneEntry { rect, target }));
}

/// Dimentica le zone registrate (all'inizio del disegno di un frame, dopo averle lette).
pub(super) fn forget(ctx: &Context) {
    ctx.data_mut(|data| {
        for pane in Pane::ALL {
            data.remove::<PaneEntry>(entry_id(pane));
        }
    });
}

/// Sposta il focus alla zona successiva (o precedente, con `backwards`) rispetto a quella
/// che contiene il controllo con il focus.
pub(super) fn cycle_focus(ctx: &Context, backwards: bool) {
    let entries: Vec<(Pane, PaneEntry)> = ctx.data(|data| {
        Pane::ALL
            .into_iter()
            .filter_map(|pane| Some((pane, data.get_temp::<PaneEntry>(entry_id(pane))?)))
            .collect()
    });
    let focused_rect = ctx
        .memory(egui::Memory::focused)
        .and_then(|id| ctx.read_response(id))
        .map(|response| response.rect);
    // Le zone possono contenersi (l'anteprima sta dentro il pannello centrale): vale la
    // più piccola che contiene il controllo.
    let current = focused_rect.and_then(|rect| {
        entries
            .iter()
            .filter(|(_, entry)| entry.rect.contains(rect.center()))
            .min_by(|(_, a), (_, b)| a.rect.area().total_cmp(&b.rect.area()))
            .map(|(pane, _)| *pane)
    });
    let present: Vec<Pane> = entries.iter().map(|(pane, _)| *pane).collect();
    let Some(next) = next_pane(&present, current, backwards) else {
        return;
    };
    if let Some((_, entry)) = entries.iter().find(|(pane, _)| *pane == next) {
        ctx.memory_mut(|memory| memory.request_focus(entry.target));
    }
}

/// La zona dopo `current` tra quelle presenti (in ordine di lettura), ricominciando dalla
/// prima; senza una zona attuale la prima (l'ultima con `backwards`).
fn next_pane(present: &[Pane], current: Option<Pane>, backwards: bool) -> Option<Pane> {
    let count = present.len();
    let index = match current.and_then(|pane| present.iter().position(|p| *p == pane)) {
        Some(index) if backwards => (index + count - 1) % count,
        Some(index) => (index + 1) % count,
        None if backwards => count.checked_sub(1)?,
        None => 0,
    };
    present.get(index).copied()
}

fn entry_id(pane: Pane) -> Id {
    Id::unique(("lynkeus-pane", pane))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panes_cycle_in_reading_order() {
        let all = Pane::ALL;
        assert_eq!(next_pane(&all, None, false), Some(Pane::Search));
        assert_eq!(next_pane(&all, None, true), Some(Pane::Preview));
        assert_eq!(
            next_pane(&all, Some(Pane::Search), false),
            Some(Pane::Results)
        );
        assert_eq!(
            next_pane(&all, Some(Pane::Preview), false),
            Some(Pane::Search)
        );
        assert_eq!(
            next_pane(&all, Some(Pane::Search), true),
            Some(Pane::Preview)
        );
        // Zona attuale non più presente (anteprima nascosta): si parte dalla prima.
        let two = [Pane::Search, Pane::Results];
        assert_eq!(
            next_pane(&two, Some(Pane::Preview), false),
            Some(Pane::Search)
        );
        assert_eq!(next_pane(&[], None, false), None);
        assert_eq!(next_pane(&[], None, true), None);
    }
}
