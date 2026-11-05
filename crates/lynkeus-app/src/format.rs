// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Testi derivati dai dati del motore, senza egui: posizioni, stato, errori, avvisi.
//!
//! Funzioni pure, testabili senza finestra; la vista le chiama e disegna il risultato.

use std::time::Duration;

use fluent_templates::fluent_bundle::FluentValue;
use lynkeus_core::{Location, MailPart, QueryError, TermError, column_name};
use lynkeus_engine::{
    ExtensionError, Hit, LimitKind, Outcome, ProgressSnapshot, Summary, WarningKind,
};

use crate::app::SearchStatus;
use crate::form::{ExtensionInputError, MAX_ROOTS, PathInputError, TermRowError};
use crate::i18n::I18n;

/// Carattere aggiunto dove una parola o un contesto è stato troncato.
pub const ELLIPSIS: &str = "…";

/// Posizione da mostrare: "riga 12", "pagina 3, riga 4", "Foglio1!B3".
pub fn location(i18n: &I18n, location: &Location) -> String {
    match location {
        Location::Line(line) => i18n.tr_args("location-line", &[("line", num(*line))]),
        Location::PageLine { page, line } => i18n.tr_args(
            "location-page-line",
            &[("page", num(*page)), ("line", num(*line))],
        ),
        Location::Cell { sheet, row, col } => i18n.tr_args(
            "location-cell",
            &[
                ("sheet", FluentValue::from(sheet.clone())),
                (
                    "cell",
                    FluentValue::from(format!("{}{row}", column_name(*col))),
                ),
            ],
        ),
        Location::Slide(slide) => i18n.tr_args("location-slide", &[("slide", num(*slide))]),
        Location::Paragraph(paragraph) => {
            i18n.tr_args("location-paragraph", &[("paragraph", num(*paragraph))])
        }
        Location::Mail { part, line } => i18n.tr_args(
            "location-mail",
            &[
                ("part", FluentValue::from(i18n.tr(mail_part_id(*part)))),
                ("line", num(*line)),
            ],
        ),
    }
}

/// ID Fluent del nome di una parte di e-mail.
pub fn mail_part_id(part: MailPart) -> &'static str {
    match part {
        MailPart::Subject => "mail-part-subject",
        MailPart::From => "mail-part-from",
        MailPart::To => "mail-part-to",
        MailPart::Body => "mail-part-body",
    }
}

/// ID Fluent del motivo di un avviso.
pub fn warning_id(kind: WarningKind) -> &'static str {
    match kind {
        WarningKind::NotFound => "warning-not-found",
        WarningKind::AccessDenied => "warning-access-denied",
        WarningKind::InUse => "warning-in-use",
        WarningKind::Io => "warning-io",
        WarningKind::Encrypted => "warning-encrypted",
        WarningKind::Corrupt => "warning-corrupt",
        WarningKind::Timeout => "warning-timeout",
        WarningKind::LimitExceeded(limit) => match limit {
            LimitKind::ArchiveDepth => "warning-limit-depth",
            LimitKind::ArchiveEntries => "warning-limit-entries",
            LimitKind::EntrySize => "warning-limit-entry-size",
            LimitKind::ArchiveSize => "warning-limit-archive-size",
            LimitKind::CompressionRatio => "warning-limit-ratio",
            LimitKind::Memory => "warning-limit-memory",
        },
        WarningKind::ContentMismatch => "warning-content-mismatch",
        WarningKind::NoText => "warning-no-text",
        WarningKind::ExtractorUnavailable => "warning-extractor-unavailable",
    }
}

/// Messaggio sotto una riga dei termini.
pub fn term_error(i18n: &I18n, error: TermRowError) -> String {
    match error {
        TermRowError::Invalid(TermError::Empty) => i18n.tr("error-term-empty"),
        TermRowError::Invalid(TermError::OnlyWildcards) => i18n.tr("error-term-only-wildcards"),
        TermRowError::Invalid(TermError::TooLong { max_chars }) => {
            i18n.tr_args("error-term-too-long", &[("max", count(max_chars))])
        }
        TermRowError::Invalid(TermError::LineBreak) => i18n.tr("error-term-line-break"),
        TermRowError::TooComplex => i18n.tr("error-query-too-complex"),
    }
}

