// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Misure dell'interfaccia del livello 3 del piano di test (P-06…P-09 di
//! `docs/piano-di-test.md`), con 200.000 risultati.
//!
//! Sono test `#[ignore]`: li lancia `cargo xtask test-plan`, uno per processo e in release,
//! oppure a mano:
//!
//! ```text
//! cargo test --release -p lynkeus-app --test prestazioni_interfaccia <nome> -- --ignored --exact --nocapture
//! ```
//!
//! Ognuno stampa la riga `MISURA` del modulo `misura` (lo stesso file delle misure del
//! motore); le soglie le confronta xtask.

#[path = "../../lynkeus-engine/tests/misura/mod.rs"]
mod misura;

use std::hint::black_box;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use egui::{Key, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use lynkeus_app::export;
use lynkeus_app::form::{RootAdded, RootKind};
use lynkeus_app::results::{ClickKind, ResultsView, SortColumn, SortSpec, Step};
use lynkeus_app::{Application, MAX_CLIPBOARD_BYTES, Notice, Pump, Settings, setup_context};
use lynkeus_core::{Location, Snippet, TermSet};
use lynkeus_engine::{DocPath, Hit, Notify};
use misura::Distribution;

/// Ripetizioni a caldo dopo quella scartata (metodo comune del piano).
const REPETITIONS: usize = 15;
/// Righe di risultato delle misure: il limite predefinito dei risultati.
const ROWS: usize = 200_000;
/// Tempo massimo di una ricerca o di una copia nelle misure.
const DEADLINE: Duration = Duration::from_secs(120);
/// Intervallo tra due frame simulati mentre un thread lavora (circa 60 al secondo).
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// Frame massimi perché la finestra si fermi dopo un tasto: lo scorrimento animato fino
/// alla riga del cursore ne chiede alcuni in più dei 4 predefiniti di kittest.
const MAX_SETTLE_FRAMES: u64 = 64;

/// Le colonne ordinabili.
const COLUMNS: [SortColumn; 5] = [
    SortColumn::File,
    SortColumn::Position,
    SortColumn::Word,
    SortColumn::Term,
    SortColumn::Context,
];

/// P-06: ordinamento finale di 200.000 righe per ogni colonna (15 ripetizioni per colonna,
/// tutte nella distribuzione); `batch_max_ms` è il lotto più lento inserito durante la
/// ricerca, a lotti di 5000 righe.
#[test]
#[ignore = "misura P-06 del piano di test, lanciata da cargo xtask test-plan"]
fn p06_sort_200k() {
    let hits = many_hits();
    let roots = roots();
    let mut finish_times = Vec::new();
    let mut slowest_batch = Duration::ZERO;
    for column in COLUMNS {
        finish_times.extend(misura::repeat(REPETITIONS, || {
            let mut view = ResultsView::new(Some(SortSpec::ascending(column)));
            let ((), took) = misura::timed(|| view.finish(&hits, &roots));
            assert_eq!(view.len(), hits.len());
            took
        }));
        let mut streaming = ResultsView::new(Some(SortSpec::ascending(column)));
        for end in (5_000..=hits.len()).step_by(5_000) {
            let ((), took) = misura::timed(|| streaming.extend_to(&hits[..end], &roots));
            slowest_batch = slowest_batch.max(took);
        }
    }
    misura::print_line(
        "P-06",
        &Distribution::new(&finish_times),
        &[
            ("rows", ROWS.to_string()),
            ("columns", COLUMNS.len().to_string()),
            ("batch_max_ms", misura::duration_ms(slowest_batch)),
        ],
    );
}

/// P-07: tempo per chiamata di `step_target` (i quattro passi dell'anteprima), righe per
/// file e per parola, cursore all'inizio, a metà e in fondo.
#[test]
#[ignore = "misura P-07 del piano di test, lanciata da cargo xtask test-plan"]
fn p07_step_target_200k() {
    const CALLS: u32 = 50;
    let steps = [
        Step::NextInFile,
        Step::PreviousInFile,
        Step::NextFile,
        Step::PreviousFile,
    ];
    let hits = many_hits();
    let mut per_call = Vec::new();
    for sort in [None, Some(SortSpec::ascending(SortColumn::Word))] {
        let mut view = ResultsView::new(sort);
        view.finish(&hits, &roots());
        for position in [0, hits.len() / 2, hits.len() - 1] {
            view.jump_to(position);
            per_call.extend(misura::repeat(REPETITIONS, || {
                let ((), took) = misura::timed(|| {
                    for _ in 0..CALLS {
                        for step in steps {
                            black_box(view.step_target(black_box(step)));
                        }
                    }
                });
                took / (CALLS * 4)
            }));
        }
    }
    misura::print_line(
        "P-07",
        &Distribution::new(&per_call),
        &[("rows", ROWS.to_string())],
    );
}

/// P-08: copia negli appunti di sistema con 200.000 risultati. Ogni ripetizione:
///
/// 1. copia le prime righe, tante quante ne stanno in [`MAX_CLIPBOARD_BYTES`]: il testo
///    arriva alla vista, che lo consegna a egui e agli appunti di sistema (come eframe, con
///    arboard, sul thread dell'interfaccia). Il valore misurato è il blocco più lungo del
///    thread dell'interfaccia: l'avvio della copia, un frame mentre il thread di copia
///    lavora o la consegna agli appunti; `copy_max_ms` è la copia completa;
/// 2. copia tutte le 200.000 righe, troppe per gli appunti: il rifiuto arriva con il
///    messaggio che propone Esporta e gli appunti restano com'erano
///    (`reject_block_max_ms`: blocco più lungo del thread dell'interfaccia; `reject_max_ms`:
///    dall'avvio al messaggio).
///
/// Scrive negli appunti di chi la esegue: `cargo xtask test-plan` la lancia solo con
/// `--clipboard`. Il testo che c'era prima torna negli appunti alla fine, anche se la misura
/// fallisce ([`SavedClipboard`]).
#[test]
#[ignore = "misura P-08 del piano di test, lanciata da cargo xtask test-plan --clipboard"]
fn p08_copy_200k() {
    // Per primo: il suo `Drop` viene per ultimo, dopo ogni altro valore della misura.
    let mut clipboard = SavedClipboard::save();
    let (mut app, _dir) = searched_app();
    assert_eq!(app.results().hits.len(), ROWS);
    let expected = text_up_to_the_limit(&app);
    let rows_at_limit = expected.lines().count();
    let mut copy_max = Duration::ZERO;
    let mut reject_block_max = Duration::ZERO;
    let mut reject_max = Duration::ZERO;
    let times = misura::repeat(REPETITIONS, || {
        app.view_mut().click_hit(0, ClickKind::Replace);
        app.view_mut()
            .click_hit(rows_at_limit - 1, ClickKind::Range);
        let (copy, text) = copy_and_wait(&mut app);
        let text = text.expect("il testo al limite va negli appunti");
        assert!(text == expected, "testo copiato diverso dalle righe scelte");
        assert_eq!(
            app.notice(),
            Some(&Notice::Copied {
                rows: rows_at_limit
            })
        );
        let handed = clipboard.hand_over(&text);
        assert_eq!(
            clipboard.text_len(),
            Some(expected.len()),
            "testo negli appunti"
        );
        copy_max = copy_max.max(copy.total);

        clipboard.put(UNTOUCHED);
        app.view_mut().select_all();
        let (reject, text) = copy_and_wait(&mut app);
        assert!(text.is_none(), "troppo per gli appunti");
        assert_eq!(app.notice(), Some(&Notice::CopyTooLarge { rows: ROWS }));
        assert!(
            clipboard.holds(UNTOUCHED),
            "il rifiuto ha toccato gli appunti"
        );
        reject_block_max = reject_block_max.max(reject.longest_block);
        reject_max = reject_max.max(reject.total);

        copy.longest_block.max(handed)
    });
    misura::print_line(
        "P-08",
        &Distribution::new(&times),
        &[
            ("rows", ROWS.to_string()),
            ("limit_rows", rows_at_limit.to_string()),
            ("limit_bytes", expected.len().to_string()),
            ("copy_max_ms", misura::duration_ms(copy_max)),
            ("reject_block_max_ms", misura::duration_ms(reject_block_max)),
            ("reject_max_ms", misura::duration_ms(reject_max)),
        ],
    );
}

/// Il testo che P-08 mette negli appunti mentre prova il rifiuto: deve restare lì.
const UNTOUCHED: &str = "P-08: appunti non toccati dal rifiuto";

/// Il testo TSV delle prime righe della tabella che stanno in [`MAX_CLIPBOARD_BYTES`]
/// (righe intere, nell'ordine di arrivo come nella vista senza ordinamento).
fn text_up_to_the_limit(app: &Application) -> String {
    let hits = &app.results().hits;
    let table = export::results_table(app.i18n(), hits, &app.results().terms, 0..hits.len());
    let mut all = export::to_tsv(&table.rows);
    assert!(
        all.len() > MAX_CLIPBOARD_BYTES,
        "le 200.000 righe superano il limite"
    );
    let cut = all.as_bytes()[..=MAX_CLIPBOARD_BYTES]
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |newline| newline + 1);
    all.truncate(cut);
    all
}

