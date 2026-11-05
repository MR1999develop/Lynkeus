// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Test dell'interfaccia: l'app vera (`eframe::App`) dentro `egui_kittest`, guidata come la
//! guiderebbe un lettore di schermo, cercando i controlli per ruolo e nome accessibile.
//! I nomi tradotti si chiedono al traduttore dell'app, mai scritti nel test; i dati
//! ("l'aragosta") vengono dai file di prova.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::accesskit::Live;
use egui::{Color32, Key, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use fluent_templates::fluent_bundle::FluentValue;
use lynkeus_app::i18n::{I18n, Language};
use lynkeus_app::preview::Loaded;
use lynkeus_app::settings::{ExtensionMode, FilterSettings, LayoutSettings};
use lynkeus_app::theme::SystemContrast;
use lynkeus_app::{Application, Notice, PreviewState, Settings, Tab, setup_context};

/// Tempo massimo per una ricerca nei file di prova.
const SEARCH_DEADLINE: Duration = Duration::from_secs(60);
/// Pausa tra due frame mentre il worker lavora: la ricerca gira su un altro thread.
const FRAME_PAUSE: Duration = Duration::from_millis(5);

fn demo_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/demo")
}

/// Un file trascinato nella finestra.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        Err("non serve".to_owned())
    }
}

fn harness(language: Language) -> (Harness<'static, Application>, I18n) {
    harness_with(Settings {
        language: Some(language),
        ..Settings::default()
    })
}

fn harness_with(settings: Settings) -> (Harness<'static, Application>, I18n) {
    let app = Application::build(settings).expect("app costruita");
    let i18n = app.i18n().clone();
    let harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(move |cc| {
            setup_context(&cc.egui_ctx, &app);
            app
        });
    (harness, i18n)
}

/// Argomento `$number` dei nomi delle righe dei termini.
fn number(n: usize) -> [(&'static str, FluentValue<'static>); 1] {
    [("number", n.into())]
}

#[test]
fn ar_finds_aragosta_through_the_window() {
    let (mut harness, i18n) = harness(Language::Italian);
    search_ar_in_demo(&mut harness, &i18n);

    // La riga ha anche file e posizione.
    let file = std::path::Path::new("ricette").join("mare.txt");
    let file = file.display().to_string();
    assert!(
        harness
            .query_all_by_role_and_label(Role::Label, &file)
            .next()
            .is_some(),
        "percorso relativo alla radice"
    );
    let line_12 = i18n.tr_args("location-line", &[("line", 12.into())]);
    assert!(
        harness
            .query_all_by_role_and_label(Role::Label, &line_12)
            .next()
            .is_some(),
        "posizione della riga 12"
    );
    assert_eq!(harness.state().view_state.tab, Tab::Results);

    // Clic sulla riga: selezionata.
    harness
        .get_by_role_and_label(Role::Label, "l'aragosta")
        .click();
    harness.run();
    assert_eq!(harness.state().view().selected_count(), 1);
}

