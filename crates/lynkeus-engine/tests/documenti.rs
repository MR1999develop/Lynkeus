// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Documenti letti nel processo estrattore (il binario `lynkeus-test-worker`, avviato con il
//! lanciatore portabile): `fixtures/demo` completo, `fixtures/formati`, `fixtures/malevoli`,
//! processi che si comportano male, annulla e cache.

mod common;

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{
    documents, documents_with, drain, file_name, fixtures, launcher, request, run, run_with,
    start_with,
};
use lynkeus_core::{Combine, Location};
use lynkeus_engine::{
    DocPath, EntryEnd, ExtractEnd, LaunchedWorker, LimitKind, Limits, Origin, Outcome, WarningKind,
    WorkerLauncher,
};
use lynkeus_extract::{ExtractEvent, ExtractOptions};

/// Un lanciatore che non avvia mai nulla: prova che i risultati vengono dalla cache.
struct NoLaunch;

impl WorkerLauncher for NoLaunch {
    fn launch(&self) -> io::Result<LaunchedWorker> {
        Err(io::Error::other("nessun processo in questo test"))
    }
}

/// (nome del file, voci interne, parola) dei risultati, ordinati.
fn words(found: &common::Outcome) -> Vec<(String, Vec<String>, String)> {
    let mut rows: Vec<_> = found
        .hits
        .iter()
        .map(|h| (file_name(&h.doc.path), h.doc.inner.clone(), h.word.clone()))
        .collect();
    rows.sort();
    rows
}

/// (nome del file, voci interne, motivo) degli avvisi, ordinati per nome.
fn warnings(found: &common::Outcome) -> Vec<(String, Vec<String>, WarningKind)> {
    let mut rows: Vec<_> = found
        .warnings
        .iter()
        .map(|w| (file_name(&w.doc.path), w.doc.inner.clone(), w.kind))
        .collect();
    rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    rows
}

fn row(file: &str, inner: &[&str], word: &str) -> (String, Vec<String>, String) {
    (
        file.to_owned(),
        inner.iter().map(|s| (*s).to_owned()).collect(),
        word.to_owned(),
    )
}

#[test]
fn ar_in_the_whole_demo() {
    let found = run_with(
        request(vec![fixtures("demo")], &["ar"], Combine::Any),
        Some(documents()),
    );
    assert_eq!(found.summary.outcome, Outcome::Completed);
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    assert_eq!(
        words(&found),
        [
            row("archivio.zip", &["elenco.pdf"], "Suonare"),
            row("archivio.zip", &["elenco.pdf"], "chitarra"),
            row("citta.txt", &[], "ARMADIO"),
            row("citta.txt", &[], "d'arte"),
            row("mare.txt", &[], "calamari"),
            row("mare.txt", &[], "l'aragosta"),
            row("note.docx", &[], "Comprare"),
            row("note.docx", &[], "carta"),
            row("utf16.txt", &[], "carta"),
            row("vecchio-1252.txt", &[], "carità"),
        ]
    );
    let carta = found
        .hits
        .iter()
        .find(|h| h.word == "carta" && file_name(&h.doc.path) == "note.docx")
        .expect("carta in note.docx");
    assert_eq!(carta.location, Location::Paragraph(3));
    let chitarra = found
        .hits
        .iter()
        .find(|h| h.word == "chitarra")
        .expect("chitarra");
    assert!(
        matches!(chitarra.location, Location::PageLine { page: 1, .. }),
        "{:?}",
        chitarra.location
    );
    let counts = found.summary.counts;
    assert_eq!(counts.binary_skipped, 1, "solo immagine.png");
    assert_eq!(counts.files_searched, 6);
    assert_eq!(counts.hits, 10);
}

#[test]
fn and_applies_to_each_document_inside_an_archive() {
    let found = run_with(
        request(
            vec![fixtures("demo/musica")],
            &["chitarra", "suonare"],
            Combine::All,
        ),
        Some(documents()),
    );
    assert_eq!(found.hits.len(), 2);
    let none = run_with(
        request(
            vec![fixtures("demo/musica")],
            &["chitarra", "carta"],
            Combine::All,
        ),
        Some(documents()),
    );
    assert!(none.hits.is_empty(), "{:?}", words(&none));
}

