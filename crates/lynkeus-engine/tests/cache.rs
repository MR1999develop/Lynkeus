// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! La cache del testo usata in concorrenza: svuotata durante una ricerca, condivisa da due
//! istanze del programma sulla stessa cartella.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use common::{documents_with, drain, file_name, fixtures, launcher, request, run_with, start_with};
use lynkeus_core::{ByteSize, Combine, Location, TextUnit};
use lynkeus_engine::{CacheKey, Outcome, TextCache};
use lynkeus_extract::{ExtractEvent, ExtractOptions, Format};

/// Tempo massimo tra Annulla e `Finished`.
const CANCEL_BUDGET: Duration = Duration::from_secs(1);

/// (file, voci interne, parola) dei risultati, ordinati.
fn words(found: &common::Outcome) -> Vec<(String, Vec<String>, String)> {
    let mut rows: Vec<_> = found
        .hits
        .iter()
        .map(|h| (file_name(&h.doc.path), h.doc.inner.clone(), h.word.clone()))
        .collect();
    rows.sort();
    rows
}

/// Svuotare la cache mentre una ricerca la legge e la scrive non rompe nulla: ogni documento
/// arriva intero (dalla cache o dal processo), nessun avviso; Annulla resta pronto.
#[test]
fn cache_clear_during_search_is_safe() {
    let cache = tempfile::tempdir().expect("cartella della cache");
    let cache_dir = cache.path().join("cache");
    let demo = || request(vec![fixtures("demo")], &["ar"], Combine::Any);
    let expected = words(&run_with(
        demo(),
        Some(documents_with(launcher(), 1, Some(&cache_dir))),
    ));

    for round in 0..10 {
        let source = documents_with(launcher(), 2, Some(&cache_dir));
        let clearing = {
            let source = Arc::clone(&source);
            std::thread::spawn(move || source.cache().map(TextCache::clear))
        };
        let found = run_with(demo(), Some(Arc::clone(&source)));
        let cleared = clearing.join().expect("thread di pulizia");
        assert!(matches!(cleared, Some(Ok(_))), "giro {round}: {cleared:?}");
        assert_eq!(found.summary.outcome, Outcome::Completed, "giro {round}");
        assert!(
            found.warnings.is_empty(),
            "giro {round}: {:?}",
            found.warnings
        );
        assert_eq!(words(&found), expected, "giro {round}");
    }

    // Svuotata a metà ricerca, poi Annulla.
    let source = documents_with(launcher(), 2, Some(&cache_dir));
    let (mut handle, wake) = start_with(demo(), Some(Arc::clone(&source)));
    let cleared = source.cache().map(TextCache::clear);
    assert!(matches!(cleared, Some(Ok(_))), "{cleared:?}");
    let cancelled = Instant::now();
    handle.cancel();
    let found = drain(&mut handle, &wake);
    let took = cancelled.elapsed();
    assert!(took < CANCEL_BUDGET, "annulla in {took:?}");
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    assert!(
        words(&found).iter().all(|row| expected.contains(row)),
        "{:?}",
        words(&found)
    );
}

/// La voce di prova numero `n`: la stessa per tutti gli scrittori, lunga uguale per ogni `n`.
fn key(n: u32) -> CacheKey {
    CacheKey::new(
        Path::new("dati/documento.docx"),
        u64::from(n),
        UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        &ExtractOptions::default(),
    )
}

fn events(n: u32) -> Vec<ExtractEvent> {
    vec![
        ExtractEvent::DocumentStart {
            inner: Vec::new(),
            format: Format::Docx,
        },
        ExtractEvent::Unit(TextUnit {
            location: Location::Paragraph(1),
            text: format!("testo della voce {n:06} {}", "carta ".repeat(200)),
        }),
        ExtractEvent::DocumentEnd,
    ]
}

/// Byte delle voci sul disco (i temporanei non contano).
fn disk_usage(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|listing| {
            listing
                .flatten()
                .filter(|item| item.path().extension().is_some_and(|e| e == "lkc"))
                .filter_map(|item| item.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

/// Due istanze sulla stessa cartella, con scritture, letture e sfratti concorrenti: ogni
/// lettura dà la voce giusta o niente; il limite vale per la cartella, non per istanza.
#[test]
fn two_instances_share_the_cache_safely() {
    let root = tempfile::tempdir().expect("cartella");
    let dir: PathBuf = root.path().join("cache");
    // Il peso di una voce, misurato con una cache a parte.
    let probe = common::text_cache(&root.path().join("misura"), ByteSize::from_mib(16));
    probe.put(&key(0), &events(0)).expect("voce di misura");
    let one = probe.usage().bytes();
    let max = ByteSize::from_bytes(one * 6);

    let instances = [
        Arc::new(common::text_cache(&dir, max)),
        Arc::new(common::text_cache(&dir, max)),
    ];
    let workers: Vec<_> = instances
        .iter()
        .enumerate()
        .map(|(which, cache)| {
            let cache = Arc::clone(cache);
            std::thread::spawn(move || {
                let first = u32::try_from(which).unwrap_or(0) * 20;
                for n in first..first + 20 {
                    cache.put(&key(n), &events(n)).expect("scrittura");
                    // Letture di voci proprie e dell'altra istanza, forse sfrattate.
                    for m in [n, (n + 20) % 40, n / 2] {
                        if let Some(read) = cache.get(&key(m)) {
                            assert_eq!(read, events(m), "voce {m} sbagliata");
                        }
                    }
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().expect("istanza");
    }
    // Al prossimo riallineamento ogni istanza vede anche le voci dell'altra.
    instances[0].set_max_size(max);
    let used = disk_usage(&dir);
    assert!(
        used <= max.bytes(),
        "{used} byte sul disco, massimo {}",
        max.bytes()
    );
    assert!(used > 0);
}
