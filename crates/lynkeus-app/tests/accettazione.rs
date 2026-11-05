// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Test d'accettazione: l'app costruita da `Application::build`, collegata al processo
//! estrattore vero (questo stesso binario con `--extract-worker`), cerca "ar" in
//! `fixtures/demo` e trova le parole attese anche nei documenti e negli archivi (vedi
//! `fixtures/README.md`); anteprima, apertura delle voci d'archivio e cache. Poi le opzioni
//! di ricerca (jolly, frasi, OR/AND, accenti, maiuscole) e i file di `fixtures/malevoli`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use lynkeus_app::extraction::{ExtractionSetup, OpenedEntries, PoolDocumentUnits};
use lynkeus_app::i18n::Language;
use lynkeus_app::launch::ArchiveEntryExtractor;
use lynkeus_app::preview::{self, Loaded, PreviewRequest};
use lynkeus_app::{Application, Pump, SearchStatus, Settings};
use lynkeus_core::{AccentMode, CaseMode, Combine, Location, MatchOptions};
use lynkeus_engine::{DocPath, Hit, LimitKind, Limits, Outcome, WarningKind};

const TIMEOUT: Duration = Duration::from_secs(60);

fn fixtures(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(sub)
}

fn demo_dir() -> PathBuf {
    fixtures("demo")
}

/// Il processo estrattore è il binario dell'app, come in produzione.
fn setup(dir: &Path) -> ExtractionSetup {
    ExtractionSetup {
        worker_exe: PathBuf::from(env!("CARGO_BIN_EXE_lynkeus-searcher")),
        cache_dir: Some(dir.join("cache")),
        opened_entries_dir: Some(dir.join("aperti")),
    }
}

/// Un'app in italiano collegata al processo estrattore, con le cartelle in `dir`.
fn connected_app(dir: &Path) -> Application {
    connected_app_with(dir, MatchOptions::default(), Combine::default())
}

/// Come [`connected_app`], con le opzioni di confronto e la combinazione dei termini date.
fn connected_app_with(dir: &Path, options: MatchOptions, combine: Combine) -> Application {
    let settings = Settings {
        language: Some(Language::Italian),
        options,
        combine,
        ..Settings::default()
    };
    let mut app = Application::build(settings).expect("app costruita");
    app.connect_extraction(&setup(dir))
        .expect("processo estrattore collegato");
    app
}

/// Avvia la ricerca e pompa gli eventi finché finisce, aspettando i segnali del worker
/// con una scadenza (nessuna attesa a vuoto).
fn search_in(app: &mut Application, root: &Path, term: &str) {
    search_terms_in(app, root, &[term]);
}

/// Come [`search_in`], con più termini.
fn search_terms_in(app: &mut Application, root: &Path, terms: &[&str]) {
    app.set_root(root);
    app.set_terms(terms);
    let (wake_tx, wake_rx) = mpsc::channel::<()>();
    let notify = Arc::new(move || {
        // A ricerca finita il test non ascolta più: un segnale perso non conta.
        if wake_tx.send(()).is_err() {}
    });
    app.start_search(notify).expect("ricerca avviata");
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match app.pump_events(usize::MAX) {
            Pump::Ended | Pump::NoSearch => return,
            Pump::MorePending => {}
            Pump::Drained => {
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(!left.is_zero(), "ricerca non finita entro {TIMEOUT:?}");
                if wake_rx.recv_timeout(left).is_err() {}
            }
        }
    }
}

fn file_name(hit: &Hit) -> String {
    hit.doc
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn find<'a>(hits: &'a [Hit], file: &str, word: &str) -> Option<&'a Hit> {
    hits.iter().find(|h| file_name(h) == file && h.word == word)
}