#[test]
fn clicking_aragosta_opens_the_preview_and_ctrl_c_copies_the_row() {
    let (mut harness, i18n) = harness(Language::Italian);
    search_ar_in_demo(&mut harness, &i18n);
    assert!(harness.state().layout.preview_visible, "anteprima visibile");
    harness
        .get_by_role_and_label(Role::Label, "l'aragosta")
        .click();

    // L'anteprima si legge in un thread: si aspetta che sia pronta.
    let deadline = Instant::now() + SEARCH_DEADLINE;
    loop {
        harness.step();
        if matches!(harness.state().preview(), PreviewState::Ready(..)) {
            break;
        }
        assert!(Instant::now() < deadline, "anteprima non pronta");
        std::thread::sleep(FRAME_PAUSE);
    }
    harness.run();

    let PreviewState::Ready(_, Loaded::Units(preview)) = harness.state().preview().clone() else {
        panic!("attese le righe del file: {:?}", harness.state().preview());
    };
    let focus = &preview.units[preview.focus];
    assert_eq!(focus.text, "Cuocere l'aragosta per 10 minuti.");
    let marked: Vec<&str> = focus
        .highlights
        .iter()
        .map(|range| &focus.text[range.clone()])
        .collect();
    assert_eq!(marked, ["ar"]);
    // Righe vicine (±20): la prima e l'ultima del file, che non contengono "ar".
    for line in ["Ricetta del pesce al forno", "Buon appetito a tutti!"] {
        assert!(
            harness.query_by_label(line).is_some(),
            "{line:?} nell'anteprima"
        );
    }
    let focus_label = format!(
        "› {}",
        i18n.tr_args("location-line", &[("line", 12.into())])
    );
    assert!(
        harness.query_by_label(&focus_label).is_some(),
        "la riga del risultato è segnata anche senza colore"
    );

    // I bottoni dell'anteprima hanno il nome completo del comando (C-13).
    for id in ["menu-previous-file", "menu-next-file", "menu-next-in-file"] {
        harness.get_by_role_and_label(Role::Button, &i18n.tr(id));
    }

    // Ctrl+C (per eframe un evento "copia"): la riga negli appunti come TSV, preparata in
    // un thread.
    harness.input_mut().events.push(egui::Event::Copy);
    step_until(&mut harness, "conferma della copia", |app| {
        app.notice() == Some(&Notice::Copied { rows: 1 })
    });
    harness.run();
    // Il messaggio è una regione "live": il lettore di schermo lo annuncia (C-P3).
    let copied = i18n.tr_args("notice-copied", &[("count", 1.into())]);
    let notice = harness.get_by_label(&copied);
    assert_eq!(notice.accesskit_node().live(), Live::Polite);
}

/// Fa frame (senza attese tra un frame e l'altro oltre a [`FRAME_PAUSE`]) finché `done`
/// non è vero, al massimo per [`SEARCH_DEADLINE`].
fn step_until(
    harness: &mut Harness<'static, Application>,
    what: &str,
    done: impl Fn(&Application) -> bool,
) {
    let deadline = Instant::now() + SEARCH_DEADLINE;
    while !done(harness.state()) {
        assert!(
            Instant::now() < deadline,
            "{what}: non avvenuto entro {SEARCH_DEADLINE:?}"
        );
        harness.step();
        std::thread::sleep(FRAME_PAUSE);
    }
}

/// Una cartella temporanea con `name`, un file di `lines` righe "ar": tante parole
/// trovate, che l'app riceve in molti frame.
fn many_words(name: &str, lines: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("cartella temporanea");
    std::fs::write(dir.path().join(name), "ar\n".repeat(lines)).expect("file scritto");
    dir
}

/// Aggiunge `root`, scrive `term` e preme Cerca, senza aspettare la fine.
fn start_search(harness: &mut Harness<'static, Application>, i18n: &I18n, root: &Path, term: &str) {
    harness.run();
    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(Dropped(root.to_owned())));
    harness.run();
    let term_name = i18n.tr_args("form-term-name", &number(1));
    let field = harness.get_by_role_and_label(Role::TextInput, &term_name);
    field.focus();
    field.type_text(term);
    harness.run();
    harness
        .get_by_role_and_label(Role::Button, &i18n.tr("action-search"))
        .click();
}

/// `count` chiamate a `step` (ognuna fa un frame per ogni evento in coda).
fn steps(harness: &mut Harness<'static, Application>, count: usize) {
    for _ in 0..count {
        harness.step();
    }
}

/// Il nodo (riga della tabella) con il focus della tastiera, se c'è.
fn focused_label(harness: &Harness<'static, Application>) -> Option<String> {
    harness
        .root()
        .children_recursive()
        .find(egui_kittest::Node::is_focused)
        .and_then(|node| node.accesskit_node().label())
}