/// Messaggio per un errore della query che non riguarda una riga.
pub fn query_error(i18n: &I18n, error: QueryError) -> String {
    match error {
        QueryError::TooManyTerms { max } => {
            i18n.tr_args("error-query-too-many", &[("max", count(max))])
        }
        QueryError::NoTerms => i18n.tr("error-term-empty"),
        QueryError::TooComplex { .. } | QueryError::WildcardModeMismatch { .. } => {
            i18n.tr("error-query-too-complex")
        }
    }
}

/// Messaggio sotto il campo delle estensioni.
pub fn extension_error(i18n: &I18n, error: &ExtensionInputError) -> String {
    match error {
        ExtensionInputError::Missing => i18n.tr("error-extensions-missing"),
        ExtensionInputError::Invalid { text, error } => {
            let text = FluentValue::from(text.clone());
            match error {
                ExtensionError::Empty | ExtensionError::InvalidChar => {
                    i18n.tr_args("error-extension-invalid", &[("extension", text)])
                }
                ExtensionError::TooLong { max_chars } => i18n.tr_args(
                    "error-extension-too-long",
                    &[("extension", text), ("max", count(*max_chars))],
                ),
            }
        }
    }
}

/// Messaggio sotto il campo del percorso.
pub fn path_error(i18n: &I18n, error: PathInputError) -> String {
    match error {
        PathInputError::NotFound => i18n.tr("error-path-not-found"),
        PathInputError::TooManyRoots => {
            i18n.tr_args("error-path-too-many-roots", &[("max", count(MAX_ROOTS))])
        }
    }
}

/// Testo della colonna Termine: i termini di `terms` (indici in `texts`), separati da
/// virgole.
pub fn matched_terms(texts: &[String], terms: lynkeus_core::TermSet) -> String {
    terms
        .iter()
        .filter_map(|index| texts.get(index))
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Testo della colonna Parola: la parola con "…" dal lato (o dai lati) in cui è stata
/// troncata.
pub fn word(hit: &Hit) -> String {
    let mut text = String::with_capacity(hit.word.len() + 2 * ELLIPSIS.len());
    if hit.word_truncated_start {
        text.push_str(ELLIPSIS);
    }
    text.push_str(&hit.word);
    if hit.word_truncated_end {
        text.push_str(ELLIPSIS);
    }
    text
}

/// Testo del contesto senza evidenziazione, con "…" dove la riga continua (per esportare e
/// copiare).
pub fn context_text(snippet: &lynkeus_core::Snippet) -> String {
    let mut text = String::with_capacity(snippet.text.len() + 2 * ELLIPSIS.len());
    if snippet.truncated_start {
        text.push_str(ELLIPSIS);
    }
    text.push_str(&snippet.text);
    if snippet.truncated_end {
        text.push_str(ELLIPSIS);
    }
    text
}

/// Riga di stato.
pub fn status(i18n: &I18n, status: &SearchStatus, max_results: usize) -> String {
    match status {
        SearchStatus::Idle => i18n.tr("status-idle"),
        SearchStatus::Running { counts, elapsed } => {
            i18n.tr_args("status-running", &counts_args(counts, *elapsed))
        }
        SearchStatus::Finished(summary) => finished(i18n, summary, max_results),
        SearchStatus::WorkerLost => i18n.tr("status-worker-lost"),
    }
}

fn finished(i18n: &I18n, summary: &Summary, max_results: usize) -> String {
    let args = counts_args(&summary.counts, summary.elapsed);
    match summary.outcome {
        Outcome::Completed => i18n.tr_args("status-completed", &args),
        Outcome::Cancelled => i18n.tr_args("status-cancelled", &args),
        Outcome::LimitReached => {
            i18n.tr_args("status-limit-reached", &[("max", count(max_results))])
        }
    }
}

fn counts_args(
    counts: &ProgressSnapshot,
    elapsed: Duration,
) -> [(&'static str, FluentValue<'static>); 6] {
    [
        ("searched", FluentValue::from(counts.files_searched)),
        ("hits", FluentValue::from(counts.hits)),
        ("skipped", FluentValue::from(counts.files_skipped)),
        ("binary", FluentValue::from(counts.binary_skipped)),
        ("warnings", FluentValue::from(counts.warnings)),
        ("seconds", FluentValue::from(elapsed.as_secs())),
    ]
}

fn num(value: u32) -> FluentValue<'static> {
    FluentValue::from(value)
}

