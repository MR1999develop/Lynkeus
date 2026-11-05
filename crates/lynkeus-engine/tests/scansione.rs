// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Scansione e lettura su cartelle di prova create dai test: filtri, nascosti, binari,
//! codifiche, radici miste, annulla, limite dei risultati, OR/AND.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use common::{TIMEOUT, drain, next_event, request, run, start};
use lynkeus_core::{ByteSize, Combine, Location};
use lynkeus_engine::{
    Extension, ExtensionFilter, Filters, HiddenPolicy, Limits, Outcome, SearchEvent,
};
use tempfile::TempDir;

/// Cartella di prova con i file dati (percorso relativo, contenuto).
fn tree(files: &[(&str, &[u8])]) -> TempDir {
    let dir = tempfile::tempdir().expect("cartella temporanea");
    for (relative, content) in files {
        write(&dir.path().join(relative), content);
    }
    dir
}

fn write(path: &Path, content: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("cartella creata");
    }
    fs::write(path, content).expect("file scritto");
}

fn ext(raw: &str) -> Extension {
    Extension::parse(raw).expect("estensione valida")
}

#[test]
fn extension_filters_include_and_exclude() {
    let dir = tree(&[
        ("a.txt", b"carta"),
        ("b.md", b"carta"),
        ("c.LOG", b"carta"),
        ("senza", b"carta"),
    ]);
    let mut include = request(vec![dir.path().into()], &["ar"], Combine::Any);
    include.filters.extensions = ExtensionFilter::Include(vec![ext("txt"), ext("log")]);
    let found = run(include);
    assert_eq!(found.files(), ["a.txt", "c.LOG"]);
    assert_eq!(found.summary.counts.files_skipped, 2);

    let mut exclude = request(vec![dir.path().into()], &["ar"], Combine::Any);
    exclude.filters.extensions = ExtensionFilter::Exclude(vec![ext("md")]);
    assert_eq!(run(exclude).files(), ["a.txt", "c.LOG", "senza"]);
}

#[test]
fn skipped_dirs_and_maximum_size() {
    let dir = tree(&[
        ("node_modules/x.txt", b"carta"),
        ("sub/target/y.txt", b"carta"),
        ("sub/ok.txt", b"carta"),
        ("grande.txt", &[b'a'; 100]),
        ("piccolo.txt", b"carta"),
    ]);
    let mut req = request(vec![dir.path().into()], &["a"], Combine::Any);
    req.filters.max_file_size = Some(ByteSize::from_bytes(50));
    let found = run(req);
    assert_eq!(found.files(), ["ok.txt", "piccolo.txt"]);
    let counts = found.summary.counts;
    assert_eq!((counts.files_seen, counts.files_skipped), (3, 1));

    let mut custom = request(vec![dir.path().into()], &["ar"], Combine::Any);
    // L'elenco scelto sostituisce quello predefinito: node_modules ora si visita.
    custom.filters.skip_dirs = vec!["sub".to_owned()];
    assert_eq!(run(custom).files(), ["piccolo.txt", "x.txt"]);
}

/// Rende nascosto `path`: attributo su Windows, già nascosto dal nome altrove.
fn make_hidden(path: &Path) {
    if cfg!(windows) {
        let system_root = std::env::var_os("SystemRoot").expect("SystemRoot");
        let attrib = PathBuf::from(system_root)
            .join("System32")
            .join("attrib.exe");
        let status = std::process::Command::new(attrib)
            .arg("+h")
            .arg(path)
            .status()
            .expect("attrib avviato");
        assert!(status.success());
    }
}