#[test]
fn ar_in_demo_finds_the_expected_words() {
    let dir = tempfile::tempdir().expect("cartella temporanea");
    let mut app = connected_app(dir.path());
    search_in(&mut app, &demo_dir(), "ar");
    let SearchStatus::Finished(summary) = app.status() else {
        panic!("ricerca non finita: {:?}", app.status());
    };
    assert_eq!(summary.outcome, Outcome::Completed);
    assert!(
        app.results().warnings.is_empty(),
        "{:?}",
        app.results().warnings
    );
    let hits = &app.results().hits;

    let aragosta = find(hits, "mare.txt", "l'aragosta").expect("l'aragosta in mare.txt");
    assert_eq!(aragosta.location, Location::Line(12));
    assert!(find(hits, "mare.txt", "calamari").is_some());

    let arte = find(hits, "citta.txt", "d'arte").expect("d'arte in citta.txt");
    assert_eq!(arte.location, Location::Line(1));
    let armadio = find(hits, "citta.txt", "ARMADIO").expect("ARMADIO in citta.txt");
    assert_eq!(armadio.location, Location::Line(3));
    let citta_words: Vec<&str> = hits
        .iter()
        .filter(|h| file_name(h) == "citta.txt")
        .map(|h| h.word.as_str())
        .collect();
    assert_eq!(citta_words, ["d'arte", "ARMADIO"]);
    let citta_context = &arte.snippet.text;
    assert!(citta_context.contains("città"), "{citta_context}");

    // Documenti: "carta" nel docx, "chitarra" nel PDF dentro lo zip.
    let carta = find(hits, "note.docx", "carta").expect("carta in note.docx");
    assert_eq!(carta.location, Location::Paragraph(3));
    assert!(find(hits, "note.docx", "Comprare").is_some());
    let chitarra = find(hits, "archivio.zip", "chitarra").expect("chitarra nello zip");
    assert_eq!(chitarra.doc.inner, ["elenco.pdf"]);
    assert!(matches!(
        chitarra.location,
        Location::PageLine { page: 1, .. }
    ));
    assert_eq!(chitarra.doc.to_string().matches(" › elenco.pdf").count(), 1);

    assert!(
        hits.iter().all(|h| file_name(h) != "immagine.png"),
        "nessun risultato da un file binario"
    );
    assert_eq!(summary.counts.binary_skipped, 1);
    assert_eq!(summary.counts.files_searched, 6);
    assert_eq!(hits.len(), 10);
}

#[test]
fn the_second_search_reads_documents_from_the_cache() {
    let dir = tempfile::tempdir().expect("cartella temporanea");
    let mut first = connected_app(dir.path());
    search_in(&mut first, &demo_dir(), "chitarra");
    assert_eq!(first.results().hits.len(), 1);
    first.shutdown_extraction();

    let mut second = connected_app(dir.path());
    search_in(&mut second, &demo_dir(), "chitarra");
    assert_eq!(second.results().hits.len(), 1);
    let source = second.document_source().expect("fonte collegata");
    assert_eq!(source.cache_hits(), 2, "docx e zip dalla cache");
    assert_eq!(source.pool().launches(), 0, "nessun processo avviato");
}

#[test]
fn preview_of_a_pdf_inside_a_zip_shows_the_nearby_units() {
    let dir = tempfile::tempdir().expect("cartella temporanea");
    let mut app = connected_app(dir.path());
    search_in(&mut app, &demo_dir().join("musica"), "chitarra");
    let hit = app.results().hits.first().cloned().expect("chitarra");
    let query = app.results().query.clone().expect("query");
    let source = Arc::clone(app.document_source().expect("fonte collegata"));
    let units = PoolDocumentUnits::new(source, Limits::default());
    let request = PreviewRequest {
        doc: Arc::clone(&hit.doc),
        location: hit.location.clone(),
        query,
    };
    let loaded = preview::load(&request, &units, &AtomicBool::new(false)).expect("anteprima");
    let Loaded::Units(preview) = loaded else {
        panic!("solo contesto: {loaded:?}");
    };
    let focus = &preview.units[preview.focus];
    assert!(focus.text.contains("chitarra"), "{}", focus.text);
    assert!(!focus.highlights.is_empty());
}

/// I byte della voce `inner` letti direttamente con la libreria zip, livello per livello.
fn zip_entry(path: &Path, inner: &[&str]) -> Vec<u8> {
    use std::io::Read;
    let mut bytes = std::fs::read(path).expect("fixture");
    for name in inner {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
        let mut entry = archive.by_name(name).expect("voce");
        let mut out = Vec::new();
        entry.read_to_end(&mut out).expect("lettura");
        bytes = out;
    }
    bytes
}

