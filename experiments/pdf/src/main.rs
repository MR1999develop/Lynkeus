// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Prototipo usa e getta: confronta `pdf_oxide` e `pdf-extract` sull'estrazione del testo per pagina.
//!
//! Comandi:
//! - `one <oxide|oxide-lines|extract> <file>`: estrae un file con una libreria e stampa testo per
//!   pagina, tempi e memoria di picco del processo (un processo per misura, così la memoria è pulita);
//! - `run <file>...`: per ogni file e libreria avvia `one` in un processo figlio con un timeout,
//!   raccoglie esito, tempo e memoria e stampa una tabella; i testi vanno in `<cartella>/<lib>-<file>.txt`
//!   se è impostata la variabile `PDF_EXP_OUT`;
//! - `gen <cartella>`: scrive PDF sintetici ostili (vuoto, intestazione con spazzatura, ciclo nel
//!   page tree, bomba Flate, annidamento profondo, conteggi falsi).

mod generate;

use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

/// Limite di pagine visitate per file: protegge il prototipo da `/Count` falsi.
const MAX_PAGES: usize = 10_000;
/// Tempo massimo di un'estrazione nel comando `run`.
const TIMEOUT: Duration = Duration::from_secs(60);
/// Librerie (e varianti d'uso) confrontate.
const LIBS: [&str; 4] = ["oxide", "oxide-lines", "oxide-plain", "extract"];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("one") if args.len() == 4 => one(&args[2], Path::new(&args[3])),
        Some("run") if args.len() >= 3 => run(&args[2..]),
        Some("gen") if args.len() == 3 => match generate::write_all(Path::new(&args[2])) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("gen: {e}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!(
                "uso: pdf-experiment one <oxide|oxide-lines|oxide-plain|extract> <file> | run <file>... | gen <cartella>"
            );
            ExitCode::from(2)
        }
    }
}

/// Esito dell'estrazione di un file con una libreria.
struct Extraction {
    open_ms: f64,
    pages: Vec<PageText>,
    page_count_error: Option<String>,
}

struct PageText {
    text: Result<String, String>,
    ms: f64,
}

