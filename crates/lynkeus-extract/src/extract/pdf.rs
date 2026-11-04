// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! PDF con `pdf_oxide` ed `extract_text_lines`, una pagina alla volta (ADR 0002).
//!
//! 1. `PdfDocument::from_bytes`: un errore è "danneggiato". I byte si leggono entro il
//!    massimo per voce; la libreria ne tiene una seconda copia.
//! 2. Cifrato e non apribile con la password vuota: "protetto da password" (altrimenti
//!    sembrerebbe un PDF senza testo).
//! 3. Per ogni pagina un'unità [`Location::PageLine`] con riga 1 e una riga del testo per
//!    ogni riga non vuota della pagina: la k-esima riga dell'unità è la riga k+1 della
//!    pagina ([`line_location`](crate::line_location)). Un a capo dentro una riga diventa
//!    uno spazio, così la numerazione resta giusta. Tra una pagina e l'altra chi riceve
//!    gli eventi può fermare l'estrazione.
//! 4. Una pagina con errore si salta; alla fine il documento ha l'avviso "danneggiato".
//! 5. Nessun carattere in nessuna pagina: "PDF senza testo" (probabile scansione).
//!
//! `extract_text`, `to_plain_text` e simili sono vietati da `clippy.toml`: sui PDF con
//! struttura (quelli di LibreOffice) incollano i paragrafi.

use lynkeus_core::Location;
use pdf_oxide::PdfDocument;

use crate::error::ExtractError;
use crate::extract::{Ctx, Fail, Input};

/// Estrae le righe di ogni pagina.
pub(crate) fn extract(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let bytes = input.into_bytes(ctx)?;
    let doc = PdfDocument::from_bytes(bytes).map_err(|_| corrupt())?;
    if doc.is_encrypted() && !doc.authenticate(b"").unwrap_or(false) {
        return Err(ExtractError::Encrypted.into());
    }
    let pages = doc.page_count().map_err(|_| corrupt())?;
    let mut damaged = false;
    let mut any_text = false;
    for index in 0..pages {
        let Ok(lines) = doc.extract_text_lines(index) else {
            damaged = true;
            continue;
        };
        let text = page_text(lines.iter().map(|line| line.text.as_str()));
        if text.is_empty() {
            continue;
        }
        any_text = true;
        let page = u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX);
        ctx.unit(Location::PageLine { page, line: 1 }, &text)?;
    }
    if damaged {
        return Err(corrupt().into());
    }
    if !any_text {
        return Err(ExtractError::NoText.into());
    }
    Ok(())
}

/// Le righe non vuote di una pagina, una per riga, senza a capo interni.
fn page_text<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    let mut text = String::new();
    for line in lines.filter(|line| !line.trim().is_empty()) {
        if !text.is_empty() {
            text.push('\n');
        }
        text.extend(
            line.chars()
                .map(|c| if c == '\n' || c == '\r' { ' ' } else { c }),
        );
    }
    text
}

fn corrupt() -> ExtractError {
    ExtractError::Corrupt {
        reason: "PDF non valido".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_text_skips_empty_lines_and_inner_breaks() {
        let lines = ["Titolo", "   ", "riga\ncon a capo", "", "fine\r"];
        assert_eq!(
            page_text(lines.into_iter()),
            "Titolo\nriga con a capo\nfine "
        );
        assert_eq!(page_text(["", " "].into_iter()), "");
    }
}