#[test]
fn archive_entries_open_from_a_private_copy() {
    let dir = tempfile::tempdir().expect("cartella temporanea");
    let app = connected_app(dir.path());
    let source = Arc::clone(app.document_source().expect("fonte collegata"));
    let opened_dir = dir.path().join("aperti");
    let entries = OpenedEntries::new(
        source,
        opened_dir.clone(),
        Limits::default(),
        lynkeus_app::platform::private_fs(),
    );

    // Una voce di testo con un nome pericoloso: il file ha un nome generato, nella cartella
    // privata, con i byte originali.
    let entry = DocPath {
        path: fixtures("malevoli/percorsi.zip"),
        inner: vec!["../../evil.txt".to_owned()],
    };
    let path = entries
        .extract_to_private_temp(&entry)
        .expect("voce leggibile")
        .expect("voce estratta");
    // Nella sottocartella di sessione di questa istanza, dentro la cartella privata.
    let session_dir = path.parent().expect("sottocartella di sessione").to_owned();
    assert_eq!(session_dir.parent(), Some(opened_dir.as_path()));
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    assert!(
        name.starts_with("voce-") && name.ends_with(".txt"),
        "{name}"
    );
    let text = std::fs::read_to_string(&path).expect("file scritto");
    assert!(text.contains("aragosta"), "{text}");

    // Documenti dentro gli archivi, anche a tre livelli: byte identici all'originale, con
    // l'estensione della voce.
    let cases: [(PathBuf, &[&str], &str); 2] = [
        (
            demo_dir().join("musica/archivio.zip"),
            &["elenco.pdf"],
            ".pdf",
        ),
        (
            fixtures("formati/annidato.zip"),
            &["livello2.zip", "livello3.zip", "interno.docx"],
            ".docx",
        ),
    ];
    for (archive, inner, extension) in cases {
        let doc = DocPath {
            path: archive.clone(),
            inner: inner.iter().map(|s| (*s).to_owned()).collect(),
        };
        let opened = entries
            .extract_to_private_temp(&doc)
            .expect("voce leggibile")
            .expect("voce estratta");
        let name = opened.file_name().and_then(|n| n.to_str()).unwrap_or("");
        assert!(name.ends_with(extension), "{name}");
        assert_eq!(opened.parent(), Some(session_dir.as_path()));
        assert_eq!(
            std::fs::read(&opened).ok(),
            Some(zip_entry(&archive, inner))
        );
    }

    // Una voce che non c'è: errore, e nessun file lasciato a metà.
    let before = std::fs::read_dir(&session_dir).map_or(0, Iterator::count);
    let missing = DocPath {
        path: demo_dir().join("musica/archivio.zip"),
        inner: vec!["manca.pdf".to_owned()],
    };
    assert!(entries.extract_to_private_temp(&missing).is_err());
    assert_eq!(
        std::fs::read_dir(&session_dir).map_or(0, Iterator::count),
        before
    );

    let mut app = app;
    app.shutdown_extraction();
    entries.close_session();
    assert!(
        !path.exists() && !session_dir.exists(),
        "sottocartella di sessione cancellata all'uscita"
    );
}

#[test]
fn empty_term_is_refused_before_starting() {
    let mut app = Application::build(Settings::default()).expect("app costruita");
    app.set_root(&demo_dir());
    app.set_term("  ");
    let notify = Arc::new(|| {});
    assert!(app.start_search(notify).is_err());
    assert!(!app.is_searching());
    assert_eq!(app.status(), SearchStatus::Idle);
}

#[test]
fn without_the_extractor_documents_are_skipped_not_misread() {
    let mut app = Application::build(Settings::default()).expect("app costruita");
    search_in(&mut app, &demo_dir(), "ar");
    let hits = &app.results().hits;
    assert!(hits.iter().all(|h| file_name(h) != "note.docx"));
    assert!(hits.iter().all(|h| !h.doc.is_inside_archive()));
}

// --- Opzioni di ricerca su `fixtures/demo` (valori attesi da `fixtures/README.md`) ---

/// Una riga come (file, parola, posizione).
type Row = (String, String, Location);