fn count(value: usize) -> FluentValue<'static> {
    FluentValue::from(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    fn italian() -> I18n {
        I18n::new(Language::Italian)
    }

    /// Toglie i caratteri d'isolamento bidi che Fluent mette intorno alle variabili.
    fn plain(text: &str) -> String {
        text.replace(['\u{2068}', '\u{2069}'], "")
    }

    #[test]
    fn word_shows_the_ellipsis_on_the_truncated_side() {
        use std::sync::Arc;

        use lynkeus_core::{Snippet, TermSet};
        use lynkeus_engine::DocPath;

        let hit = |start: bool, end: bool| Hit {
            doc: Arc::new(DocPath::file("a.txt")),
            root: 0,
            location: Location::Line(1),
            word: "carta".to_owned(),
            word_truncated_start: start,
            word_truncated_end: end,
            snippet: Snippet {
                text: "carta".to_owned(),
                highlights: Vec::new(),
                truncated_start: false,
                truncated_end: false,
            },
            terms: TermSet::single(0),
        };
        assert_eq!(word(&hit(false, false)), "carta");
        assert_eq!(word(&hit(true, false)), "…carta");
        assert_eq!(word(&hit(false, true)), "carta…");
        assert_eq!(word(&hit(true, true)), "…carta…");
    }

    #[test]
    fn locations() {
        let it = italian();
        assert_eq!(plain(&location(&it, &Location::Line(12))), "riga 12");
        let cell = Location::Cell {
            sheet: "Foglio1".to_owned(),
            row: 3,
            col: 2,
        };
        assert_eq!(plain(&location(&it, &cell)), "Foglio1!B3");
        let mail = Location::Mail {
            part: MailPart::Subject,
            line: 1,
        };
        assert_eq!(plain(&location(&it, &mail)), "oggetto, riga 1");
    }

    #[test]
    fn every_warning_and_error_has_a_message() {
        let kinds = [
            WarningKind::NotFound,
            WarningKind::AccessDenied,
            WarningKind::InUse,
            WarningKind::Io,
            WarningKind::Encrypted,
            WarningKind::Corrupt,
            WarningKind::Timeout,
            WarningKind::LimitExceeded(LimitKind::ArchiveDepth),
            WarningKind::LimitExceeded(LimitKind::ArchiveEntries),
            WarningKind::LimitExceeded(LimitKind::EntrySize),
            WarningKind::LimitExceeded(LimitKind::ArchiveSize),
            WarningKind::LimitExceeded(LimitKind::CompressionRatio),
            WarningKind::LimitExceeded(LimitKind::Memory),
            WarningKind::ContentMismatch,
            WarningKind::NoText,
            WarningKind::ExtractorUnavailable,
        ];
        for language in Language::ALL {
            let i18n = I18n::new(language);
            for kind in kinds {
                assert!(
                    i18n.try_tr_args(warning_id(kind), &[]).is_some(),
                    "{kind:?}"
                );
            }
            for part in [
                MailPart::Subject,
                MailPart::From,
                MailPart::To,
                MailPart::Body,
            ] {
                assert!(i18n.try_tr_args(mail_part_id(part), &[]).is_some());
            }
        }
    }

    #[test]
    fn plural_in_status() {
        let it = italian();
        let counts = ProgressSnapshot {
            files_searched: 1,
            hits: 2,
            ..ProgressSnapshot::default()
        };
        let running = SearchStatus::Running {
            counts,
            elapsed: Duration::from_secs(3),
        };
        let text = plain(&status(&it, &running, 10));
        assert!(text.contains("1 file letto"), "{text}");
        assert!(text.contains("2 parole trovate"), "{text}");
    }
}
