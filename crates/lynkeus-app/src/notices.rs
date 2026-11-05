// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! I messaggi per l'utente sopra la tabella ([`Notice`]) e la loro coda ([`Notices`]).
//!
//! Due classi, decise da [`Notice::is_lasting`]:
//!
//! - **momentanei**: l'esito di un gesto (copia, esportazione, apertura, filtri
//!   ripristinati, luoghi oltre il massimo). Ce n'è al più uno, il più recente: il messaggio
//!   successivo, di qualunque classe, lo sostituisce;
//! - **duraturi**: un problema che resta vero anche dopo (cartelle non private, impostazioni
//!   che non si salvano o rimesse ai predefiniti, voce del clic destro, cache che non si
//!   scrive). Restano finché l'utente li chiude: un messaggio arrivato dopo non li nasconde.
//!   Uno nuovo dello stesso tipo prende il posto del precedente.
//!
//! La vista li mostra tutti, dal più recente. La coda ha quindi al più un messaggio per
//! ogni tipo duraturo più uno momentaneo: è limitata per costruzione.

use std::mem::discriminant;
use std::sync::Arc;

use lynkeus_engine::DocPath;

use crate::context_menu::MenuError;
use crate::form::FilterForm;
use crate::launch::EntryFailure;
use crate::settings::SettingsProblem;

/// Un messaggio per l'utente sopra la tabella; quanto resta lo dice [`Notice::is_lasting`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// Il file è un programma o uno script: non si apre.
    Executable {
        /// Il documento (per "Mostra nella cartella").
        doc: Arc<DocPath>,
    },
    /// Aprire una voce d'archivio non è ancora possibile.
    ArchiveEntry,
    /// Il sistema non ha aperto il file.
    OpenFailed {
        /// Il documento, come mostrato.
        file: String,
    },
    /// Il sistema non ha mostrato il file nella cartella.
    RevealFailed {
        /// Il documento, come mostrato.
        file: String,
    },
    /// Righe copiate negli appunti.
    Copied {
        /// Quante.
        rows: usize,
    },
    /// Esportazione riuscita.
    Exported {
        /// Righe scritte.
        rows: usize,
        /// Il file scritto.
        file: String,
    },
    /// Esportazione non riuscita: il file di prima, se c'era, è intatto.
    ExportFailed {
        /// Il file che si voleva scrivere.
        file: String,
    },
    /// Il sistema non ha avviato il thread di lavoro.
    NoThread,
    /// Cache dei documenti svuotata.
    CacheCleared {
        /// MiB liberati (arrotondati per eccesso).
        freed_mib: u64,
    },
    /// La cache non si è svuotata del tutto (file in uso o permessi).
    CacheClearFailed,
    /// Una voce d'archivio non si è estratta per aprirla.
    EntryFailed {
        /// Il documento, come mostrato.
        file: String,
        /// Perché.
        reason: EntryFailure,
    },
    /// La voce del clic destro non si è accesa o spenta.
    ContextMenuFailed(MenuError),
    /// Cartelle dell'app non private (di altri o aperte ad altri): le funzioni che le
    /// usano sono spente per questa sessione.
    PrivateDirsRefused {
        /// Quante cartelle.
        count: usize,
    },
    /// Luoghi non aggiunti: l'elenco ha già il massimo di voci.
    RootsLimit {
        /// Il massimo ([`MAX_ROOTS`](crate::form::MAX_ROOTS)).
        max: usize,
    },
    /// Filtri riportati ai predefiniti; "Annulla" rimette questi.
    FiltersReset {
        /// I filtri di prima.
        previous: Box<FilterForm>,
    },
    /// Le modifiche alle impostazioni non arrivano sul disco.
    SettingsNotSaved(SettingsProblem),
    /// Il file delle impostazioni non era valido: messo da parte, si riparte dai
    /// predefiniti.
    SettingsReplaced,
    /// La cache dei documenti non si scrive (spazio, permessi): le ricerche funzionano, ma
    /// rileggono i documenti ogni volta.
    CacheNotWritten,
}

