// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Testi derivati dai dati del motore, senza egui: posizioni, stato, errori, avvisi.
//!
//! Funzioni pure, testabili senza finestra; la vista le chiama e disegna il risultato.

use std::time::Duration;

use fluent_templates::fluent_bundle::FluentValue;
use lynkeus_core::{Location, MailPart, QueryError, TermError, column_name};
use lynkeus_engine::{LimitKind, Outcome, ProgressSnapshot, Summary, WarningKind};

use crate::app::{SearchStatus, StartError};
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
    }
}

/// Messaggio per un errore all'avvio della ricerca, da mostrare sotto il modulo.
pub fn start_error(i18n: &I18n, error: &StartError) -> String {
    match error {
        StartError::NoRoot => i18n.tr("error-no-folder"),
        StartError::Term(TermError::Empty) | StartError::Query(QueryError::NoTerms) => {
            i18n.tr("error-term-empty")
        }
        StartError::Term(TermError::OnlyWildcards) => i18n.tr("error-term-only-wildcards"),
        StartError::Term(TermError::TooLong { max_chars }) => {
            i18n.tr_args("error-term-too-long", &[("max", count(*max_chars))])
        }
        StartError::Term(TermError::LineBreak) => i18n.tr("error-term-line-break"),
        StartError::Query(QueryError::TooManyTerms { max }) => {
            i18n.tr_args("error-query-too-many", &[("max", count(*max))])
        }
        StartError::Query(
            QueryError::TooComplex { .. } | QueryError::WildcardModeMismatch { .. },
        ) => i18n.tr("error-query-too-complex"),
        StartError::Spawn(_) => i18n.tr("error-start-failed"),
    }
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