/// T-C01 (C-01, C-P2): con un menu aperto durante una ricerca, Esc chiude il menu e la
/// ricerca continua; Invio non apre il risultato corrente.
#[test]
fn esc_closes_menu_while_searching() {
    // Un file "programma" con molte parole: la ricerca dura molti frame (5.000 righe per
    // frame) e aprirlo darebbe solo un messaggio, mai un programma avviato.
    // Oltre il limite dei risultati: 200.000 righe, almeno 40 frame per riceverle.
    let dir = many_words("parole.cmd", 250_000);
    let (mut harness, i18n) = harness(Language::Italian);
    start_search(&mut harness, &i18n, dir.path(), "ar");
    step_until(&mut harness, "prime righe", |app| !app.view().is_empty());
    assert!(harness.state().is_searching());
    harness.state_mut().view_mut().jump_to(0);

    let menu_item = i18n.tr("menu-cancel");
    // La voce del menu ha nel nome anche la scorciatoia ("Annulla la ricerca Esc").
    let menu_open = |harness: &Harness<'static, Application>| {
        harness.root().children_recursive().any(|node| {
            node.accesskit_node()
                .label()
                .is_some_and(|label| label.contains(&menu_item))
        })
    };
    harness.get_by_label(&i18n.tr("menu-search-menu")).click();
    steps(&mut harness, 3);
    assert!(menu_open(&harness), "menu aperto");

    harness.key_press(Key::Escape);
    steps(&mut harness, 3);
    assert!(!menu_open(&harness), "menu chiuso");
    assert!(harness.state().is_searching(), "la ricerca continua");

    harness.get_by_label(&i18n.tr("menu-search-menu")).click();
    steps(&mut harness, 3);
    assert!(menu_open(&harness), "menu aperto di nuovo");
    harness.key_press(Key::Enter);
    steps(&mut harness, 3);
    assert_eq!(
        harness.state().notice(),
        None,
        "Invio non apre il risultato"
    );
    assert!(harness.state().is_searching(), "la ricerca continua");

    harness.state_mut().cancel_search();
    step_until(&mut harness, "fine della ricerca", |app| {
        !app.is_searching()
    });
}

/// T-C02 (C-04): con "Solo queste" e nessuna estensione salvata, i Filtri chiusi: F5 apre
/// i Filtri, mostra l'errore e porta il focus sul campo.
#[test]
fn form_error_opens_filters() {
    let (mut harness, i18n) = harness_with(Settings {
        language: Some(Language::Italian),
        roots: vec![demo_dir()],
        terms: vec!["ar".to_owned()],
        filters: FilterSettings {
            extension_mode: ExtensionMode::Include,
            ..FilterSettings::default()
        },
        ..Settings::default()
    });
    harness.run();
    let extensions = i18n.tr("filters-extensions");
    assert!(
        harness
            .query_by_role_and_label(Role::TextInput, &extensions)
            .is_none(),
        "Filtri chiusi"
    );
    harness.key_press(Key::F5);
    harness.run();
    assert!(!harness.state().is_searching());
    harness.get_by_label(&i18n.tr("error-extensions-missing"));
    let field = harness.get_by_role_and_label(Role::TextInput, &extensions);
    assert!(field.is_focused(), "focus sul campo delle estensioni");
    harness.get_by_label(&i18n.tr("error-form-invalid"));
}

/// T-C03 (C-04): con il pannello di ricerca nascosto (F9) e un termine non valido, F5 fa
/// ricomparire il pannello con il focus sul termine e l'errore sotto.
#[test]
fn form_error_with_hidden_panel() {
    let (mut harness, i18n) = harness_with(Settings {
        language: Some(Language::Italian),
        roots: vec![demo_dir()],
        terms: vec!["**".to_owned()],
        layout: LayoutSettings {
            search_panel_visible: false,
            ..LayoutSettings::default()
        },
        ..Settings::default()
    });
    harness.run();
    let term_name = i18n.tr_args("form-term-name", &number(1));
    assert!(
        harness.query_by_label(&term_name).is_none(),
        "pannello nascosto"
    );
    harness.key_press(Key::F5);
    harness.run();
    let field = harness.get_by_role_and_label(Role::TextInput, &term_name);
    assert!(field.is_focused(), "focus sul termine");
    harness.get_by_label(&i18n.tr("error-term-only-wildcards"));
}