#[test]
fn aragosta_in_the_supported_formats() {
    let found = run_with(
        request(vec![fixtures("formati")], &["aragosta"], Combine::Any),
        Some(documents()),
    );
    assert_eq!(found.summary.outcome, Outcome::Completed);
    let files: Vec<(String, Vec<String>)> = words(&found)
        .into_iter()
        .map(|(file, inner, _)| (file, inner))
        .collect();
    for (file, inner) in [
        ("relazione.docx", vec![]),
        ("appunti.odt", vec![]),
        ("catalogo.pdf", vec![]),
        ("spesa.xlsx", vec![]),
        ("bilancio.ods", vec![]),
        ("presentazione.pptx", vec![]),
        ("lezione.odp", vec![]),
        ("spesa.txt", vec![]),
        (
            "annidato.zip",
            vec!["livello2.zip", "livello3.zip", "interno.docx"],
        ),
    ] {
        let inner: Vec<String> = inner.into_iter().map(str::to_owned).collect();
        assert!(
            files.contains(&(file.to_owned(), inner.clone())),
            "{file} {inner:?} mancante in {files:?}"
        );
    }
    let relazione = found
        .hits
        .iter()
        .find(|h| file_name(&h.doc.path) == "relazione.docx")
        .expect("relazione.docx");
    assert_eq!(relazione.location, Location::Paragraph(4));
    let catalogo = found
        .hits
        .iter()
        .find(|h| file_name(&h.doc.path) == "catalogo.pdf")
        .expect("catalogo.pdf");
    assert_eq!(catalogo.location, Location::PageLine { page: 2, line: 3 });
    let warned = warnings(&found);
    for file in ["protetto-password.docx", "protetto-password.pdf"] {
        assert!(
            warned
                .iter()
                .any(|(f, _, kind)| f == file && *kind == WarningKind::Encrypted),
            "{file}: {warned:?}"
        );
    }
}

#[test]
fn malicious_files_give_warnings_without_blocking() {
    let started = Instant::now();
    let found = run_with(
        request(vec![fixtures("malevoli")], &["aragosta"], Combine::Any),
        Some(documents()),
    );
    assert_eq!(found.summary.outcome, Outcome::Completed);
    assert!(started.elapsed() < Duration::from_secs(30));
    let warned = warnings(&found);
    let kind_of = |file: &str| {
        warned
            .iter()
            .filter(|(f, _, _)| f == file)
            .map(|(_, _, kind)| *kind)
            .collect::<Vec<_>>()
    };
    assert!(
        kind_of("bomba.zip")
            .iter()
            .any(|k| matches!(k, WarningKind::LimitExceeded(_))),
        "{warned:?}"
    );
    assert!(
        kind_of("annidato-profondo.zip")
            .contains(&WarningKind::LimitExceeded(LimitKind::ArchiveDepth)),
        "{warned:?}"
    );
    assert_eq!(
        kind_of("immagine-finta.pdf"),
        [WarningKind::ContentMismatch]
    );
    assert_eq!(kind_of("troncato.pdf"), [WarningKind::Corrupt]);
    assert_eq!(kind_of("entita.docx"), [WarningKind::Corrupt]);
    // Nessun risultato inventato: `fondo.txt` (oltre la profondità) non si raggiunge.
    assert!(
        found
            .hits
            .iter()
            .all(|h| !h.doc.inner.iter().any(|n| n.ends_with("fondo.txt")))
    );
    // La riga lunga si legge nel processo principale.
    assert!(
        found
            .hits
            .iter()
            .any(|h| file_name(&h.doc.path) == "riga-lunga.txt" && h.location == Location::Line(2))
    );
}

/// Una cartella temporanea con i file dei processi che si comportano male, più un documento
/// vero.
fn misbehaving_dir(names: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("cartella temporanea");
    for name in names {
        std::fs::write(dir.path().join(name), b"%PDF-1.4\n").expect("file");
    }
    std::fs::copy(
        fixtures("demo/ufficio/note.docx"),
        dir.path().join("note.docx"),
    )
    .expect("copia");
    dir
}

