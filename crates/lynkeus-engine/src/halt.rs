// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Perché una ricerca si ferma prima della fine, e il primo motivo condiviso tra i suoi
//! thread. Modulo foglia: lo usano la scansione, la lettura dei file e quella dei documenti.

use std::sync::atomic::{AtomicU8, Ordering};

/// Perché la ricerca si ferma prima della fine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    /// Annullata dal chiamante.
    Cancelled,
    /// Raggiunto il limite dei risultati.
    LimitReached,
    /// Il chiamante ha chiuso la maniglia: non c'è più nessuno a cui consegnare eventi.
    Disconnected,
}

/// Il primo motivo d'arresto, condiviso tra i thread; il canale chiuso prevale su tutto.
#[derive(Debug, Default)]
pub(crate) struct Halt(AtomicU8);

impl Halt {
    const RUNNING: u8 = 0;
    const CANCELLED: u8 = 1;
    const LIMIT_REACHED: u8 = 2;
    const DISCONNECTED: u8 = 3;

    /// Registra `stop`, se è il primo motivo o se è la chiusura del canale.
    pub(crate) fn record(&self, stop: Stop) {
        let code = match stop {
            Stop::Cancelled => Self::CANCELLED,
            Stop::LimitReached => Self::LIMIT_REACHED,
            Stop::Disconnected => {
                self.0.store(Self::DISCONNECTED, Ordering::Relaxed);
                return;
            }
        };
        // Vince il primo; se un altro thread ha già registrato un motivo, va bene così.
        if self
            .0
            .compare_exchange(Self::RUNNING, code, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {}
    }

    /// Il motivo registrato, se c'è.
    pub(crate) fn get(&self) -> Option<Stop> {
        match self.0.load(Ordering::Relaxed) {
            Self::CANCELLED => Some(Stop::Cancelled),
            Self::LIMIT_REACHED => Some(Stop::LimitReached),
            Self::DISCONNECTED => Some(Stop::Disconnected),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halt_keeps_the_first_reason_but_disconnection_wins() {
        let halt = Halt::default();
        assert_eq!(halt.get(), None);
        halt.record(Stop::LimitReached);
        halt.record(Stop::Cancelled);
        assert_eq!(halt.get(), Some(Stop::LimitReached));
        halt.record(Stop::Disconnected);
        assert_eq!(halt.get(), Some(Stop::Disconnected));
    }
}