/// T-C04 (C-03, C-P1): la riga del cursore è un nodo "riga" selezionato con il focus; la
/// freccia giù sposta selezione e focus alla riga dopo.
#[test]
fn row_selection_is_exposed() {
    let (mut harness, i18n) = harness(Language::Italian);
    search_ar_in_demo(&mut harness, &i18n);
    harness
        .get_by_role_and_label(Role::Label, "l'aragosta")
        .click();
    harness.run();
    let selected = |harness: &Harness<'static, Application>| -> Vec<(String, bool)> {
        harness
            .get_all_by_role(Role::Row)
            .filter(|row| row.accesskit_node().is_selected() == Some(true))
            .map(|row| {
                let label = row.accesskit_node().label().unwrap_or_default();
                (label, row.is_focused())
            })
            .collect()
    };
    let first = selected(&harness);
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(first[0].0.contains("l'aragosta"), "{first:?}");
    assert!(first[0].1, "la riga ha il focus");

    harness.key_press(Key::ArrowDown);
    harness.run();
    let second = selected(&harness);
    assert_eq!(second.len(), 1, "{second:?}");
    assert_ne!(second[0].0, first[0].0, "un'altra riga");
    assert!(second[0].1, "il focus segue il cursore");
    assert_eq!(focused_label(&harness), Some(second[0].0.clone()));

    // Le celle non prendono il focus e la tabella è un solo passo del Tab: dopo la riga il
    // focus non va a un'altra riga né a una cella (C-P1).
    harness.key_press(Key::Tab);
    harness.run();
    let rows: Vec<egui::Rect> = harness
        .get_all_by_role(Role::Row)
        .map(|row| row.rect())
        .collect();
    let focused = harness
        .root()
        .children_recursive()
        .find(egui_kittest::Node::is_focused)
        .map(|node| (node.accesskit_node().role(), node.rect()));
    // Una cella focalizzabile è un contenitore generico (senza posizione) o sta in una riga.
    let in_a_row = |(role, rect): &(Role, egui::Rect)| {
        matches!(role, Role::Row | Role::GenericContainer)
            || rows.iter().any(|r| r.contains_rect(*rect))
    };
    assert!(
        focused.as_ref().is_some_and(|focused| !in_a_row(focused)),
        "dopo il Tab: {focused:?}"
    );
}

/// C-15: l'ordine del Tab segue la lettura: luoghi, termini, poi Cerca.
#[test]
fn tab_order_follows_reading_order() {
    let (mut harness, i18n) = harness(Language::Italian);
    harness.run();
    let mut order = Vec::new();
    for _ in 0..40 {
        harness.key_press(Key::Tab);
        harness.run();
        if let Some(label) = focused_label(&harness) {
            order.push(label);
        }
    }
    let position = |label: &str| order.iter().position(|l| l == label);
    let add_folder = position(&i18n.tr("form-add-folder")).expect("Aggiungi cartella");
    let term = position(&i18n.tr_args("form-term-name", &number(1))).expect("Termine 1");
    let search = position(&i18n.tr("action-search")).expect("Cerca");
    assert!(add_folder < term && term < search, "{order:?}");
}

/// C-P4: F6 sposta il focus tra pannello di ricerca, tabella e anteprima, in giro.
#[test]
fn f6_moves_between_panes() {
    let (mut harness, i18n) = harness(Language::Italian);
    search_ar_in_demo(&mut harness, &i18n);
    harness
        .get_by_role_and_label(Role::Label, "l'aragosta")
        .click();
    harness.run();
    let row = focused_label(&harness).expect("riga con il focus");
    harness.key_press(Key::F6);
    harness.run();
    assert_eq!(
        focused_label(&harness),
        Some(i18n.tr("menu-previous-file")),
        "anteprima"
    );
    harness.key_press(Key::F6);
    harness.run();
    assert_eq!(
        focused_label(&harness),
        Some(i18n.tr_args("form-term-name", &number(1))),
        "pannello di ricerca"
    );
    harness.key_press(Key::F6);
    harness.run();
    assert_eq!(focused_label(&harness), Some(row), "di nuovo la tabella");
    harness.key_press_modifiers(egui::Modifiers::SHIFT, Key::F6);
    harness.run();
    assert_eq!(
        focused_label(&harness),
        Some(i18n.tr_args("form-term-name", &number(1))),
        "Maiusc+F6 torna indietro"
    );
}

