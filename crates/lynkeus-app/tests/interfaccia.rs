// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Test dell'interfaccia: l'app vera (`eframe::App`) dentro `egui_kittest`, guidata come la
//! guiderebbe un lettore di schermo, cercando i controlli per ruolo e nome accessibile.
//! I nomi tradotti si chiedono al traduttore dell'app, mai scritti nel test; i dati
//! ("l'aragosta") vengono dai file di prova.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use fluent_templates::fluent_bundle::FluentValue;
use lynkeus_app::i18n::{I18n, Language};
use lynkeus_app::{Application, Settings, Tab, setup_context};

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
    let settings = Settings {
        language: Some(language),
        ..Settings::default()
    };
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