/// Estrae `path` con `lib` nel processo corrente e stampa il rapporto su stdout.
fn one(lib: &str, path: &Path) -> ExitCode {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            println!("RESULT io-error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let start = Instant::now();
    let outcome = catch_unwind(AssertUnwindSafe(|| match lib {
        "oxide" => extract_oxide(bytes, OxideMode::Text),
        "oxide-lines" => extract_oxide(bytes, OxideMode::Lines),
        "oxide-plain" => extract_oxide(bytes, OxideMode::Plain),
        "extract" => extract_pdf_extract(&bytes),
        other => Err(format!("libreria sconosciuta: {other}")),
    }));
    let total_ms = ms(start.elapsed());
    let mut out = std::io::stdout().lock();
    let result = match outcome {
        Err(panic) => format!("panic: {}", panic_message(&*panic)),
        Ok(Err(e)) => format!("error: {e}"),
        Ok(Ok(ex)) => {
            let _ = writeln!(out, "OPEN_MS {:.2}", ex.open_ms);
            let _ = writeln!(out, "PAGES {}", ex.pages.len());
            if let Some(e) = &ex.page_count_error {
                let _ = writeln!(out, "PAGE_COUNT_ERROR {e}");
            }
            let mut page_errors = 0usize;
            let mut chars = 0usize;
            for (i, page) in ex.pages.iter().enumerate() {
                match &page.text {
                    Ok(text) => {
                        chars += text.chars().count();
                        let _ = writeln!(
                            out,
                            "PAGE {} ms={:.2} chars={}",
                            i + 1,
                            page.ms,
                            text.chars().count()
                        );
                        for line in text.lines() {
                            let _ = writeln!(out, "  | {line}");
                        }
                    }
                    Err(e) => {
                        page_errors += 1;
                        let _ = writeln!(out, "PAGE {} ms={:.2} ERROR {e}", i + 1, page.ms);
                    }
                }
            }
            format!(
                "ok pages={} page_errors={page_errors} chars={chars}",
                ex.pages.len()
            )
        }
    };
    let _ = writeln!(out, "TOTAL_MS {total_ms:.2}");
    let (peak_commit, peak_ws) = peak_memory_mib();
    let _ = writeln!(out, "PEAK_COMMIT_MIB {peak_commit:.1}");
    let _ = writeln!(out, "PEAK_WS_MIB {peak_ws:.1}");
    let _ = writeln!(out, "RESULT {result}");
    ExitCode::SUCCESS
}

/// API di `pdf_oxide` usata per il testo di una pagina.
#[derive(Clone, Copy)]
enum OxideMode {
    /// `extract_text`: ordine dell'albero di struttura se il PDF è "tagged".
    Text,
    /// `extract_text_lines`: righe raggruppate per geometria, con riquadro.
    Lines,
    /// `to_plain_text` con le opzioni predefinite.
    Plain,
}

/// `pdf_oxide`: apertura dai byte, conteggio delle pagine, poi una pagina alla volta.
fn extract_oxide(bytes: Vec<u8>, mode: OxideMode) -> Result<Extraction, String> {
    let start = Instant::now();
    let doc = pdf_oxide::PdfDocument::from_bytes(bytes).map_err(|e| e.to_string())?;
    let open_ms = ms(start.elapsed());
    // Un PDF cifrato con password d'apertura dà testo vuoto senza errore: si distingue così.
    if doc.is_encrypted() {
        let readable = doc.authenticate(b"").map_err(|e| e.to_string())?;
        println!("ENCRYPTED leggibile_con_password_vuota={readable}");
    }
    let (count, page_count_error) = match doc.page_count() {
        Ok(n) => (n, None),
        Err(e) => (0, Some(e.to_string())),
    };
    // Soglia orizzontale (punti PDF) tra due parole per `extract_text_lines`; `None` = adattiva.
    let word_gap: Option<f32> = std::env::var("PDF_EXP_WORD_GAP")
        .ok()
        .and_then(|v| v.parse().ok());
    // Profilo di estrazione degli span (`CONSERVATIVE`, `BALANCED`, ...); `None` = predefinito.
    let profile = std::env::var("PDF_EXP_PROFILE")
        .ok()
        .and_then(|n| pdf_oxide::config::ExtractionProfile::by_name(&n));
    let mut pages = Vec::new();
    for i in 0..count.min(MAX_PAGES) {
        let t = Instant::now();
        let text = match mode {
            OxideMode::Lines => doc
                .extract_text_lines_with_thresholds(i, word_gap, None, profile.clone())
                .map(|ls| {
                    ls.into_iter()
                        .map(|l| l.text)
                        .collect::<Vec<_>>()
                        .join("\n")
                }),
            OxideMode::Text => doc.extract_text(i),
            OxideMode::Plain => {
                doc.to_plain_text(i, &pdf_oxide::converters::ConversionOptions::default())
            }
        };
        pages.push(PageText {
            text: text.map_err(|e| e.to_string()),
            ms: ms(t.elapsed()),
        });
    }
    Ok(Extraction {
        open_ms,
        pages,
        page_count_error,
    })
}

/// `pdf-extract`: l'API pubblica estrae tutte le pagine in un colpo e restituisce un `Vec` per pagina.
fn extract_pdf_extract(bytes: &[u8]) -> Result<Extraction, String> {
    let start = Instant::now();
    let texts = pdf_extract::extract_text_from_mem_by_pages(bytes).map_err(|e| e.to_string())?;
    let elapsed = ms(start.elapsed());
    let n = texts.len().max(1);
    #[expect(clippy::cast_precision_loss, reason = "prototipo: media indicativa")]
    let per_page = elapsed / n as f64;
    let pages = texts
        .into_iter()
        .take(MAX_PAGES)
        .map(|t| PageText {
            text: Ok(t),
            ms: per_page,
        })
        .collect();
    Ok(Extraction {
        open_ms: 0.0,
        pages,
        page_count_error: None,
    })
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "(payload non testuale)".to_owned()
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Memoria di picco del processo corrente in MiB: (commit privato, working set).
#[cfg(windows)]
#[expect(unsafe_code, reason = "prototipo: GetProcessMemoryInfo")]
fn peak_memory_mib() -> (f64, f64) {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    let size = u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).unwrap_or(0);
    counters.cb = size;
    // SAFETY: `counters` è una struttura valida di `cb` byte; lo pseudo-handle del processo corrente
    // non va chiuso.
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &raw mut counters, size) };
    if ok == 0 {
        return (f64::NAN, f64::NAN);
    }
    #[expect(clippy::cast_precision_loss, reason = "MiB indicativi")]
    let mib = |b: usize| b as f64 / 1_048_576.0;
    (
        mib(counters.PeakPagefileUsage),
        mib(counters.PeakWorkingSetSize),
    )
}

