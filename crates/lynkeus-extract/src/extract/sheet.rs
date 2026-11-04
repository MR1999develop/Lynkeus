// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Fogli di calcolo xlsx e ods con `calamine`: un'unità per cella non vuota
//! ([`Location::Cell`] con il nome del foglio come etichetta), fogli nell'ordine del file.
//!
//! `calamine` legge le parti in memoria senza i nostri limiti, quindi prima si fa un
//! controllo preliminare dell'intero contenitore: ogni voce si decomprime con i limiti del
//! file radice ([`LimitedReader`](super::budget::LimitedReader)) e ogni XML si legge con il
//! nostro lettore (DTD rifiutati). Solo un contenitore che lo supera arriva a `calamine`.
//! Il costo è una decompressione in più.
//!
//! Testi, numeri e date diventano testo (numeri nella forma di Rust, "2.5"); valori logici
//! ed errori di formula si saltano. Un foglio illeggibile si salta e, alla fine, il
//! documento ha l'avviso "danneggiato".

use std::io::{self, BufReader};

use calamine::{Data, Ods, Reader, Xlsx};
use lynkeus_core::Location;

use crate::error::ExtractError;
use crate::extract::archive::{Container, entry_label};
use crate::extract::budget::classify_io;
use crate::extract::opendoc::{ODS_MIME, check_package};
use crate::extract::xml::XmlReader;
use crate::extract::{Ctx, Fail, Input, Seekable};

const XLSX_MAIN: &str = "xl/workbook.xml";

/// Estrae le celle di un xlsx.
pub(crate) fn extract_xlsx(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let mut container = Container::open(ctx, input)?;
    if !container.has(XLSX_MAIN) {
        return Err(ExtractError::ContentMismatch.into());
    }
    preflight(ctx, &mut container)?;
    let workbook: Xlsx<Seekable> = Xlsx::new(container.into_source()?).map_err(|_| corrupt())?;
    cells(ctx, workbook)
}

/// Estrae le celle di un ods.
pub(crate) fn extract_ods(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let mut container = Container::open(ctx, input)?;
    check_package(ctx, &mut container, ODS_MIME)?;
    preflight(ctx, &mut container)?;
    let workbook: Ods<Seekable> = Ods::new(container.into_source()?).map_err(|_| corrupt())?;
    cells(ctx, workbook)
}

/// Decomprime ogni voce con i limiti e valida ogni XML, senza tenere nulla.
fn preflight(ctx: &Ctx<'_>, container: &mut Container) -> Result<(), ExtractError> {
    for name in container.names() {
        if name.ends_with('/') {
            continue;
        }
        let Some(mut part) = container.part(ctx, &name)? else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".xml") || lower.ends_with(".rels") {
            XmlReader::new(BufReader::new(part)).validate()?;
        } else {
            io::copy(&mut part, &mut io::sink()).map_err(classify_io)?;
        }
    }
    Ok(())
}

/// Manda le celle di tutti i fogli.
fn cells<R: Reader<Seekable>>(ctx: &mut Ctx<'_>, mut workbook: R) -> Result<(), Fail> {
    let mut damaged = false;
    for name in workbook.sheet_names() {
        let Ok(range) = workbook.worksheet_range(&name) else {
            damaged = true;
            continue;
        };
        let Some((first_row, first_col)) = range.start() else {
            continue;
        };
        let sheet = entry_label(&name);
        for (row, col, data) in range.used_cells() {
            let Some(text) = cell_text(data) else {
                continue;
            };
            let location = Location::Cell {
                sheet: sheet.clone(),
                row: one_based(first_row, row),
                col: one_based(first_col, col),
            };
            ctx.unit(location, &text)?;
        }
    }
    if damaged {
        return Err(corrupt().into());
    }
    Ok(())
}

/// Indice 1-based di una riga o colonna: inizio dell'area più posizione relativa, più uno.
fn one_based(start: u32, offset: usize) -> u32 {
    u32::try_from(offset)
        .unwrap_or(u32::MAX)
        .saturating_add(start)
        .saturating_add(1)
}

/// Il testo di una cella, o `None` per le celle da saltare.
fn cell_text(data: &Data) -> Option<String> {
    match data {
        Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => Some(s.clone()),
        Data::Int(i) => Some(i.to_string()),
        Data::Float(f) => Some(f.to_string()),
        Data::DateTime(d) => Some(d.to_string()),
        Data::Bool(_) | Data::Error(_) | Data::Empty => None,
    }
}

fn corrupt() -> ExtractError {
    ExtractError::Corrupt {
        reason: "foglio di calcolo non valido".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_text_partitions() {
        assert_eq!(
            cell_text(&Data::String("città".to_owned())).as_deref(),
            Some("città")
        );
        assert_eq!(cell_text(&Data::Int(2026)).as_deref(), Some("2026"));
        assert_eq!(cell_text(&Data::Float(2.5)).as_deref(), Some("2.5"));
        assert_eq!(cell_text(&Data::Float(3.0)).as_deref(), Some("3"));
        assert_eq!(cell_text(&Data::Bool(true)), None);
        assert_eq!(cell_text(&Data::Empty), None);
    }

    #[test]
    fn one_based_saturates() {
        assert_eq!(one_based(0, 0), 1);
        assert_eq!(one_based(5, 1), 7);
        assert_eq!(one_based(u32::MAX, 3), u32::MAX);
    }
}