/// I tempi di una copia sul thread dell'interfaccia.
struct CopyTimes {
    /// Il blocco più lungo: l'avvio o un frame.
    longest_block: Duration,
    /// Dall'avvio all'esito (testo pronto o rifiuto).
    total: Duration,
}

/// Copia la selezione come la finestra: l'avvio, poi un frame all'avviso del thread di
/// copia o dopo un intervallo, finché la copia non finisce. Il testo pronto per gli
/// appunti, se c'è.
fn copy_and_wait(app: &mut Application) -> (CopyTimes, Option<String>) {
    let (tx, rx) = mpsc::channel::<()>();
    let notify: Notify = Arc::new(move || {
        // La copia può essere già consegnata: un avviso perso non conta.
        if tx.send(()).is_err() {}
    });
    let started = Instant::now();
    let ((), mut longest) = misura::timed(|| app.copy_selection(&notify));
    assert!(app.is_copying(), "copia avviata");
    loop {
        let ((), frame) = misura::timed(|| app.pump_actions());
        longest = longest.max(frame);
        if !app.is_copying() {
            break;
        }
        assert!(started.elapsed() < DEADLINE, "copia non finita");
        // Il prossimo frame: all'avviso del thread di copia o dopo un intervallo.
        if rx.recv_timeout(FRAME_INTERVAL).is_err() {}
    }
    let total = started.elapsed();
    let times = CopyTimes {
        longest_block: longest,
        total,
    };
    (times, app.take_clipboard_text())
}