#[test]
fn misbehaving_workers_are_replaced_and_reported() {
    let dir = misbehaving_dir(&[
        "lento.pdf",
        "muore.pdf",
        "sporco.pdf",
        "fuori-sequenza.pdf",
        "altro-id.pdf",
    ]);
    let source = documents_with(launcher(), 1, None);
    let mut req = request(vec![dir.path().to_owned()], &["carta"], Combine::Any);
    req.limits = Limits {
        extract_timeout: Duration::from_secs(2),
        ..Limits::default()
    };
    let found = run_with(req, Some(Arc::clone(&source)));
    assert_eq!(found.summary.outcome, Outcome::Completed);
    assert_eq!(
        warnings(&found),
        [
            ("altro-id.pdf".to_owned(), vec![], WarningKind::Corrupt),
            (
                "fuori-sequenza.pdf".to_owned(),
                vec![],
                WarningKind::Corrupt
            ),
            ("lento.pdf".to_owned(), vec![], WarningKind::Timeout),
            ("muore.pdf".to_owned(), vec![], WarningKind::Corrupt),
            ("sporco.pdf".to_owned(), vec![], WarningKind::Corrupt),
        ]
    );
    // Il documento vero si legge comunque, con un processo nuovo dopo ogni guasto.
    assert_eq!(words(&found), [row("note.docx", &[], "carta")]);
    assert!(
        source.pool().launches() >= 5,
        "{}",
        source.pool().launches()
    );
}

#[test]
fn cancel_stops_a_busy_worker_within_one_second() {
    let dir = misbehaving_dir(&["lento.pdf"]);
    let source = documents_with(launcher(), 1, None);
    let req = request(vec![dir.path().join("lento.pdf")], &["carta"], Combine::Any);
    let (mut handle, wake) = start_with(req, Some(Arc::clone(&source)));
    // Si aspetta che il processo stia leggendo il file lento.
    let deadline = Instant::now() + Duration::from_secs(30);
    while source.pool().launches() == 0 {
        assert!(Instant::now() < deadline, "processo mai avviato");
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(200));
    let cancelled = Instant::now();
    handle.cancel();
    let found = drain(&mut handle, &wake);
    let took = cancelled.elapsed();
    assert_eq!(found.summary.outcome, Outcome::Cancelled);
    assert!(took < Duration::from_secs(1), "annulla in {took:?}");
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
}

#[test]
fn second_search_reads_from_the_cache_and_corrupt_entries_are_dropped() {
    let cache = tempfile::tempdir().expect("cartella della cache");
    let cache_dir = cache.path().join("cache");
    let demo = || request(vec![fixtures("demo")], &["ar"], Combine::Any);

    let first = run_with(
        demo(),
        Some(documents_with(launcher(), 1, Some(&cache_dir))),
    );
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);

    // Nessun processo avviabile: i documenti arrivano solo dalla cache.
    let cached = documents_with(Arc::new(NoLaunch), 1, Some(&cache_dir));
    let second = run_with(demo(), Some(Arc::clone(&cached)));
    assert!(second.warnings.is_empty(), "{:?}", second.warnings);
    assert_eq!(words(&second), words(&first));
    assert_eq!(cached.cache_hits(), 2);
    assert_eq!(cached.pool().launches(), 0);

    // Voci rovinate: si ignorano, si cancellano e il documento torna al processo (che qui
    // non c'è: avviso).
    let entries: Vec<PathBuf> = std::fs::read_dir(&cache_dir)
        .expect("cartella della cache")
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(entries.len(), 2, "{entries:?}");
    for entry in &entries {
        std::fs::write(entry, b"rovinata").expect("scrittura");
    }
    let third = run_with(
        demo(),
        Some(documents_with(Arc::new(NoLaunch), 1, Some(&cache_dir))),
    );
    let warned = warnings(&third);
    assert_eq!(warned.len(), 2, "{warned:?}");
    assert!(
        warned
            .iter()
            .all(|(_, _, kind)| *kind == WarningKind::ExtractorUnavailable)
    );
    assert!(entries.iter().all(|e| !e.exists()));
}