#[test]
fn hidden_files_and_dirs_are_skipped_unless_included() {
    let dir = tree(&[
        (".nascosto.txt", b"carta"),
        (".cartella/dentro.txt", b"carta"),
        ("visibile.txt", b"carta"),
    ]);
    make_hidden(&dir.path().join(".nascosto.txt"));
    make_hidden(&dir.path().join(".cartella"));

    let found = run(request(vec![dir.path().into()], &["ar"], Combine::Any));
    assert_eq!(found.files(), ["visibile.txt"]);

    let mut include = request(vec![dir.path().into()], &["ar"], Combine::Any);
    include.filters.hidden = HiddenPolicy::Include;
    assert_eq!(
        run(include).files(),
        [".nascosto.txt", "dentro.txt", "visibile.txt"]
    );

    // Una radice scelta dall'utente non si esclude perché nascosta.
    let root_file = dir.path().join(".nascosto.txt");
    let found = run(request(vec![root_file], &["ar"], Combine::Any));
    assert_eq!(found.files(), [".nascosto.txt"]);
}

#[test]
fn binaries_are_skipped_and_counted() {
    let dir = tree(&[
        // Non "nul.bin": su Windows "NUL" è un nome di dispositivo riservato.
        ("zero.bin", b"carta\0carta"),
        ("immagine.txt", b"\x89PNG\r\n\x1A\ncarta"),
        ("documento.txt", b"%PDF-1.7 carta"),
        ("testo.txt", b"carta"),
    ]);
    let found = run(request(vec![dir.path().into()], &["ar"], Combine::Any));
    assert_eq!(found.files(), ["testo.txt"]);
    assert_eq!(
        found.summary.counts.binary_skipped, 3,
        "{:?}",
        found.summary.counts
    );
    assert!(found.warnings.is_empty());
}

fn utf16(text: &str, little_endian: bool, bom: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    let units = bom
        .then_some('\u{FEFF}')
        .into_iter()
        .chain(text.chars())
        .collect::<String>();
    for unit in units.encode_utf16() {
        if little_endian {
            bytes.extend(unit.to_le_bytes());
        } else {
            bytes.extend(unit.to_be_bytes());
        }
    }
    bytes
}

#[test]
fn every_supported_encoding_is_decoded() {
    let text = "prima riga\r\nUna carta d'identità\r\n";
    let utf8_bom = [b"\xEF\xBB\xBF".as_slice(), text.as_bytes()].concat();
    let le_bom = utf16(text, true, true);
    let be_bom = utf16(text, false, true);
    let le_plain = utf16(text, true, false);
    let be_plain = utf16(text, false, false);
    let legacy = b"prima riga\nUna carta d'identit\xE0\n".to_vec();
    let dir = tree(&[
        ("utf8.txt", text.as_bytes()),
        ("utf8-bom.txt", &utf8_bom),
        ("utf16le-bom.txt", &le_bom),
        ("utf16be-bom.txt", &be_bom),
        ("utf16le.txt", &le_plain),
        ("utf16be.txt", &be_plain),
        ("1252.txt", &legacy),
    ]);
    let found = run(request(vec![dir.path().into()], &["identit"], Combine::Any));
    let rows = found.rows();
    assert_eq!(rows.len(), 7, "{rows:?}");
    for (file, word, location) in rows {
        assert_eq!(word, "d'identità", "{file}");
        assert_eq!(location, Location::Line(2), "{file}");
    }
}

#[test]
fn mixed_duplicate_and_nested_roots_read_each_file_once() {
    let dir = tree(&[
        ("a/uno.txt", b"carta"),
        ("a/b/due.txt", b"carta"),
        ("a/node_modules/tre.txt", b"carta"),
        ("fuori.txt", b"carta"),
    ]);
    let a = dir.path().join("a");
    let roots = vec![
        a.clone(),
        a.join("b"),
        a.join("b").join("due.txt"),
        a.join(".").join("b").join(".."),
        dir.path().join("fuori.txt"),
        a.join("node_modules"),
    ];
    let found = run(request(roots, &["ar"], Combine::Any));
    assert_eq!(
        found.files(),
        ["due.txt", "fuori.txt", "tre.txt", "uno.txt"]
    );
    let root_of = |name: &str| {
        found
            .hits
            .iter()
            .find(|h| h.doc.path.ends_with(name))
            .map(|h| h.root)
    };
    // La radice più interna visita i suoi file; il duplicato non conta.
    assert_eq!(root_of("uno.txt"), Some(0));
    assert_eq!(root_of("due.txt"), Some(2));
    assert_eq!(root_of("fuori.txt"), Some(4));
    // Una radice dentro una cartella saltata si visita lo stesso.
    assert_eq!(root_of("tre.txt"), Some(5));
    assert_eq!(found.summary.counts.files_seen, 4);
}

