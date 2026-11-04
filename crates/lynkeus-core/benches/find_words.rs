// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Benchmark di `find_words` su un testo italiano generato di circa 4 MB.
//!
//! Si esegue con `cargo bench -p lynkeus-core`. Il testo è deterministico (generatore
//! lineare congruenziale con seme fisso), con accenti, punteggiatura e righe CRLF.

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use lynkeus_core::{
    AccentMode, CaseMode, Combine, MatchOptions, Query, Term, WildcardMode, find_words,
};

/// Dimensione approssimativa del testo, in byte.
const TEXT_BYTES: usize = 4 * 1024 * 1024;

const WORDS: &[&str] = &[
    "la",
    "carta",
    "città",
    "perché",
    "aragosta",
    "chitarra",
    "è",
    "più",
    "così",
    "casa",
    "cortile",
    "dell'arte",
    "«libro»",
    "però",
    "università",
    "caffè",
    "(nota)",
    "ventitré",
    "il",
    "documento",
    "ricerca",
    "testo",
    "parola",
    "e",
    "di",
    "che",
    "un",
    "una",
    "per",
];

/// Testo deterministico: parole dal vocabolario, punteggiatura e a capo ogni tanto.
fn sample_text() -> String {
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut text = String::with_capacity(TEXT_BYTES + 64);
    let mut words_in_line = 0;
    while text.len() < TEXT_BYTES {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let pick = usize::try_from(state >> 33).unwrap_or(0);
        text.push_str(WORDS[pick % WORDS.len()]);
        words_in_line += 1;
        match pick % 17 {
            0 => text.push_str(", "),
            1 => text.push_str(". "),
            _ if words_in_line > 12 => {
                text.push_str("\r\n");
                words_in_line = 0;
            }
            _ => text.push(' '),
        }
    }
    text
}

fn compile(terms: &[&str], options: MatchOptions) -> lynkeus_core::CompiledQuery {
    let terms = terms
        .iter()
        .filter_map(|t| Term::parse(t, options.wildcards).ok())
        .collect();
    match Query::new(terms, options, Combine::Any).and_then(|q| q.compile()) {
        Ok(query) => query,
        Err(error) => panic!("query del benchmark non valida: {error}"),
    }
}

fn bench_find_words(c: &mut Criterion) {
    let text = sample_text();
    let default = MatchOptions::default();
    let ignored = MatchOptions {
        accents: AccentMode::Ignored,
        ..default
    };
    let sensitive = MatchOptions {
        case: CaseMode::Sensitive,
        ..default
    };
    let literal = MatchOptions {
        wildcards: WildcardMode::Literal,
        ..default
    };
    let cases: [(&str, Vec<&str>, MatchOptions); 7] = [
        ("letterale", vec!["ar"], default),
        ("letterale_maiuscole", vec!["ar"], sensitive),
        ("raro", vec!["zqx"], default),
        ("jolly", vec!["c?rt*"], default),
        ("jolly_iniziale", vec!["*arta"], default),
        ("accenti_ignorati", vec!["citta", "perche"], ignored),
        (
            "frase_e_piu_termini",
            vec!["la carta", "città", "a*", "?"],
            literal,
        ),
    ];
    let mut group = c.benchmark_group("find_words");
    group.throughput(Throughput::Bytes(
        u64::try_from(text.len()).unwrap_or(u64::MAX),
    ));
    group.sample_size(10);
    for (name, terms, options) in cases {
        let query = compile(&terms, options);
        group.bench_function(name, |b| {
            b.iter(|| find_words(black_box(&query), black_box(&text)).len());
        });
    }
    group.finish();
}

criterion_group!(benches, bench_find_words);
criterion_main!(benches);