/// C-P6: un paragrafo lungo nell'anteprima va a capo invece di scorrere in orizzontale.
#[test]
fn long_preview_lines_wrap() {
    let dir = tempfile::tempdir().expect("cartella temporanea");
    let line = format!("aragosta {}", "parola ".repeat(600));
    std::fs::write(dir.path().join("lungo.txt"), &line).expect("scritto");
    let (mut harness, i18n) = harness(Language::Italian);
    start_search(&mut harness, &i18n, dir.path(), "aragosta");
    step_until(&mut harness, "fine della ricerca", |app| {
        !app.is_searching() && !app.view().is_empty()
    });
    harness.run();
    harness.state_mut().view_mut().jump_to(0);
    step_until(&mut harness, "anteprima", |app| {
        matches!(app.preview(), PreviewState::Ready(..))
    });
    harness.run();
    let unit = harness
        .query_all_by_role(Role::Label)
        .find(|node| {
            node.accesskit_node()
                .value()
                .is_some_and(|value| value.starts_with("aragosta parola"))
        })
        .expect("riga dell'anteprima");
    let rect = unit.rect();
    assert!(
        rect.max.x <= 1280.0,
        "nessuno scorrimento orizzontale: {rect:?}"
    );
    assert!(rect.height() > 40.0, "il paragrafo va a capo: {rect:?}");
}

/// Trascina `fixtures/demo` nella finestra, scrive "ar", preme Cerca e aspetta la fine.
fn search_ar_in_demo(harness: &mut Harness<'static, Application>, i18n: &I18n) {
    // Primo frame: ogni controllo ha un nome accessibile (controllato da `run`).
    harness.run();

    // Si trascina la cartella dei file di prova nella finestra.
    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(Dropped(demo_dir())));
    harness.run();
    assert_eq!(harness.state().form.roots.len(), 1, "radice aggiunta");

    let term_name = i18n.tr_args("form-term-name", &number(1));
    let term = harness.get_by_role_and_label(Role::TextInput, &term_name);
    term.focus();
    term.type_text("ar");
    harness.run();

    harness
        .get_by_role_and_label(Role::Button, &i18n.tr("action-search"))
        .click();

    let deadline = Instant::now() + SEARCH_DEADLINE;
    loop {
        harness.step();
        let found = harness
            .query_by_role_and_label(Role::Label, "l'aragosta")
            .is_some();
        if found && !harness.state().is_searching() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "\"l'aragosta\" non trovata entro {SEARCH_DEADLINE:?}"
        );
        std::thread::sleep(FRAME_PAUSE);
    }
    harness.run();
}

#[test]
fn empty_term_shows_the_error_under_the_field() {
    let (mut harness, i18n) = harness(Language::Italian);
    harness.run();
    harness
        .get_by_role_and_label(Role::Button, &i18n.tr("action-search"))
        .click();
    harness.run();
    assert!(!harness.state().is_searching());
    harness.get_by_label(&i18n.tr("error-no-folder"));
    harness.get_by_label(&i18n.tr("error-term-empty"));
}

#[test]
fn add_and_remove_term_rows() {
    let (mut harness, i18n) = harness(Language::English);
    harness.run();
    harness
        .get_by_role_and_label(Role::Button, &i18n.tr("form-add-term"))
        .click();
    harness.run();
    assert_eq!(harness.state().form.terms().len(), 2);
    harness.get_by_role_and_label(Role::TextInput, &i18n.tr_args("form-term-name", &number(2)));
    harness
        .get_by_role_and_label(
            Role::Button,
            &i18n.tr_args("form-term-remove-name", &number(2)),
        )
        .click();
    harness.run();
    assert_eq!(harness.state().form.terms().len(), 1);
}

#[test]
fn the_window_fits_1024_by_600() {
    let (mut harness, i18n) = harness(Language::Italian);
    harness.set_size(egui::vec2(1024.0, 600.0));
    harness.run();
    // Il bottone primario resta raggiungibile anche nella finestra piccola.
    let label = i18n.tr("action-search");
    let search = harness.get_by_role_and_label(Role::Button, &label);
    let rect = search.rect();
    assert!(rect.max.y <= 600.0 && rect.max.x <= 1024.0, "{rect:?}");
}

