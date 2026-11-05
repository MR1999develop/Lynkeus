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
//!   prima e l'ultima cella non vuota (e per xls riserva anche le dimensioni dichiarate nel
//!   file), prima che si possa leggere una cella; un'allocazione impossibile termina il
//!   processo. Prima di passargli il file lo si misura con un lettore proprio (l'XML di
//!   `content.xml`, i record BIFF del flusso `Workbook`): oltre [`MAX_WORKBOOK_CELLS`] celle
//!   l'avviso è "memoria" e il file non si legge. Per xls `calamine` riceve un file composto
//!   rifatto con il solo flusso misurato.
//!
//! Testi, numeri e date diventano testo (numeri nella forma di Rust, "2.5"); valori logici
//! ed errori di formula si saltano. Un foglio che non si legge fino in fondo dà, alla fine,
//! l'avviso "danneggiato" al documento (le sue celle già lette restano).

use std::io::{self, BufRead, BufReader, Cursor};

use calamine::{Data, Ods, Reader, Xls, XlsError, Xlsx, XlsxError};
use lynkeus_core::Location;

use crate::error::ExtractError;
use crate::event::Stopped;
use crate::extract::archive::{Container, entry_label};
use crate::extract::budget::classify_io;
use crate::extract::cfbfile::{Compound, single_stream, to_usize, u16_at, u32_at};
use crate::extract::opendoc::{ODS_MIME, check_package};
use crate::extract::xml::{Element, XmlEvent, XmlReader};
use crate::extract::{Ctx, Fail, Input, Seekable};
use crate::limits::LimitKind;

const XLSX_MAIN: &str = "xl/workbook.xml";
const ODS_CONTENT: &str = "content.xml";
/// Flussi BIFF di un xls, nell'ordine in cui li cerca `calamine` (BIFF8, poi BIFF5).
const XLS_STREAMS: &[&str] = &["Workbook", "Book"];

/// Celle che `calamine` può tenere in memoria per un file ods o xls, sommate su tutti i
/// fogli (rettangoli, celle lette, dimensioni dichiarate). Una cella occupa circa 24-32
/// byte: con il limite di memoria del processo estrattore (1 GiB) restano centinaia di MB
/// per il resto. Un foglio vero di 100.000 righe per 40 colonne ci sta.
pub(crate) const MAX_WORKBOOK_CELLS: u64 = 4 * 1024 * 1024;

/// Colonne al più in una riga di ods: `calamine` tronca lì le ripetizioni.
const ODS_MAX_COLUMNS: u64 = 16_384;

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
    within_cell_limit(ods_cells(BufReader::new(content))?)?;
    let workbook: Ods<Seekable> = Ods::new(container.into_source()?).map_err(|_| corrupt())?;
    cells(ctx, workbook)
}

/// Estrae le celle di un xls (Excel 97-2003), dopo averne misurato i fogli.
///
/// Il file si legge in memoria entro il massimo per voce; il flusso BIFF si misura e solo
/// lui, in un file composto rifatto, arriva a `calamine`. Una cartella di lavoro cifrata dà
/// "protetto da password".
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
    let Some(stream) = stream else {
        return Err(corrupt().into());
    };
    within_cell_limit(xls_cells(&stream)?)?;
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