/// Una scrittura della cache interrotta (app uccisa, disco pieno) lascia un temporaneo
/// parziale; una copia o un ripristino interrotti della cartella lasciano una voce troncata.
/// La voce troncata si scarta e il documento si rilegge; il temporaneo vecchio si cancella,
/// quello recente (forse di un'altra istanza che sta scrivendo) resta.
#[test]
fn interrupted_cache_writes_are_ignored_and_the_document_is_read_again() {
    let docs = misbehaving_dir(&[]);
    let cache = tempfile::tempdir().expect("cartella della cache");
    let cache_dir = cache.path().join("cache");
    let carta = || request(vec![docs.path().to_owned()], &["carta"], Combine::Any);

    let first = run_with(
        carta(),
        Some(documents_with(launcher(), 1, Some(&cache_dir))),
    );
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    assert_eq!(words(&first), [row("note.docx", &[], "carta")]);
    let entries: Vec<PathBuf> = std::fs::read_dir(&cache_dir)
        .expect("cartella della cache")
        .flatten()
        .map(|e| e.path())
        .collect();
    let [entry] = entries.as_slice() else {
        panic!("una sola voce attesa: {entries:?}");
    };
    let bytes = std::fs::read(entry).expect("voce");
    let half = bytes.get(..bytes.len() / 2).expect("metà");
    std::fs::write(entry, half).expect("voce troncata");
    let name = file_name(entry);
    let stale = cache_dir.join(format!("{name}.1-0.tmp"));
    let fresh = cache_dir.join(format!("{name}.2-0.tmp"));
    std::fs::write(&stale, half).expect("temporaneo vecchio");
    std::fs::write(&fresh, half).expect("temporaneo recente");
    let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(2 * 60 * 60);
    File::options()
        .write(true)
        .open(&stale)
        .and_then(|f| f.set_modified(two_hours_ago))
        .expect("data del temporaneo");

    let source = documents_with(launcher(), 1, Some(&cache_dir));
    let second = run_with(carta(), Some(Arc::clone(&source)));
    assert!(second.warnings.is_empty(), "{:?}", second.warnings);
    assert_eq!(words(&second), words(&first));
    assert_eq!(source.cache_hits(), 0);
    assert_eq!(source.pool().launches(), 1);
    assert!(!stale.exists(), "temporaneo vecchio non cancellato");
    assert!(fresh.exists(), "temporaneo recente cancellato");

    // La voce riscritta è valida: senza processi la terza ricerca la legge dalla cache.
    let cached = documents_with(Arc::new(NoLaunch), 1, Some(&cache_dir));
    let third = run_with(carta(), Some(Arc::clone(&cached)));
    assert!(third.warnings.is_empty(), "{:?}", third.warnings);
    assert_eq!(words(&third), words(&first));
    assert_eq!(cached.cache_hits(), 1);
}

#[test]
fn without_a_document_source_documents_are_skipped() {
    let found = run(request(
        vec![fixtures("demo/ufficio")],
        &["carta"],
        Combine::Any,
    ));
    assert!(found.hits.is_empty());
    assert_eq!(found.summary.counts.files_skipped, 1);
}

#[test]
fn units_around_a_hit_for_the_preview() {
    let source = documents();
    let doc = DocPath {
        path: fixtures("demo/musica/archivio.zip"),
        inner: vec!["elenco.pdf".to_owned()],
    };
    let hit = run_with(
        request(vec![fixtures("demo/musica")], &["chitarra"], Combine::Any),
        Some(Arc::clone(&source)),
    )
    .hits
    .pop()
    .expect("chitarra");
    let never = || false;
    let units = source
        .units_around(
            &doc,
            &hit.location,
            20,
            &ExtractOptions::default(),
            Duration::from_secs(30),
            &never,
        )
        .expect("documento leggibile")
        .expect("unità trovata");
    assert!(
        units
            .iter()
            .any(|u| u.location == hit.location && u.text.contains("chitarra"))
    );

    let missing = DocPath {
        path: fixtures("demo/musica/archivio.zip"),
        inner: vec!["non-esiste.pdf".to_owned()],
    };
    let none = source.units_around(
        &missing,
        &hit.location,
        20,
        &ExtractOptions::default(),
        Duration::from_secs(30),
        &never,
    );
    assert_eq!(none, Ok(None));
}