impl Notice {
    /// Vero per un problema che resta e che l'utente deve leggere: il messaggio resta finché
    /// lo chiude. Falso per l'esito di un gesto, che il messaggio successivo sostituisce.
    pub fn is_lasting(&self) -> bool {
        match self {
            Notice::PrivateDirsRefused { .. }
            | Notice::SettingsNotSaved(_)
            | Notice::SettingsReplaced
            | Notice::ContextMenuFailed(_)
            | Notice::CacheNotWritten => true,
            Notice::Executable { .. }
            | Notice::ArchiveEntry
            | Notice::OpenFailed { .. }
            | Notice::RevealFailed { .. }
            | Notice::Copied { .. }
            | Notice::Exported { .. }
            | Notice::ExportFailed { .. }
            | Notice::NoThread
            | Notice::CacheCleared { .. }
            | Notice::CacheClearFailed
            | Notice::EntryFailed { .. }
            | Notice::RootsLimit { .. }
            | Notice::FiltersReset { .. } => false,
        }
    }
}

/// I messaggi da mostrare, dal più recente, con la regola di [`Notice::is_lasting`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Notices {
    /// Dal più recente; il momentaneo, se c'è, è il primo.
    items: Vec<Notice>,
}

impl Notices {
    /// Mostra `notice` per primo: toglie il momentaneo di prima e, se `notice` è duraturo,
    /// quello dello stesso tipo.
    pub(crate) fn push(&mut self, notice: Notice) {
        let kind = discriminant(&notice);
        let lasting = notice.is_lasting();
        self.items
            .retain(|old| old.is_lasting() && !(lasting && discriminant(old) == kind));
        self.items.insert(0, notice);
    }

    /// Il più recente.
    pub(crate) fn newest(&self) -> Option<&Notice> {
        self.items.first()
    }

    /// Tutti, dal più recente.
    pub(crate) fn all(&self) -> &[Notice] {
        &self.items
    }

    /// Chiude il messaggio in posizione `index` di [`Notices::all`]; fuori dall'elenco non
    /// fa nulla.
    pub(crate) fn dismiss(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
        }
    }

    /// Toglie e restituisce il più recente, se `wanted` lo accetta.
    pub(crate) fn take_newest_if(
        &mut self,
        wanted: impl FnOnce(&Notice) -> bool,
    ) -> Option<Notice> {
        if self.items.first().is_some_and(wanted) {
            Some(self.items.remove(0))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copied(rows: usize) -> Notice {
        Notice::Copied { rows }
    }

    /// Un momentaneo sostituisce il momentaneo di prima.
    #[test]
    fn a_momentary_notice_replaces_the_previous_one() {
        let mut notices = Notices::default();
        notices.push(copied(1));
        notices.push(Notice::NoThread);
        assert_eq!(notices.all(), [Notice::NoThread]);
    }

    /// I duraturi restano sotto i messaggi arrivati dopo, anche momentanei, finché l'utente
    /// li chiude: all'avvio l'avviso sulle cartelle non nasconde quello sulle impostazioni.
    #[test]
    fn lasting_notices_stay_under_newer_ones() {
        let mut notices = Notices::default();
        notices.push(Notice::SettingsReplaced);
        notices.push(Notice::PrivateDirsRefused { count: 2 });
        notices.push(copied(3));
        assert_eq!(
            notices.all(),
            [
                copied(3),
                Notice::PrivateDirsRefused { count: 2 },
                Notice::SettingsReplaced,
            ]
        );
        // Un messaggio nuovo toglie solo il momentaneo.
        notices.push(Notice::CacheNotWritten);
        assert_eq!(notices.newest(), Some(&Notice::CacheNotWritten));
        assert_eq!(notices.all().len(), 3);
        notices.dismiss(1);
        assert_eq!(
            notices.all(),
            [Notice::CacheNotWritten, Notice::SettingsReplaced]
        );
        notices.dismiss(7);
        assert_eq!(notices.all().len(), 2);
    }

    /// Un duraturo nuovo prende il posto di quello dello stesso tipo: la coda resta corta.
    #[test]
    fn a_lasting_notice_replaces_one_of_its_kind() {
        let mut notices = Notices::default();
        notices.push(Notice::SettingsNotSaved(SettingsProblem::NewerVersion));
        notices.push(Notice::SettingsReplaced);
        notices.push(Notice::SettingsNotSaved(SettingsProblem::WriteFailed));
        assert_eq!(
            notices.all(),
            [
                Notice::SettingsNotSaved(SettingsProblem::WriteFailed),
                Notice::SettingsReplaced,
            ]
        );
    }

    #[test]
    fn only_the_newest_is_taken() {
        let mut notices = Notices::default();
        notices.push(Notice::SettingsReplaced);
        assert_eq!(notices.take_newest_if(|n| !n.is_lasting()), None);
        notices.push(copied(1));
        assert_eq!(notices.take_newest_if(|n| !n.is_lasting()), Some(copied(1)));
        assert_eq!(notices.all(), [Notice::SettingsReplaced]);
    }
}