#[cfg(not(windows))]
fn peak_memory_mib() -> (f64, f64) {
    (f64::NAN, f64::NAN)
}

/// Esegue `one` per ogni file e libreria in processi figli con timeout e stampa una tabella.
fn run(files: &[String]) -> ExitCode {
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("current_exe non disponibile");
        return ExitCode::FAILURE;
    };
    let out_dir = std::env::var_os("PDF_EXP_OUT").map(PathBuf::from);
    println!(
        "| file | libreria | esito | pagine | caratteri | tempo totale ms | commit di picco MiB | working set di picco MiB |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    for file in files {
        let name = Path::new(file)
            .file_name()
            .map_or_else(|| file.clone(), |n| n.to_string_lossy().into_owned());
        for lib in LIBS {
            let start = Instant::now();
            let report = run_child(&exe, lib, file);
            let wall = ms(start.elapsed());
            let (status, stdout) = match report {
                Ok(r) => r,
                Err(e) => (e, String::new()),
            };
            if let Some(dir) = &out_dir {
                let _ = std::fs::write(dir.join(format!("{lib}-{name}.txt")), &stdout);
            }
            let field = |key: &str| {
                stdout
                    .lines()
                    .find_map(|l| l.strip_prefix(key).map(|v| v.trim().to_owned()))
                    .unwrap_or_else(|| "-".to_owned())
            };
            let result = field("RESULT ");
            let summary = if result == "-" {
                status.clone()
            } else {
                result
            };
            let pages = field("PAGES ");
            let chars = summary
                .split_whitespace()
                .find_map(|w| w.strip_prefix("chars="))
                .unwrap_or("-")
                .to_owned();
            println!(
                "| {name} | {lib} | {} | {pages} | {chars} | {} (processo {wall:.0}) | {} | {} |",
                summary
                    .split(" pages=")
                    .next()
                    .unwrap_or(&summary)
                    .chars()
                    .take(90)
                    .collect::<String>(),
                field("TOTAL_MS "),
                field("PEAK_COMMIT_MIB "),
                field("PEAK_WS_MIB "),
            );
        }
    }
    ExitCode::SUCCESS
}

/// Avvia `one` e aspetta al massimo [`TIMEOUT`]; restituisce lo stato d'uscita e lo stdout.
fn run_child(exe: &Path, lib: &str, file: &str) -> Result<(String, String), String> {
    let mut child = Command::new(exe)
        .args(["one", lib, file])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("stdout assente")?;
    let reader = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = stdout.read_to_end(&mut s);
        String::from_utf8_lossy(&s).into_owned()
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        // Prototipo: attesa a intervalli; il prodotto userà l'attesa sull'handle del processo.
        match child.try_wait() {
            Ok(Some(st)) => break format!("uscita {st}"),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break format!("timeout {} s", TIMEOUT.as_secs());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => break format!("wait: {e}"),
        }
    };
    let text = reader.join().unwrap_or_default();
    Ok((status, text))
}