/// Cerca `terms` in `fixtures/demo` con le opzioni date, controlla che la ricerca sia
/// finita senza avvisi e restituisce le righe ordinate per file e parola.
fn demo_rows(options: MatchOptions, combine: Combine, terms: &[&str]) -> Vec<Row> {
    let dir = tempfile::tempdir().expect("cartella temporanea");
    let mut app = connected_app_with(dir.path(), options, combine);
    search_terms_in(&mut app, &demo_dir(), terms);
    let SearchStatus::Finished(summary) = app.status() else {
        panic!("ricerca non finita: {:?}", app.status());
    };
    assert_eq!(summary.outcome, Outcome::Completed);
    assert!(
        app.results().warnings.is_empty(),
        "{:?}",
        app.results().warnings
    );
    let mut rows: Vec<Row> = app
        .results()
        .hits
        .iter()
        .map(|h| (file_name(h), h.word.clone(), h.location.clone()))
        .collect();
    // L'ordine di arrivo dipende dai thread: si confronta un ordine fisso.
    rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    rows
}

fn row(file: &str, word: &str, location: Location) -> Row {
    (file.to_owned(), word.to_owned(), location)
}

fn with_accents(accents: AccentMode) -> MatchOptions {
    MatchOptions {
        accents,
        ..MatchOptions::default()
    }
}

fn with_case(case: CaseMode) -> MatchOptions {
    MatchOptions {
        case,
        ..MatchOptions::default()
    }
}

/// "c" + un carattere + "rt" + qualsiasi cosa: "carta" nel testo UTF-16 e nel docx, non
/// "carità" (c-a-r-i).
#[test]
fn wildcards_question_and_star_find_carta_in_text_and_docx() {
    let rows = demo_rows(MatchOptions::default(), Combine::Any, &["c?rt*"]);
    assert_eq!(
        rows,
        [
            row("note.docx", "carta", Location::Paragraph(3)),
            row("utf16.txt", "carta", Location::Line(1)),
        ]
    );
}

/// Un termine con uno spazio è una frase: "la car" trova "la carta" del docx e nient'altro.
#[test]
fn phrase_with_a_space_finds_the_whole_phrase_in_docx() {
    let rows = demo_rows(MatchOptions::default(), Combine::Any, &["la car"]);
    assert_eq!(rows, [row("note.docx", "la carta", Location::Paragraph(3))]);
}

/// OR: ogni riga con almeno un termine, da file diversi.
#[test]
fn or_terms_in_different_files_finds_each() {
    let rows = demo_rows(
        MatchOptions::default(),
        Combine::Any,
        &["carta", "aragosta"],
    );
    assert_eq!(
        rows,
        [
            row("mare.txt", "l'aragosta", Location::Line(12)),
            row("note.docx", "carta", Location::Paragraph(3)),
            row("utf16.txt", "carta", Location::Line(1)),
        ]
    );
}

/// AND: nessun file contiene sia "carta" sia "aragosta".
#[test]
fn and_terms_in_different_files_finds_nothing() {
    let rows = demo_rows(
        MatchOptions::default(),
        Combine::All,
        &["carta", "aragosta"],
    );
    assert_eq!(rows, []);
}

/// AND: "carta" e "stampante" stanno nella stessa frase del docx.
#[test]
fn and_terms_in_the_same_file_finds_both() {
    let rows = demo_rows(
        MatchOptions::default(),
        Combine::All,
        &["carta", "stampante"],
    );
    assert_eq!(
        rows,
        [
            row("note.docx", "carta", Location::Paragraph(3)),
            row("note.docx", "stampante", Location::Paragraph(3)),
        ]
    );
}

/// "città" in citta.txt: riga 1 in forma composta (U+00E0) e riga 4 in forma NFD
/// (`a` + U+0300), con la parola com'è scritta nel testo.
fn assert_citta_lines(rows: &[Row]) {
    assert_eq!(
        rows,
        [
            row("citta.txt", "citta\u{300}", Location::Line(4)),
            row("citta.txt", "citt\u{e0}", Location::Line(1)),
        ]
    );
}

/// Accenti distinti (predefinito): "citta" non trova "città".
#[test]
fn accents_distinct_unaccented_term_finds_nothing() {
    let rows = demo_rows(with_accents(AccentMode::Distinct), Combine::Any, &["citta"]);
    assert_eq!(rows, []);
}