/// Gli appunti di sistema durante P-08. Ricorda il testo che c'era prima e lo rimette nel
/// `Drop`, anche se la misura fallisce; se prima non c'era testo, alla fine li svuota. Il
/// testo dell'utente resta in memoria e basta: non si stampa mai.
struct SavedClipboard {
    clipboard: arboard::Clipboard,
    before: Option<String>,
}

impl SavedClipboard {
    /// Legge gli appunti; se non si possono aprire o leggere la misura si ferma prima di
    /// toccarli.
    fn save() -> SavedClipboard {
        let mut clipboard = match arboard::Clipboard::new() {
            Ok(clipboard) => clipboard,
            Err(error) => panic!("appunti di sistema non disponibili: {error}"),
        };
        let before = match clipboard.get_text() {
            Ok(text) => Some(text),
            Err(arboard::Error::ContentNotAvailable) => None,
            Err(error) => panic!("appunti non leggibili, misura non eseguita: {error}"),
        };
        SavedClipboard { clipboard, before }
    }

    /// Consegna `text` come la finestra: a egui, poi (eframe, a fine frame) agli appunti di
    /// sistema con arboard. Il tempo sul thread dell'interfaccia.
    fn hand_over(&mut self, text: &str) -> Duration {
        let ctx = egui::Context::default();
        let for_egui = text.to_owned();
        let started = Instant::now();
        ctx.copy_text(for_egui);
        let written = self.clipboard.set_text(text);
        let took = started.elapsed();
        assert!(written.is_ok(), "scrittura negli appunti: {written:?}");
        took
    }

    /// Mette negli appunti un testo breve.
    fn put(&mut self, text: &str) {
        let written = self.clipboard.set_text(text);
        assert!(written.is_ok(), "scrittura negli appunti: {written:?}");
    }

    /// La lunghezza del testo negli appunti, se c'è testo.
    fn text_len(&mut self) -> Option<usize> {
        self.clipboard.get_text().ok().map(|text| text.len())
    }

    /// Vero se negli appunti c'è proprio `text`.
    fn holds(&mut self, text: &str) -> bool {
        self.clipboard.get_text().is_ok_and(|now| now == text)
    }
}

