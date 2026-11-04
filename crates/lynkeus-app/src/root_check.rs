// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Verifica delle radici (cartella, file o non trovata) fuori dal thread dell'interfaccia.
//!
//! Leggere i metadati di un percorso può bloccare per decine di secondi (condivisione di
//! rete scollegata, disco che si risveglia): ogni verifica gira in un thread suo, così una
//! verifica lenta non ferma né la finestra né le altre. Le risposte arrivano su un canale
//! che l'app svuota a ogni frame; il `waker` chiede un nuovo frame quando ne arriva una.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, OnceLock};

use lynkeus_engine::Notify;

use crate::form::RootKind;

/// La risposta a una verifica.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RootCheck {
    /// Il percorso verificato, come chiesto.
    pub path: PathBuf,
    /// Che cosa è sul disco.
    pub kind: RootKind,
}

/// Le verifiche chieste e le loro risposte.
pub(crate) struct RootChecks {
    sender: Sender<RootCheck>,
    receiver: Receiver<RootCheck>,
    /// Chi avvisare a ogni risposta; impostato una volta, quando c'è la finestra.
    waker: Arc<OnceLock<Notify>>,
    /// Verifiche partite e non ancora ricevute.
    pending: usize,
}

impl RootChecks {
    /// Nessuna verifica in corso, nessuno da avvisare.
    pub fn new() -> RootChecks {
        let (sender, receiver) = mpsc::channel();
        RootChecks {
            sender,
            receiver,
            waker: Arc::new(OnceLock::new()),
            pending: 0,
        }
    }

    /// Vero se c'è già chi avvisare.
    pub fn has_waker(&self) -> bool {
        self.waker.get().is_some()
    }

    /// Chi avvisare quando arriva una risposta; vale solo la prima chiamata. Le risposte
    /// arrivate prima restano nel canale e si leggono al primo frame.
    pub fn set_waker(&self, waker: Notify) {
        // Una seconda chiamata non cambia nulla: il primo avviso resta valido.
        let _ignored = self.waker.set(waker);
    }

    /// Avvia la verifica di `path` in un thread; falso se il sistema non l'ha creato (la
    /// radice resta allora da verificare).
    pub fn request(&mut self, path: PathBuf) -> bool {
        let sender = self.sender.clone();
        let waker = Arc::clone(&self.waker);
        let spawned = std::thread::Builder::new()
            .name("lynkeus-root-check".to_owned())
            .spawn(move || {
                let kind = RootKind::probe(&path);
                // Se l'app è già chiusa nessuno aspetta la risposta.
                if sender.send(RootCheck { path, kind }).is_ok()
                    && let Some(waker) = waker.get()
                {
                    waker();
                }
            });
        if spawned.is_ok() {
            self.pending += 1;
        }
        spawned.is_ok()
    }

    /// La prossima risposta arrivata, senza attendere.
    pub fn try_recv(&mut self) -> Option<RootCheck> {
        match self.receiver.try_recv() {
            Ok(check) => {
                self.pending = self.pending.saturating_sub(1);
                Some(check)
            }
            // Il mittente originale vive in `self`: il canale non si chiude mai qui.
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }

    /// Vero se qualche verifica non ha ancora risposto.
    pub fn is_pending(&self) -> bool {
        self.pending > 0
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::*;

    fn wait(checks: &mut RootChecks) -> Vec<RootCheck> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut answers = Vec::new();
        while checks.is_pending() {
            assert!(Instant::now() < deadline, "verifiche senza risposta");
            match checks.try_recv() {
                Some(answer) => answers.push(answer),
                None => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        answers
    }

    #[test]
    fn answers_arrive_and_wake_the_window() {
        let mut checks = RootChecks::new();
        let woken = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&woken);
        checks.set_waker(Arc::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        }));
        assert!(checks.has_waker());
        let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let file = here.join("Cargo.toml");
        let missing = std::env::temp_dir().join("lynkeus-non-esiste-7f3a9c");
        for path in [&here, &file, &missing] {
            assert!(checks.request(path.clone()));
        }
        let mut answers = wait(&mut checks);
        answers.sort_by(|a, b| a.path.cmp(&b.path));
        let mut expected = vec![
            RootCheck {
                path: here,
                kind: RootKind::Folder,
            },
            RootCheck {
                path: file,
                kind: RootKind::File,
            },
            RootCheck {
                path: missing,
                kind: RootKind::Missing,
            },
        ];
        expected.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(answers, expected);
        // Ogni thread avvisa subito dopo aver mandato la risposta.
        let deadline = Instant::now() + Duration::from_secs(30);
        while woken.load(Ordering::Relaxed) < 3 {
            assert!(Instant::now() < deadline, "avvisi mancanti");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(woken.load(Ordering::Relaxed), 3);
        assert_eq!(checks.try_recv(), None);
    }
}