/// Accenti distinti: "città" trova anche la forma decomposta della riga 4.
#[test]
fn accents_distinct_accented_term_finds_the_nfd_form_too() {
    let rows = demo_rows(with_accents(AccentMode::Distinct), Combine::Any, &["città"]);
    assert_citta_lines(&rows);
}

/// Accenti ignorati: "citta" e "città" trovano entrambe le forme di "città".
#[test]
fn accents_ignored_both_spellings_find_both_forms() {
    for term in ["citta", "città"] {
        let rows = demo_rows(with_accents(AccentMode::Ignored), Combine::Any, &[term]);
        assert_citta_lines(&rows);
    }
}

/// Maiuscole ignorate (predefinito): "ARMADIO" trova "ARMADIO".
#[test]
fn case_insensitive_uppercase_term_finds_armadio() {
    let rows = demo_rows(with_case(CaseMode::Insensitive), Combine::Any, &["ARMADIO"]);
    assert_eq!(rows, [row("citta.txt", "ARMADIO", Location::Line(3))]);
}

/// Maiuscole distinte: "armadio" non trova "ARMADIO".
#[test]
fn case_sensitive_lowercase_term_finds_nothing() {
    let rows = demo_rows(with_case(CaseMode::Sensitive), Combine::Any, &["armadio"]);
    assert_eq!(rows, []);
}

// --- File malevoli ---

/// I percorsi sotto `dir`, ricorsivamente, in ordine.
fn tree(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path.clone());
            }
            out.push(path);
        }
    }
    out.sort();
    out
}

/// I motivi degli avvisi sul file `file`, nell'ordine di arrivo.
fn warning_kinds(app: &Application, file: &str) -> Vec<WarningKind> {
    app.results()
        .warnings
        .iter()
        .filter(|w| {
            w.doc
                .path
                .file_name()
                .is_some_and(|n| n.to_string_lossy() == file)
        })
        .map(|w| w.kind)
        .collect()
}

/// La ricerca su `fixtures/malevoli` finisce entro la scadenza di [`search_in`], dà gli
/// avvisi attesi e non scrive file fuori dalla cartella temporanea del test.
#[test]
fn malicious_files_end_with_the_expected_warnings_and_write_nothing_outside() {
    let fixtures_before = tree(&fixtures(""));
    let dir = tempfile::tempdir().expect("cartella temporanea");
    let mut app = connected_app(dir.path());
    search_in(&mut app, &fixtures("malevoli"), "aragosta");
    let SearchStatus::Finished(summary) = app.status() else {
        panic!("ricerca non finita: {:?}", app.status());
    };
    assert_eq!(summary.outcome, Outcome::Completed);

    let all = &app.results().warnings;
    let expected = [
        (
            "bomba.zip",
            WarningKind::LimitExceeded(LimitKind::CompressionRatio),
        ),
        (
            "annidato-profondo.zip",
            WarningKind::LimitExceeded(LimitKind::ArchiveDepth),
        ),
        ("immagine-finta.pdf", WarningKind::ContentMismatch),
        ("entita.docx", WarningKind::Corrupt),
        ("troncato.pdf", WarningKind::Corrupt),
    ];
    for (file, kind) in expected {
        assert_eq!(warning_kinds(&app, file), [kind], "{file}: {all:?}");
    }
    // Nessun altro avviso: percorsi.zip e riga-lunga.txt si leggono.
    assert_eq!(all.len(), expected.len(), "{all:?}");

    let hits = &app.results().hits;
    assert!(
        hits.iter()
            .all(|h| !h.doc.inner.iter().any(|n| n == "fondo.txt")),
        "fondo.txt è oltre il limite di profondità"
    );
    let long_line = find(hits, "riga-lunga.txt", "aragosta").expect("aragosta in riga-lunga.txt");
    assert_eq!(long_line.location, Location::Line(2));

    // Nessun file nuovo tra le fixture né dove puntano i nomi delle voci di percorsi.zip.
    app.shutdown_extraction();
    assert_eq!(tree(&fixtures("")), fixtures_before);
    let repo = fixtures("..");
    for escaped in [
        repo.join("evil.txt"),
        repo.join("../evil.txt"),
        PathBuf::from("/assoluto/evil.txt"),
        PathBuf::from("C:/Windows/evil.txt"),
    ] {
        assert!(!escaped.exists(), "{}", escaped.display());
    }
}