/// T-C09: a 1024×600, con risultati al limite (banner), anteprima e un messaggio, in
/// italiano e in inglese: Cerca visibile, almeno 5 righe, banner e messaggio nella
/// finestra.
#[test]
fn the_window_fits_1024_by_600_with_everything_shown() {
    let dir = many_words("parole.txt", 200_001);
    for language in Language::ALL {
        let (mut harness, i18n) = harness(language);
        harness.set_size(egui::vec2(1024.0, 600.0));
        start_search(&mut harness, &i18n, dir.path(), "ar");
        step_until(&mut harness, "limite dei risultati", |app| {
            !app.is_searching() && app.limit_reached()
        });
        harness.run();
        harness
            .state_mut()
            .set_notice(Notice::RootsLimit { max: 256 });
        harness.state_mut().view_mut().jump_to(0);
        step_until(&mut harness, "anteprima", |app| {
            matches!(app.preview(), PreviewState::Ready(..))
        });
        harness.run();

        let inside = |rect: egui::Rect| rect.max.x <= 1024.0 && rect.max.y <= 600.0;
        let search_label = i18n.tr("action-search");
        let search = harness.get_by_role_and_label(Role::Button, &search_label);
        assert!(
            inside(search.rect()),
            "{language:?}: Cerca {:?}",
            search.rect()
        );
        let rows = harness
            .get_all_by_role(Role::Row)
            .filter(|row| inside(row.rect()))
            .count();
        assert!(rows >= 5, "{language:?}: {rows} righe visibili");
        let banner_title = i18n.tr("banner-limit-title");
        let banner = harness.get_by_label(&banner_title);
        assert!(
            inside(banner.rect()),
            "{language:?}: banner {:?}",
            banner.rect()
        );
        let notice_text = i18n.tr_args("notice-roots-limit", &[("max", 256.into())]);
        let notice = harness.get_by_label(&notice_text);
        assert!(
            inside(notice.rect()),
            "{language:?}: messaggio {:?}",
            notice.rect()
        );
        harness.get_by_role_and_label(Role::Button, &i18n.tr("menu-previous-file"));
    }
}

#[test]
fn language_menu_switches_the_interface_to_english() {
    let (mut harness, it) = harness(Language::Italian);
    harness.run();
    harness.get_by_label(&it.tr("menu-tools")).click();
    harness.run();
    harness.get_by_label(&it.tr("menu-language")).click();
    harness.run();
    harness.get_by_label(&it.tr("language-english")).click();
    harness.run();
    let en = I18n::new(Language::English);
    harness.get_by_role_and_label(Role::Label, &en.tr("form-terms-title"));
    assert!(harness.query_by_label(&it.tr("form-terms-title")).is_none());
    assert_eq!(harness.state().settings().language, Some(Language::English));
}

/// Quando la finestra torna in primo piano l'app rilegge il tema a contrasto elevato del
/// sistema: un tema cambiato o spento nel frattempo non resta applicato. Senza l'evento
/// nessuna rilettura.
#[test]
fn regaining_focus_rereads_the_system_contrast() {
    let stale = SystemContrast {
        window: Color32::from_rgb(1, 2, 3),
        window_text: Color32::from_rgb(250, 251, 252),
        highlight: Color32::from_rgb(4, 5, 6),
        highlight_text: Color32::from_rgb(253, 254, 255),
        button_face: Color32::from_rgb(7, 8, 9),
    };
    let mut app = Application::build(Settings::default()).expect("app costruita");
    app.set_system_contrast(Some(stale));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(move |cc| {
            setup_context(&cc.egui_ctx, &app);
            app
        });
    harness.run();
    assert_eq!(harness.state().system_contrast(), Some(stale));
    harness
        .input_mut()
        .events
        .push(egui::Event::WindowFocused(true));
    harness.run();
    assert_eq!(
        harness.state().system_contrast(),
        lynkeus_app::platform::high_contrast()
    );
}