#[test]
fn or_and_and_between_two_terms() {
    let dir = tree(&[
        ("entrambi.txt", b"carta\nsasso\n"),
        ("solo-uno.txt", b"carta\n"),
    ]);
    let any = run(request(
        vec![dir.path().into()],
        &["ar", "ss"],
        Combine::Any,
    ));
    assert_eq!(
        any.files(),
        ["entrambi.txt", "entrambi.txt", "solo-uno.txt"]
    );
    let all = run(request(
        vec![dir.path().into()],
        &["ar", "ss"],
        Combine::All,
    ));
    assert_eq!(all.files(), ["entrambi.txt", "entrambi.txt"]);
}

#[test]
fn the_result_limit_is_exact_across_threads() {
    let files: Vec<(String, Vec<u8>)> = (0..60)
        .map(|i| (format!("f{i:02}.txt"), b"carta\ncarta\n".to_vec()))
        .collect();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, c)| (n.as_str(), c.as_slice()))
        .collect();
    let dir = tree(&refs);
    let mut req = request(vec![dir.path().into()], &["ar"], Combine::Any);
    req.limits = Limits {
        max_results: 7,
        ..Limits::default()
    };
    let found = run(req);
    assert_eq!(found.summary.outcome, Outcome::LimitReached);
    assert_eq!(found.hits.len(), 7);
    assert_eq!(found.summary.counts.hits, 7);
}

#[test]
fn cancel_during_the_search() {
    // Più file della coda degli eventi: senza leggere, la ricerca non può finire da sola.
    let files: Vec<(String, Vec<u8>)> = (0..600)
        .map(|i| (format!("f{i:03}.txt"), b"carta".to_vec()))
        .collect();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, c)| (n.as_str(), c.as_slice()))
        .collect();
    let dir = tree(&refs);
    let (mut handle, wake) = start(request(vec![dir.path().into()], &["ar"], Combine::Any));
    let first = next_event(&mut handle, &wake, Instant::now() + TIMEOUT);
    assert!(matches!(first, SearchEvent::Hits(_)));
    let cancelled_at = Instant::now();
    handle.cancel();
    let found = drain(&mut handle, &wake);
    assert_eq!(found.summary.outcome, Outcome::Cancelled);
    assert!(found.hits.len() < 599, "{}", found.hits.len());
    assert!(cancelled_at.elapsed() < TIMEOUT);
}

/// `cancel_and_wait` annulla e aspetta il thread della ricerca: dopo, il thread ha già
/// rilasciato tutto ciò che teneva (la funzione `Notify` resta solo al test).
#[test]
fn cancel_and_wait_ends_the_search_thread() {
    let files: Vec<(String, Vec<u8>)> = (0..600)
        .map(|i| (format!("f{i:03}.txt"), b"carta".to_vec()))
        .collect();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, c)| (n.as_str(), c.as_slice()))
        .collect();
    let dir = tree(&refs);
    let (woken_tx, woken) = std::sync::mpsc::channel();
    let notify: lynkeus_engine::Notify = std::sync::Arc::new(move || {
        // Il test può essere già finito: un segnale perso non conta.
        if woken_tx.send(()).is_err() {}
    });
    let handle = lynkeus_engine::start_search(
        request(vec![dir.path().into()], &["ar"], Combine::Any),
        None,
        std::sync::Arc::clone(&notify),
    )
    .expect("thread avviato");
    woken.recv_timeout(TIMEOUT).expect("primo evento");
    let started = Instant::now();
    handle.cancel_and_wait();
    assert!(started.elapsed() < TIMEOUT);
    assert_eq!(
        std::sync::Arc::strong_count(&notify),
        1,
        "thread ancora vivo"
    );
}