/// Errore "memoria" se `cells` supera [`MAX_WORKBOOK_CELLS`].
fn within_cell_limit(cells: u64) -> Result<(), ExtractError> {
    if cells > MAX_WORKBOOK_CELLS {
        return Err(ExtractError::LimitExceeded(LimitKind::Memory));
    }
    Ok(())
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

/// Il rettangolo delle celle non vuote di un foglio, in righe e colonne 0-based.
#[derive(Debug, Default, Clone, Copy)]
struct Extent {
    rows: Option<(u64, u64)>,
    cols: Option<(u64, u64)>,
}

impl Extent {
    /// Allarga il rettangolo alle righe `rows` e alle colonne `cols` (estremi compresi).
    fn add(&mut self, rows: (u64, u64), cols: (u64, u64)) {
        let widen = |current: Option<(u64, u64)>, (low, high): (u64, u64)| {
            Some(current.map_or((low, high), |(a, b)| (a.min(low), b.max(high))))
        };
        self.rows = widen(self.rows, rows);
        self.cols = widen(self.cols, cols);
    }

    /// Le celle del rettangolo.
    fn cells(&self) -> u64 {
        match (self.rows, self.cols) {
            (Some((top, bottom)), Some((left, right))) => bottom
                .saturating_sub(top)
                .saturating_add(1)
                .saturating_mul(right.saturating_sub(left).saturating_add(1)),
            _ => 0,
        }
    }
}

/// Le celle che `calamine` terrebbe in memoria per il `content.xml` di un ods: per ogni
/// tabella il rettangolo delle celle con un valore (ripetizioni di righe e colonne
/// comprese), più le celle che legge riga per riga fino all'ultima con un valore, più gli
/// spazi ripetuti (`text:s`), contati come celle ogni [`ODS_SPACES_PER_CELL`].
///
/// Prudente: elementi e attributi si riconoscono dal nome locale in qualunque namespace
/// (`calamine` li riconosce dal prefisso) e le ripetizioni prendono il valore più grande.
fn ods_cells<R: BufRead>(source: R) -> Result<u64, ExtractError> {
    let mut xml = XmlReader::new(source);
    let mut tables: Vec<Extent> = Vec::new();
    let mut total = 0_u64;
    let mut spaces = 0_u64;
    let mut row = 0_u64;
    let mut row_repeat = 1_u64;
    let mut col = 0_u64;
    let mut row_width = 0_u64;
    loop {
        let event = xml.next()?;
        let (XmlEvent::Open(e) | XmlEvent::Empty(e)) = &event else {
            match event {
                XmlEvent::Close(_, local) if local == "table" => {
                    let area = tables.pop().map_or(0, |t| t.cells());
                    total = total.saturating_add(area);
                }
                XmlEvent::Close(_, local) if local == "table-row" => {
                    total = total.saturating_add(row_width.min(ODS_MAX_COLUMNS));
                    row = row.saturating_add(row_repeat);
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            continue;
        };
        let open = matches!(event, XmlEvent::Open(_));
        match e.local.as_str() {
            "table" if open => {
                tables.push(Extent::default());
                row = 0;
            }
            "table-row" if open => {
                row_repeat = repeats(e, "number-rows-repeated");
                col = 0;
                row_width = 0;
            }
            "table-row" => row = row.saturating_add(repeats(e, "number-rows-repeated")),
            "table-cell" | "covered-table-cell" => {
                let width = repeats(e, "number-columns-repeated");
                if may_have_value(e) {
                    let last_col = col.saturating_add(width.saturating_sub(1));
                    let last_row = row.saturating_add(row_repeat.saturating_sub(1));
                    if let Some(table) = tables.last_mut() {
                        table.add((row, last_row), (col, last_col));
                    }
                    row_width = last_col.saturating_add(1);
                }
                col = col.saturating_add(width);
            }
            "s" => spaces = spaces.saturating_add(repeats(e, "c")),
            _ => {}
        }
    }
    for table in tables {
        total = total.saturating_add(table.cells());
    }
    Ok(total.saturating_add(spaces / ODS_SPACES_PER_CELL))
}

/// Spazi ripetuti di `text:s` che valgono una cella: `calamine` li scrive uno per uno nel
/// testo della cella.
const ODS_SPACES_PER_CELL: u64 = 32;

/// Gli attributi con cui `calamine` dà un valore a una cella di ods.
const ODS_VALUE_ATTRIBUTES: &[&str] = &[
    "value-type",
    "value",
    "string-value",
    "date-value",
    "time-value",
    "boolean-value",
    "formula",
];

/// Vero se la cella può avere un valore (tipo, valore o formula): le altre hanno solo
/// stile e ripetizioni, e `calamine` non le tiene.
fn may_have_value(e: &Element) -> bool {
    ODS_VALUE_ATTRIBUTES
        .iter()
        .any(|local| e.attrs_named(local).next().is_some())
}

/// La ripetizione più grande dichiarata con l'attributo `local`, almeno 1.
fn repeats(e: &Element, local: &str) -> u64 {
    e.attrs_named(local)
        .filter_map(|value| value.trim().parse::<u64>().ok())
        .max()
        .unwrap_or(1)
        .max(1)
}

/// Tipi dei record BIFF che contano.
const BIFF_EOF: u16 = 0x000A;
const BIFF_BOF: u16 = 0x0809;
const BIFF_BOUND_SHEET: u16 = 0x0085;
const BIFF_FILE_PASS: u16 = 0x002F;
const BIFF_DIMENSIONS: u16 = 0x0200;
const BIFF_MUL_RK: u16 = 0x00BD;
/// Record di una cella con riga e colonna nei primi quattro byte, quelli che `calamine`
/// legge: NUMBER, LABEL, RSTRING, BOOLERR, RK, LABELSST, FORMULA.
const BIFF_CELLS: &[u16] = &[0x0203, 0x0204, 0x00D6, 0x0205, 0x027E, 0x00FD, 0x0006];

/// Il record BIFF all'offset `at`: tipo, dati e offset del seguente; `None` alla fine o se
/// il record esce dal flusso.
fn biff_record(stream: &[u8], at: usize) -> Option<(u16, &[u8], usize)> {
    let kind = u16_at(stream, at)?;
    let len = usize::from(u16_at(stream, at.checked_add(2)?)?);
    let start = at.checked_add(4)?;
    let end = start.checked_add(len)?;
    Some((kind, stream.get(start..end)?, end))
}

/// Le celle che `calamine` terrebbe in memoria per il flusso BIFF `stream` di un xls,
/// seguendo gli stessi passi: i fogli dalle posizioni dei record BOUNDSHEET del primo
/// blocco (fino al primo EOF), e per ogni foglio, fino al suo EOF e fuori dai blocchi
/// annidati (grafici), le dimensioni dichiarate (che riserva) e il rettangolo delle celle
/// (che costruisce). [`ExtractError::Encrypted`] se il blocco iniziale ha un FILEPASS.
fn xls_cells(stream: &[u8]) -> Result<u64, ExtractError> {
    let mut sheets: Vec<usize> = Vec::new();
    let mut at = 0;
    while let Some((kind, data, next)) = biff_record(stream, at) {
        match kind {
            BIFF_FILE_PASS if u16_at(data, 0) != Some(0) => return Err(ExtractError::Encrypted),
            BIFF_BOUND_SHEET => sheets.extend(u32_at(data, 0).map(to_usize)),
            BIFF_EOF => break,
            _ => {}
        }
        at = next;
    }
    let mut total = 0_u64;
    for start in sheets {
        let Some(sheet) = stream.get(start..) else {
            continue;
        };
        total = total.saturating_add(biff_sheet_cells(sheet));
    }
    Ok(total)
}

/// Le celle di un foglio BIFF che inizia all'inizio di `sheet` (vedi [`xls_cells`]).
fn biff_sheet_cells(sheet: &[u8]) -> u64 {
    let mut declared = 0_u64;
    let mut extent = Extent::default();
    let mut seen_bof = false;
    let mut nested = 0_usize;
    let mut at = 0;
    while let Some((kind, data, next)) = biff_record(sheet, at) {
        at = next;
        if nested > 0 {
            match kind {
                BIFF_BOF => nested += 1,
                BIFF_EOF => nested -= 1,
                _ => {}
            }
            continue;
        }
        match kind {
            BIFF_BOF if seen_bof => nested += 1,
            BIFF_BOF => seen_bof = true,
            BIFF_EOF => break,
            BIFF_DIMENSIONS => declared = declared.saturating_add(declared_cells(data)),
            BIFF_MUL_RK => {
                let first = u16_at(data, 2);
                let last = data.len().checked_sub(2).and_then(|at| u16_at(data, at));
                if let (Some(row), Some(first), Some(last)) = (u16_at(data, 0), first, last) {
                    let row = u64::from(row);
                    extent.add(
                        (row, row),
                        (u64::from(first), u64::from(last).max(u64::from(first))),
                    );
                }
            }
            kind if BIFF_CELLS.contains(&kind) => {
                if let (Some(row), Some(col)) = (u16_at(data, 0), u16_at(data, 2)) {
                    let (row, col) = (u64::from(row), u64::from(col));
                    extent.add((row, row), (col, col));
                }
            }
            _ => {}
        }
    }
    declared.saturating_add(extent.cells())
}

/// Le celle di un record DIMENSIONS come le calcola `calamine` (che ne riserva lo spazio):
/// righe e colonne a 32 bit con l'aritmetica circolare della sua build di rilascio, così un
/// intervallo rovesciato diventa enorme come lì. Lunghezze diverse da 10, 14 e 16 byte non
/// riservano nulla (`calamine` le rifiuta).
fn declared_cells(data: &[u8]) -> u64 {
    let fields = match data.len() {
        10 => (
            u16_at(data, 0).map(u32::from),
            u16_at(data, 2).map(u32::from),
            u16_at(data, 4).map(u32::from),
            u16_at(data, 6).map(u32::from),
        ),
        14 | 16 => (
            u32_at(data, 0),
            u32_at(data, 4),
            u16_at(data, 8).map(u32::from),
            u16_at(data, 10).map(u32::from),
        ),
        _ => return 0,
    };
    let (Some(first_row), Some(last_row), Some(first_col), Some(last_col)) = fields else {
        return 0;
    };
    let first_col = if first_col > 0xFF || last_col < first_col {
        0
    } else {
        first_col
    };
    let (end_row, end_col) = if last_row >= 1 && last_col >= 1 {
        (last_row - 1, last_col - 1)
    } else {
        (first_row, first_col)
    };
    let rows = u64::from(end_row.wrapping_sub(first_row).wrapping_add(1));
    let cols = u64::from(end_col.wrapping_sub(first_col).wrapping_add(1));
    rows.saturating_mul(cols)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;
    use crate::event::{ExtractEvent, WarningKind};
    use crate::extract::{ExtractOptions, extract_bytes};

    const ODS_HEAD: &str = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="Fogli">"#;
    const ODS_TAIL: &str =
        "</table:table></office:spreadsheet></office:body></office:document-content>";

    fn ods_content(rows: &str) -> String {
        format!("{ODS_HEAD}{rows}{ODS_TAIL}")
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

    #[test]
    fn ods_cells_partitions() {
        let measure = |rows: &str| ods_cells(ods_content(rows).as_bytes()).ok();
        // Celle di solo stile, anche ripetute all'infinito: nulla.
        let styled = r#"<table:table-row table:number-rows-repeated="1048576"><table:table-cell table:number-columns-repeated="16384"/></table:table-row>"#;
        assert_eq!(measure(styled), Some(0));
        // Una cella con valore ripetuta su 10 colonne e 3 righe: rettangolo 30, lette 10.
        let block = r#"<table:table-row table:number-rows-repeated="3"><table:table-cell office:value="1" table:number-columns-repeated="10"/></table:table-row>"#;
        assert_eq!(measure(block), Some(40));
        // Prefissi diversi, stesso nome locale: contano lo stesso.
        let other_prefix = r#"<table:table-row xmlns:x="urn:altro"><table:table-cell x:value-type="float" x:number-columns-repeated="5"/></table:table-row>"#;
        assert_eq!(measure(other_prefix), Some(10));
        // Spazi ripetuti: contano come celle.
        let spaces = r#"<table:table-row><table:table-cell office:value-type="string"><text:p><text:s text:c="3200000000"/></text:p></table:table-cell></table:table-row>"#;
        assert!(measure(spaces).is_some_and(|cells| cells > MAX_WORKBOOK_CELLS));
    }

    fn biff(kind: u16, data: &[u8]) -> Vec<u8> {
        let mut out = kind.to_le_bytes().to_vec();
        let len = u16::try_from(data.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    /// Un NUMBER (riga, colonna, formato 0, valore 1).
    fn number(row: u16, col: u16) -> Vec<u8> {
        let mut data = row.to_le_bytes().to_vec();
        data.extend_from_slice(&col.to_le_bytes());
        data.extend_from_slice(&0_u16.to_le_bytes());
        data.extend_from_slice(&1.0_f64.to_le_bytes());
        biff(0x0203, &data)
    }

    /// DIMENSIONS BIFF8: righe [first, last), colonne [first, last).
    fn dimensions(rows: (u32, u32), cols: (u16, u16)) -> Vec<u8> {
        let mut data = rows.0.to_le_bytes().to_vec();
        data.extend_from_slice(&rows.1.to_le_bytes());
        data.extend_from_slice(&cols.0.to_le_bytes());
        data.extend_from_slice(&cols.1.to_le_bytes());
        data.extend_from_slice(&0_u16.to_le_bytes());
        biff(BIFF_DIMENSIONS, &data)
    }

    /// Un flusso BIFF8 con un foglio fatto dei record `sheet`.
    fn workbook(sheet: &[Vec<u8>]) -> Vec<u8> {
        let bof = |kind: u16| {
            let mut data = 0x0600_u16.to_le_bytes().to_vec();
            data.extend_from_slice(&kind.to_le_bytes());
            data.extend_from_slice(&[0; 12]);
            biff(BIFF_BOF, &data)
        };
        let name = b"Foglio";
        // BOUNDSHEET: posizione, stato, tipo, lunghezza del nome, opzioni, nome.
        let bound_len = 4 + 4 + 4 + name.len();
        let globals_len = bof(5).len() + bound_len + 4;
        let mut bound = u32::try_from(globals_len)
            .unwrap_or(0)
            .to_le_bytes()
            .to_vec();
        bound.extend_from_slice(&[0, 0, 6, 0]);
        bound.extend_from_slice(name);
        let mut stream = bof(5);
        stream.extend(biff(BIFF_BOUND_SHEET, &bound));
        stream.extend(biff(BIFF_EOF, &[]));
        assert_eq!(stream.len(), globals_len);
        stream.extend(bof(0x10));
        for record in sheet {
            stream.extend_from_slice(record);
        }
        stream.extend(biff(BIFF_EOF, &[]));
        stream
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
    fn xls_cells_partitions() {
        // Rettangolo delle celle più dimensioni dichiarate.
        let small = workbook(&[dimensions((0, 2), (0, 3)), number(0, 0), number(1, 2)]);
        assert_eq!(xls_cells(&small).ok(), Some(6 + 6));
        // Le celle di un blocco annidato (un grafico) non contano.
        let nested = workbook(&[
            biff(BIFF_BOF, &[0; 16]),
            number(60_000, 200),
            biff(BIFF_EOF, &[]),
            number(0, 0),
        ]);
        assert_eq!(xls_cells(&nested).ok(), Some(1));
        // Cifrato: FILEPASS nel blocco iniziale.
        let mut encrypted = biff(BIFF_FILE_PASS, &[1, 0]);
        encrypted.extend(workbook(&[]));
        assert!(matches!(
            xls_cells(&encrypted),
            Err(ExtractError::Encrypted)
        ));
    }

    #[test]
    fn declared_cells_partitions() {
        let dims = |rows: (u32, u32), cols: (u16, u16)| {
            let record = dimensions(rows, cols);
            declared_cells(record.get(4..).unwrap_or_default())
        };
        assert_eq!(dims((0, 10), (0, 4)), 40);
        assert_eq!(dims((0, 0), (0, 0)), 1);
        // Intervallo rovesciato: con l'aritmetica circolare è enorme, come in `calamine`.
        assert!(dims((10, 5), (0, 4)) > MAX_WORKBOOK_CELLS);
        // Lunghezza non prevista: nulla.
        assert_eq!(declared_cells(&[0; 12]), 0);
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
