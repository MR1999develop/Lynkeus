// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Fogli di calcolo xlsx, xls e ods con `calamine`: un'unità per cella non vuota
//! ([`Location::Cell`] con il nome del foglio come etichetta), fogli nell'ordine del file.
//!
//! `calamine` legge le parti in memoria senza i nostri limiti, quindi prima si fa un
//! controllo preliminare dell'intero contenitore: ogni voce si decomprime con i limiti del
//! file radice ([`LimitedReader`](super::budget::LimitedReader)) e ogni XML si legge con il
//! nostro lettore (DTD rifiutati). Solo un contenitore che lo supera arriva a `calamine`.
//! Il costo è una decompressione in più.
//!
//! La memoria delle celle:
//!
//! - **xlsx**: le celle si leggono a flusso, nell'ordine del file. Un foglio con due celle
//!   lontanissime (A1 e XFD1048576) occupa la memoria di due celle, non del rettangolo che
//!   le contiene.
//! - **ods e xls**: `calamine` costruisce in memoria, per ogni foglio, il rettangolo tra la
//!   prima e l'ultima cella non vuota (con il testo di ogni cella, clonato per ogni
//!   ripetizione), prima che si possa leggere una cella; un'allocazione impossibile termina
//!   il processo. Prima di passargli il file se ne stima la memoria
//!   ([`sheet_memory`](super::sheet_memory)): oltre il massimo l'avviso è "memoria" e il file
//!   non si legge. Per xls `calamine` riceve un file composto rifatto con il solo flusso
//!   misurato, con le formule svuotate (il programma non le usa).
//!
//! Testi, numeri e date diventano testo (numeri nella forma di Rust, "2.5"); valori logici
//! ed errori di formula si saltano. Un foglio che non si legge fino in fondo dà, alla fine,
//! l'avviso "danneggiato" al documento (le sue celle già lette restano).

use std::io::{self, BufReader, Cursor};

use calamine::{Data, Ods, Reader, Xls, XlsError, Xlsx, XlsxError};
use lynkeus_core::Location;

use crate::error::ExtractError;
use crate::event::Stopped;
use crate::extract::archive::{Container, entry_label};
use crate::extract::budget::classify_io;
use crate::extract::cfbfile::{Compound, single_stream};
use crate::extract::opendoc::{ODS_MIME, check_package};
use crate::extract::sheet_memory::{measure_ods, measure_xls, neutralize_xls_formulas};
use crate::extract::xml::XmlReader;
use crate::extract::{Ctx, Fail, Input, Seekable};

const XLSX_MAIN: &str = "xl/workbook.xml";
const ODS_CONTENT: &str = "content.xml";
/// Flussi BIFF di un xls, nell'ordine in cui li cerca `calamine` (BIFF8, poi BIFF5).
const XLS_STREAMS: &[&str] = &["Workbook", "Book"];

/// Estrae le celle di un xlsx, a flusso.
pub(crate) fn extract_xlsx(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let mut container = Container::open(ctx, input)?;
    if !container.has(XLSX_MAIN) {
        return Err(ExtractError::ContentMismatch.into());
    }
    preflight(ctx, &mut container)?;
    let mut workbook: Xlsx<Seekable> =
        Xlsx::new(container.into_source()?).map_err(|_| corrupt())?;
    let mut damaged = false;
    for name in workbook.sheet_names() {
        damaged |= !xlsx_sheet(ctx, &mut workbook, &name)?;
    }
    if damaged {
        return Err(corrupt().into());
    }
    Ok(())
}

/// Manda le celle del foglio `name` man mano che si leggono; falso se il foglio non si è
/// letto fino in fondo. Un foglio che non è un foglio di lavoro (grafico) non ha celle.
fn xlsx_sheet(
    ctx: &mut Ctx<'_>,
    workbook: &mut Xlsx<Seekable>,
    name: &str,
) -> Result<bool, Stopped> {
    let mut reader = match workbook.worksheet_cells_reader(name) {
        Ok(reader) => reader,
        Err(XlsxError::NotAWorksheet(_)) => return Ok(true),
        Err(_) => return Ok(false),
    };
    let sheet = entry_label(name);
    loop {
        let cell = match reader.next_cell() {
            Ok(Some(cell)) => cell,
            Ok(None) => return Ok(true),
            Err(_) => return Ok(false),
        };
        let Some(text) = cell_text(&Data::from(cell.get_value().clone())) else {
            continue;
        };
        let (row, col) = cell.get_position();
        let location = Location::Cell {
            sheet: sheet.clone(),
            row: one_based(row, 0),
            col: one_based(col, 0),
        };
        ctx.unit(location, &text)?;
    }
}