/// Misura P-03 (piano di test): 16 file con 300.000 "il" ciascuno, AND con "zzz": nessun
/// risultato, e i risultati trattenuti da tutti i thread insieme non superano il limite (il
/// picco di memoria privata si legge con Process Explorer). `cargo test --release -p
/// lynkeus-engine --test scansione and_retained_hits -- --ignored --nocapture`.
#[test]
#[ignore = "misura di prestazioni (P-03), da lanciare a mano in release"]
fn and_retained_hits_are_bounded_across_threads() {
    let content = format!("{}\n", "il ".repeat(30))
        .repeat(10_000)
        .into_bytes();
    let names: Vec<String> = (0..16).map(|n| format!("f{n:02}.txt")).collect();
    let refs: Vec<(&str, &[u8])> = names
        .iter()
        .map(|n| (n.as_str(), content.as_slice()))
        .collect();
    let dir = tree(&refs);
    for round in 0..5 {
        let started = Instant::now();
        let found = run(request(
            vec![dir.path().into()],
            &["il", "zzz"],
            Combine::All,
        ));
        println!("P-03 giro {round}: {:?}", started.elapsed());
        assert_eq!(found.summary.outcome, Outcome::Completed);
        assert!(found.hits.is_empty());
    }
}

#[test]
fn a_file_larger_than_a_block_keeps_exact_line_numbers() {
    let mut text = "riga di riempimento\n".repeat(600_000);
    text.push_str("l'aragosta\n");
    let dir = tree(&[("grande.txt", text.as_bytes())]);
    assert!(text.len() > lynkeus_extract::text::BLOCK_BYTES);
    let mut req = request(vec![dir.path().into()], &["arag"], Combine::Any);
    req.filters = Filters {
        max_file_size: None,
        ..Filters::default()
    };
    let found = run(req);
    assert_eq!(
        found.rows(),
        [(
            "grande.txt".to_owned(),
            "l'aragosta".to_owned(),
            Location::Line(600_001)
        )]
    );
}

#[cfg(windows)]
#[test]
fn a_file_locked_by_another_program_becomes_an_in_use_warning() {
    use std::os::windows::fs::OpenOptionsExt;

    let dir = tree(&[("bloccato.txt", b"carta"), ("libero.txt", b"carta")]);
    let locked = dir.path().join("bloccato.txt");
    let _guard = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&locked)
        .expect("file aperto in esclusiva");
    let found = run(request(vec![dir.path().into()], &["ar"], Combine::Any));
    assert_eq!(found.files(), ["libero.txt"]);
    assert_eq!(found.warnings.len(), 1);
    assert_eq!(found.warnings[0].kind, lynkeus_engine::WarningKind::InUse);
    assert!(found.warnings[0].doc.path.ends_with("bloccato.txt"));
}

/// Crea `link` che punta alla cartella `target`: junction su Windows (non servono
/// privilegi), link simbolico altrove.
fn link_dir(target: &Path, link: &Path) {
    #[cfg(windows)]
    {
        let system_root = std::env::var_os("SystemRoot").expect("SystemRoot");
        let cmd = PathBuf::from(system_root).join("System32").join("cmd.exe");
        let status = std::process::Command::new(cmd)
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("mklink avviato");
        assert!(status.success());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).expect("link creato");
}

#[test]
fn links_and_junctions_are_not_followed_below_a_root() {
    let dir = tree(&[("dati/carta.txt", b"carta")]);
    link_dir(&dir.path().join("dati"), &dir.path().join("collegamento"));
    let found = run(request(vec![dir.path().into()], &["ar"], Combine::Any));
    assert_eq!(found.files(), ["carta.txt"]);
    assert!(
        found.hits[0]
            .doc
            .path
            .parent()
            .is_some_and(|p| p.ends_with("dati"))
    );

    // Una radice che è essa stessa un collegamento si visita.
    let found = run(request(
        vec![dir.path().join("collegamento")],
        &["ar"],
        Combine::Any,
    ));
    assert_eq!(found.files(), ["carta.txt"]);
}