#[test]
fn the_pool_streams_events_directly() {
    let source = documents();
    let mut events: Vec<ExtractEvent> = Vec::new();
    let never = || false;
    let end = source.pool().extract(
        Path::new(&fixtures("demo/ufficio/note.docx")),
        &ExtractOptions::default(),
        Duration::from_secs(30),
        &never,
        &mut events,
    );
    assert_eq!(end, ExtractEnd::Completed);
    assert!(matches!(
        events.first(),
        Some(ExtractEvent::DocumentStart { .. })
    ));
    assert_eq!(events.last(), Some(&ExtractEvent::DocumentEnd));
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

/// Estrae una voce con il pool dei test e restituisce esito e byte.
fn pool_entry(path: PathBuf, inner: &[&str], max_bytes: u64) -> (EntryEnd, Vec<u8>) {
    let source = documents();
    let doc = DocPath {
        path,
        inner: inner.iter().map(|s| (*s).to_owned()).collect(),
    };
    let mut bytes = Vec::new();
    let never = || false;
    let end = source.extract_entry(
        &doc,
        max_bytes,
        &ExtractOptions::default(),
        Duration::from_secs(30),
        &never,
        &mut bytes,
    );
    (end, bytes)
}

#[test]
fn archive_entries_come_back_byte_identical() {
    let cases: [(&str, &[&str]); 2] = [
        (
            "formati/annidato.zip",
            &["livello2.zip", "livello3.zip", "interno.docx"],
        ),
        ("demo/musica/archivio.zip", &["elenco.pdf"]),
    ];
    for (archive, inner) in cases {
        let expected = zip_entry(&fixtures(archive), inner);
        let (end, bytes) = pool_entry(fixtures(archive), inner, u64::MAX);
        assert_eq!(
            end,
            EntryEnd::Extracted {
                bytes: u64::try_from(expected.len()).unwrap_or(0)
            },
            "{archive}"
        );
        assert_eq!(bytes, expected, "{archive}");
    }
}

#[test]
fn entry_partitions_through_the_pool() {
    let (end, _) = pool_entry(
        fixtures("demo/musica/archivio.zip"),
        &["manca.pdf"],
        u64::MAX,
    );
    assert_eq!(
        end,
        EntryEnd::NotExtracted(lynkeus_extract::WarningKind::NotFound)
    );
    let (end, _) = pool_entry(fixtures("demo/musica/archivio.zip"), &["elenco.pdf"], 10);
    assert_eq!(
        end,
        EntryEnd::NotExtracted(lynkeus_extract::WarningKind::LimitExceeded(
            LimitKind::EntrySize
        ))
    );
}

/// Un percorso relativo alla cartella corrente di questo processo (quella del pacchetto,
/// nei test di cargo): il processo estrattore ha un'altra cartella corrente (quella del
/// suo eseguibile), quindi il percorso deve arrivargli assoluto. La cache usa la stessa
/// chiave per il percorso relativo e per quello assoluto.
#[test]
fn relative_paths_are_resolved_in_this_process() {
    let here = tempfile::tempdir_in(".").expect("cartella nella cartella corrente");
    let name = here
        .path()
        .file_name()
        .map(PathBuf::from)
        .expect("nome della cartella");
    std::fs::copy(
        fixtures("demo/ufficio/note.docx"),
        here.path().join("note.docx"),
    )
    .expect("copia");
    std::fs::copy(
        fixtures("demo/musica/archivio.zip"),
        here.path().join("archivio.zip"),
    )
    .expect("copia");
    let relative = name.join("note.docx");
    assert!(relative.is_relative());

    let cache = tempfile::tempdir().expect("cartella della cache");
    let source = documents_with(launcher(), 1, Some(&cache.path().join("cache")));
    let never = || false;
    let mut events: Vec<ExtractEvent> = Vec::new();
    let (end, _) = source.extract(
        &relative,
        &ExtractOptions::default(),
        Duration::from_secs(30),
        &never,
        &mut events,
    );
    assert_eq!(end, ExtractEnd::Completed);
    assert!(
        matches!(events.first(), Some(ExtractEvent::DocumentStart { .. })),
        "{events:?}"
    );
    let absolute = std::path::absolute(&relative).expect("assoluto");
    let mut again: Vec<ExtractEvent> = Vec::new();
    let (end, origin) = source.extract(
        &absolute,
        &ExtractOptions::default(),
        Duration::from_secs(30),
        &never,
        &mut again,
    );
    assert_eq!((end, origin), (ExtractEnd::Completed, Origin::Cache));
    assert_eq!(again, events);

    let (end, bytes) = pool_entry(name.join("archivio.zip"), &["elenco.pdf"], u64::MAX);
    assert!(matches!(end, EntryEnd::Extracted { .. }), "{end:?}");
    assert!(bytes.starts_with(b"%PDF"));
}