/// Estrae le celle di un ods, dopo averne misurato i fogli.
pub(crate) fn extract_ods(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let mut container = Container::open(ctx, input)?;
    check_package(ctx, &mut container, ODS_MIME)?;
    preflight(ctx, &mut container)?;
    let Some(content) = container.part(ctx, ODS_CONTENT)? else {
        return Err(ExtractError::ContentMismatch.into());
    };
    measure_ods(BufReader::new(content))?.within_limit()?;
    let workbook: Ods<Seekable> = Ods::new(container.into_source()?).map_err(|_| corrupt())?;
    cells(ctx, workbook)
}

/// Estrae le celle di un xls (Excel 97-2003), dopo averne misurato i fogli.
///
/// Il file si legge in memoria entro il massimo per voce; il flusso BIFF, con le formule
/// svuotate, si misura e solo lui, in un file composto rifatto, arriva a `calamine`. Una
/// cartella di lavoro cifrata dà "protetto da password".
pub(crate) fn extract_xls(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let mut compound = Compound::open(ctx, input)?;
    let mut stream = None;
    for name in XLS_STREAMS {
        stream = compound.stream(ctx, name)?;
        if stream.is_some() {
            break;
        }
    }
    drop(compound);
    let Some(mut stream) = stream else {
        return Err(corrupt().into());
    };
    neutralize_xls_formulas(&mut stream);
    measure_xls(&stream)?.within_limit()?;
    let rebuilt = single_stream(XLS_STREAMS[0], &stream)?;
    drop(stream);
    let source = Seekable::Bytes(Cursor::new(rebuilt));
    let workbook: Xls<Seekable> = Xls::new(source).map_err(|error| match error {
        XlsError::Password => ExtractError::Encrypted,
        XlsError::Io(io) => classify_io(io),
        _ => corrupt(),
    })?;
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

/// Manda le celle di tutti i fogli (ods e xls, già misurati).
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
    use std::io::Write;

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;
    use crate::event::{EventSink, ExtractEvent, WarningKind};
    use crate::extract::sheet_memory::tests::{
        dimensions, mul_rk, number, ods_content, string_formula, string_value, workbook,
    };
    use crate::extract::{ExtractOptions, extract_bytes};
    use crate::limits::LimitKind;

    /// Le formule si svuotano prima di `calamine`, ma il loro valore resta: il testo di una
    /// formula (nel record STRING) si legge nella cella della formula.
    #[test]
    fn formula_values_survive_neutralization() {
        let stream = workbook(&[string_formula(1, 1), string_value("aragosta"), number(2, 0)]);
        let file = single_stream("Workbook", &stream).unwrap_or_default();
        let mut events = Vec::new();
        let options = ExtractOptions::default();
        assert_eq!(
            extract_bytes("formule.xls", file, &options, &mut events),
            Ok(())
        );
        let cells: Vec<(Location, String)> = events
            .into_iter()
            .filter_map(|e| match e {
                ExtractEvent::Unit(unit) => Some((unit.location, unit.text)),
                _ => None,
            })
            .collect();
        let cell = |row, col| Location::Cell {
            sheet: "Foglio".to_owned(),
            row,
            col,
        };
        assert_eq!(
            cells,
            [
                (cell(2, 2), "aragosta".to_owned()),
                (cell(3, 1), "1".to_owned())
            ]
        );
    }

    /// Conta le unità e tiene gli avvisi, senza tenere il testo.
    #[derive(Default)]
    struct CountUnits {
        units: usize,
        warnings: Vec<WarningKind>,
    }

    impl EventSink for CountUnits {
        fn emit(&mut self, event: ExtractEvent) -> Result<(), Stopped> {
            match event {
                ExtractEvent::Unit(_) => self.units += 1,
                ExtractEvent::Warning { kind, .. } => self.warnings.push(kind),
                _ => {}
            }
            Ok(())
        }
    }

    /// Un xls denso di 65.536 righe (tutte quelle di BIFF8) per 40 colonne, con le
    /// dimensioni dichiarate: sta nel limite di memoria e si legge per intero.
    #[test]
    fn dense_xls_is_read_whole() {
        let mut records = vec![dimensions((0, 65_536), (0, 40))];
        records.extend((0..=u16::MAX).map(|row| mul_rk(row, 40)));
        let file = single_stream("Workbook", &workbook(&records)).unwrap_or_default();
        let mut count = CountUnits::default();
        let options = ExtractOptions::default();
        assert_eq!(
            extract_bytes("denso.xls", file, &options, &mut count),
            Ok(())
        );
        assert!(count.warnings.is_empty(), "{:?}", count.warnings);
        assert_eq!(count.units, 65_536 * 40);
    }

    /// Un ods con `rows` come righe della sua unica tabella.
    fn ods_of(rows: &str) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        assert!(writer.start_file("mimetype", stored).is_ok());
        assert!(writer.write_all(ODS_MIME.as_bytes()).is_ok());
        let manifest = format!(
            r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="/" manifest:media-type="{ODS_MIME}"/></manifest:manifest>"#
        );
        assert!(
            writer
                .start_file("META-INF/manifest.xml", SimpleFileOptions::default())
                .is_ok()
        );
        assert!(writer.write_all(manifest.as_bytes()).is_ok());
        assert!(
            writer
                .start_file("content.xml", SimpleFileOptions::default())
                .is_ok()
        );
        assert!(writer.write_all(ods_content(rows).as_bytes()).is_ok());
        writer.finish().map(Cursor::into_inner).unwrap_or_default()
    }

    fn warnings(events: &[ExtractEvent]) -> Vec<WarningKind> {
        events
            .iter()
            .filter_map(|e| match e {
                ExtractEvent::Warning { kind, .. } => Some(*kind),
                _ => None,
            })
            .collect()
    }

    fn run(name: &str, bytes: Vec<u8>) -> Vec<ExtractEvent> {
        let mut events = Vec::new();
        let result = extract_bytes(name, bytes, &ExtractOptions::default(), &mut events);
        assert_eq!(result, Ok(()));
        events
    }

    /// A1 e una cella in riga 1000, colonna 16384: 16 milioni di celle nel rettangolo,
    /// centinaia di MB in `calamine`. Avviso "memoria" prima di leggerlo.
    #[test]
    fn sparse_ods_is_a_limit_not_a_crash() {
        let rows = concat!(
            r#"<table:table-row><table:table-cell office:value-type="string"><text:p>aragosta</text:p></table:table-cell></table:table-row>"#,
            r#"<table:table-row table:number-rows-repeated="998"><table:table-cell/></table:table-row>"#,
            r#"<table:table-row><table:table-cell table:number-columns-repeated="16383"/><table:table-cell office:value-type="string"><text:p>città</text:p></table:table-cell></table:table-row>"#,
        );
        let events = run("sparso.ods", ods_of(rows));
        assert_eq!(
            warnings(&events),
            [WarningKind::LimitExceeded(LimitKind::Memory)]
        );
        // Le stesse due celle vicine si leggono.
        let near = concat!(
            r#"<table:table-row><table:table-cell office:value-type="string"><text:p>aragosta</text:p></table:table-cell></table:table-row>"#,
            r#"<table:table-row table:number-rows-repeated="1048000"><table:table-cell table:number-columns-repeated="1024"/></table:table-row>"#,
        );
        let events = run("vicino.ods", ods_of(near));
        assert!(warnings(&events).is_empty(), "{events:?}");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ExtractEvent::Unit(u) if u.text == "aragosta"))
        );
    }

    /// Un xls con A1 e IV65536 dichiarati e scritti: 16 milioni di celle che `calamine`
    /// riserverebbe. Avviso "memoria", senza leggerlo.
    #[test]
    fn sparse_xls_is_a_limit_not_a_crash() {
        let stream = workbook(&[
            dimensions((0, 65_536), (0, 256)),
            number(0, 0),
            number(65_535, 255),
        ]);
        let file = single_stream("Workbook", &stream).unwrap_or_default();
        let events = run("sparso.xls", file);
        assert_eq!(
            warnings(&events),
            [WarningKind::LimitExceeded(LimitKind::Memory)]
        );
    }

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