impl Drop for SavedClipboard {
    fn drop(&mut self) {
        let restored = match self.before.take() {
            Some(text) => self.clipboard.set_text(text),
            None => self.clipboard.clear(),
        };
        if let Err(error) = restored {
            eprintln!("P-08: appunti non ripristinati: {error}");
        }
    }
}

/// P-09: tempo del frame che riceve la freccia giù con 200.000 risultati (la risposta a
/// un tasto); `first_frame_ms` è l'apertura della finestra con i risultati già presenti,
/// fino a una finestra stabile (più frame).
#[test]
#[ignore = "misura P-09 del piano di test, lanciata da cargo xtask test-plan"]
fn p09_key_response_200k() {
    let (app, _dir) = searched_app();
    let (mut harness, first_frame) = misura::timed(|| {
        Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_max_steps(MAX_SETTLE_FRAMES)
            .build_eframe(move |cc| {
                setup_context(&cc.egui_ctx, &app);
                app
            })
    });
    harness
        .query_all_by_role_and_label(Role::Label, "ar")
        .next()
        .expect("una riga con la parola \"ar\"")
        .click();
    harness.run();
    assert!(
        harness.state().view().cursor_position().is_some(),
        "cursore su una riga"
    );
    let times = misura::repeat(REPETITIONS, || {
        let before = harness.state().view().cursor_position();
        harness.key_down(Key::ArrowDown);
        let ((), took) = misura::timed(|| harness.step());
        harness.key_up(Key::ArrowDown);
        harness.run();
        assert_ne!(
            harness.state().view().cursor_position(),
            before,
            "la freccia sposta il cursore"
        );
        took
    });
    misura::print_line(
        "P-09",
        &Distribution::new(&times),
        &[
            ("rows", ROWS.to_string()),
            ("first_frame_ms", misura::duration_ms(first_frame)),
        ],
    );
}

/// L'app dopo una ricerca di "ar" in un file di [`ROWS`] righe "ar", con tutti gli eventi
/// letti; la cartella temporanea vive quanto il valore restituito.
fn searched_app() -> (Application, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("cartella temporanea");
    let file = dir.path().join("parole.txt");
    std::fs::write(&file, "ar\n".repeat(ROWS)).expect("file scritto");
    let mut app = Application::build(Settings::default()).expect("app");
    assert_eq!(app.form.add_root(file, RootKind::File), RootAdded::Added);
    app.set_term("ar");
    let (tx, rx) = mpsc::channel::<()>();
    let notify: Notify = Arc::new(move || {
        // La ricerca può essere già letta: un avviso perso non conta.
        if tx.send(()).is_err() {}
    });
    app.start_search(notify).expect("ricerca avviata");
    let deadline = Instant::now() + DEADLINE;
    while app.pump_events(usize::MAX) != Pump::Ended {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "ricerca non finita entro {DEADLINE:?}");
        // Il segnale può arrivare prima o dopo la lettura: si rilegge comunque.
        if rx.recv_timeout(left).is_err() {}
    }
    (app, dir)
}

/// 200.000 righe su 2000 documenti in 37 cartelle, con parole diverse: come una ricerca
/// grande su un albero vero.
fn many_hits() -> Vec<Hit> {
    let docs: Vec<Arc<DocPath>> = (0..2_000)
        .map(|f| {
            let path = PathBuf::from("radice")
                .join(format!("cartella {}", f % 37))
                .join("sotto cartella con un nome lungo")
                .join(format!("Documento numero {f}.txt"));
            Arc::new(DocPath::file(path))
        })
        .collect();
    (0..ROWS)
        .map(|i| {
            let doc = &docs[(i * 7919) % docs.len()];
            let line = u32::try_from(i % 5000).expect("riga entro u32");
            hit(doc, line, &format!("parola{}", i % 977))
        })
        .collect()
}

fn hit(doc: &Arc<DocPath>, line: u32, word: &str) -> Hit {
    Hit {
        doc: Arc::clone(doc),
        root: 0,
        location: Location::Line(line),
        word: word.to_owned(),
        word_truncated_start: false,
        word_truncated_end: false,
        snippet: Snippet {
            text: word.to_owned(),
            highlights: Vec::new(),
            truncated_start: false,
            truncated_end: false,
        },
        terms: TermSet::single(0),
    }
}

fn roots() -> Vec<PathBuf> {
    vec![PathBuf::from("radice")]
}
