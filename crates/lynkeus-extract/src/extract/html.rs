// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Pagine HTML, lette solo come caso innocuo (un .xls che è una pagina HTML, come lo
//! esportano molti gestionali).
//!
//! La codifica si riconosce come per i file di testo; l'HTML si riduce a testo con
//! `mail_parser::html_to_text` (lo stesso usato per i corpi HTML delle e-mail: niente
//! script né stili, entità risolte, blocchi su righe diverse). Un'unità per riga non vuota
//! del testo ridotto ([`Location::Paragraph`], numerate da 1).

use lynkeus_core::Location;

use crate::extract::{Ctx, Fail, Input};
use crate::text::TextReader;

/// Estrae le righe di testo di una pagina HTML.
pub(crate) fn extract(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let max = ctx.limits().max_entry_size;
    let mut reader = TextReader::new(input.into_reader(), max)?;
    let mut html = String::new();
    while let Some(chunk) = reader.next_chunk()? {
        html.push_str(chunk.text);
    }
    emit_lines(ctx, &mail_parser::html_to_text(&html))
}

/// Un'unità per riga non vuota di `text`, numerate da 1.
fn emit_lines(ctx: &mut Ctx<'_>, text: &str) -> Result<(), Fail> {
    let mut number = 0_u32;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        number = number.saturating_add(1);
        ctx.unit(Location::Paragraph(number), line)?;
    }
    Ok(())
}
