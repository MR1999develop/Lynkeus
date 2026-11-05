// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Stima della memoria che `calamine` userebbe per un foglio di calcolo ods o xls, prima di
//! passargli il file: oltre [`MAX_WORKBOOK_BYTES`] il file non si legge (avviso "memoria").
//!
//! `calamine` costruisce in memoria, per ogni foglio, il rettangolo tra la prima e l'ultima
//! cella con un valore, e un'allocazione impossibile termina il processo. La stima segue i
//! suoi passi con un lettore proprio (l'XML di `content.xml`, i record BIFF del flusso
//! `Workbook`) e conta i byte, non le celle:
//!
//! - ciò che resta per tutta la lettura ([`WorkbookMemory`]): il rettangolo di ogni foglio
//!   (un valore per cella, più il testo di ogni cella, che `calamine` clona per ogni
//!   ripetizione) e, per xls, le stringhe condivise (SST);
//! - ciò che serve a un foglio alla volta: le celle lette prima di costruire il rettangolo,
//!   o la copia del rettangolo che `calamine` restituisce quando si chiede il foglio. Conta
//!   solo il foglio più grande.
//!
//! Le stime sono prudenti (mai sotto il vero, salvo le crescite intermedie dei vettori):
//! ripetizioni al valore più grande dichiarato, ogni byte di testo al massimo che può
//! diventare in UTF-8. Le formule degli xls non si stimano: [`neutralize_xls_formulas`] le
//! svuota prima, così il testo che `calamine` ne ricava ha una lunghezza fissa.

use std::io::BufRead;

use crate::error::ExtractError;
use crate::extract::cfbfile::{to_usize, u16_at, u32_at};
use crate::extract::xml::{Element, XmlEvent, XmlReader};
use crate::limits::LimitKind;

/// Byte di celle che `calamine` può tenere in memoria per un file ods o xls. Il processo
/// estrattore ha 1 GiB: l'altra metà resta per il file, le sue copie e il resto. Ci stanno un
/// foglio xls denso di 65.536 righe (tutte quelle di BIFF8) per 40 colonne (circa 190 MB) e
/// un foglio ods di 100.000 righe per 40 colonne con valori brevi (circa 460 MB).
pub(crate) const MAX_WORKBOOK_BYTES: u64 = 512 * 1024 * 1024;

/// La memoria stimata di una cartella di lavoro in `calamine`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkbookMemory {
    /// Ciò che resta per tutta la lettura: rettangoli di tutti i fogli e dati comuni.
    kept: u64,
    /// La parte più grande tra quelle che servono a un foglio alla volta.
    peak_transient: u64,
}

impl WorkbookMemory {
    /// I byte stimati nel momento peggiore.
    pub fn bytes(&self) -> u64 {
        self.kept.saturating_add(self.peak_transient)
    }

    /// Errore "memoria" se la stima supera [`MAX_WORKBOOK_BYTES`].
    pub fn within_limit(&self) -> Result<(), ExtractError> {
        if self.bytes() > MAX_WORKBOOK_BYTES {
            return Err(ExtractError::LimitExceeded(LimitKind::Memory));
        }
        Ok(())
    }

    /// Aggiunge dati che restano per tutta la lettura.
    fn add_shared(&mut self, bytes: u64) {
        self.kept = self.kept.saturating_add(bytes);
    }

    /// Aggiunge un foglio: `kept` resta, `transient` serve solo mentre lo si legge.
    fn add_sheet(&mut self, kept: u64, transient: u64) {
        self.kept = self.kept.saturating_add(kept);
        self.peak_transient = self.peak_transient.max(transient);
    }
}

/// Byte di un valore di cella di `calamine` (`Data`).
const DATA_BYTES: u64 = 32;
/// Byte di una stringa vuota (`String`), come le formule delle celle senza formula.
const STRING_BYTES: u64 = 24;

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

// ── ods ───────────────────────────────────────────────────────────────────────────────────

/// Byte di una cella di ods in `calamine`: il valore (`Data`) e la formula (`String`), in
/// due rettangoli uguali.
const ODS_CELL_BYTES: u64 = DATA_BYTES + STRING_BYTES;

/// Righe al più in una tabella di ods: `calamine` tronca lì le ripetizioni.
const ODS_MAX_ROWS: u64 = 1_048_576;

/// Colonne al più in una riga di ods: `calamine` tronca lì le ripetizioni.
const ODS_MAX_COLUMNS: u64 = 16_384;

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

/// Gli attributi il cui testo `calamine` copia nella cella (valore o formula).
const ODS_TEXT_ATTRIBUTES: &[&str] = &["string-value", "date-value", "time-value", "formula"];

/// Una tabella di ods in lettura.
#[derive(Debug, Default)]
struct OdsTable {
    /// Rettangolo delle celle con un valore, ripetizioni comprese.
    extent: Extent,
    /// Testo delle celle del rettangolo, ripetuto quante volte le celle si ripetono.
    rect_text: u64,
    /// Celle lette riga per riga (prima di costruire il rettangolo), senza ripetere le righe.
    read_cells: u64,
    /// Testo delle celle lette riga per riga.
    read_text: u64,
}

impl OdsTable {
    /// Ciò che resta della tabella e la parte che serve solo mentre la si legge.
    fn memory(&self) -> (u64, u64) {
        let kept = self
            .extent
            .cells()
            .saturating_mul(ODS_CELL_BYTES)
            .saturating_add(self.rect_text);
        let read = self
            .read_cells
            .saturating_mul(ODS_CELL_BYTES)
            .saturating_add(self.read_text);
        (kept, read.max(kept))
    }
}

/// Una cella di ods aperta: posizione, ripetizioni e testo visto finora.
#[derive(Debug)]
struct OdsCell {
    col: u64,
    width: u64,
    has_value: bool,
    text: u64,
}

/// La riga di ods in lettura.
#[derive(Debug, Default)]
struct OdsRow {
    /// Prima riga (0-based) e ripetizioni.
    first: u64,
    repeat: u64,
    /// Prossima colonna.
    col: u64,
    /// Colonne lette fino all'ultima con un valore.
    width: u64,
    /// Testo delle celle lette.
    text: u64,
}

/// La memoria che `calamine` userebbe per il `content.xml` di un ods.
///
/// Prudente: elementi e attributi si riconoscono dal nome locale in qualunque namespace
/// (`calamine` li riconosce dal prefisso), le ripetizioni prendono il valore più grande e il
/// testo di una cella conta gli attributi di valore e formula, il contenuto (un byte per
/// elemento, `text:s` per i suoi spazi) e le annotazioni.
pub(crate) fn measure_ods<R: BufRead>(source: R) -> Result<WorkbookMemory, ExtractError> {
    let mut xml = XmlReader::new(source);
    let mut memory = WorkbookMemory::default();
    let mut tables: Vec<OdsTable> = Vec::new();
    let mut cells: Vec<OdsCell> = Vec::new();
    let mut row = OdsRow::default();
    let mut next_row = 0_u64;
    loop {
        let event = xml.next()?;
        let open = matches!(event, XmlEvent::Open(_));
        match event {
            XmlEvent::Open(e) | XmlEvent::Empty(e) => match e.local.as_str() {
                "table" if open => {
                    tables.push(OdsTable::default());
                    next_row = 0;
                }
                "table-row" if open => {
                    row = OdsRow {
                        first: next_row,
                        repeat: repeats(&e, "number-rows-repeated").min(ODS_MAX_ROWS),
                        ..OdsRow::default()
                    };
                }
                "table-row" => {
                    next_row = next_row.saturating_add(repeats(&e, "number-rows-repeated"));
                }
                "table-cell" | "covered-table-cell" => {
                    let cell = open_cell(&e, &mut row);
                    if open {
                        cells.push(cell);
                    } else {
                        close_cell(&cell, &mut row, tables.last_mut());
                    }
                }
                local => {
                    // Un elemento dentro una cella: un carattere (paragrafo, tabulazione,
                    // a capo), o gli spazi di `text:s`.
                    if let Some(cell) = cells.last_mut() {
                        let chars = if local == "s" { repeats(&e, "c") } else { 1 };
                        cell.text = cell.text.saturating_add(chars);
                    }
                }
            },
            XmlEvent::Text(text) => {
                if let Some(cell) = cells.last_mut() {
                    let len = u64::try_from(text.len()).unwrap_or(u64::MAX);
                    cell.text = cell.text.saturating_add(len);
                }
            }
            XmlEvent::Close(_, local) => match local.as_str() {
                "table" => {
                    if let Some(table) = tables.pop() {
                        let (kept, transient) = table.memory();
                        memory.add_sheet(kept, transient);
                    }
                }
                "table-row" => {
                    if let Some(table) = tables.last_mut() {
                        let width = row.width.min(ODS_MAX_COLUMNS);
                        table.read_cells = table.read_cells.saturating_add(width);
                        table.read_text = table.read_text.saturating_add(row.text);
                    }
                    next_row = row.first.saturating_add(row.repeat);
                }
                "table-cell" | "covered-table-cell" => {
                    if let Some(cell) = cells.pop() {
                        close_cell(&cell, &mut row, tables.last_mut());
                    }
                }
                _ => {}
            },
            XmlEvent::Eof => break,
        }
    }
    // Tabelle non chiuse (XML troncato): contano lo stesso.
    for table in tables {
        let (kept, transient) = table.memory();
        memory.add_sheet(kept, transient);
    }
    Ok(memory)
}

/// Una cella che si apre nella riga `row`: avanza la colonna della riga.
fn open_cell(e: &Element, row: &mut OdsRow) -> OdsCell {
    let width = repeats(e, "number-columns-repeated");
    let text = ODS_TEXT_ATTRIBUTES
        .iter()
        .flat_map(|local| e.attrs_named(local))
        .map(|value| u64::try_from(value.len()).unwrap_or(u64::MAX))
        .fold(0_u64, u64::saturating_add);
    let cell = OdsCell {
        col: row.col,
        width,
        has_value: may_have_value(e),
        text,
    };
    row.col = row.col.saturating_add(width);
    cell
}

/// Conta una cella chiusa nella riga e nella tabella, se ha un valore.
fn close_cell(cell: &OdsCell, row: &mut OdsRow, table: Option<&mut OdsTable>) {
    if !cell.has_value {
        return;
    }
    let last_col = cell.col.saturating_add(cell.width.saturating_sub(1));
    let last_row = row.first.saturating_add(row.repeat.saturating_sub(1));
    let width = cell.width.min(ODS_MAX_COLUMNS);
    let row_text = cell.text.saturating_mul(width);
    row.width = last_col.saturating_add(1);
    row.text = row.text.saturating_add(row_text);
    if let Some(table) = table {
        table
            .extent
            .add((row.first, last_row), (cell.col, last_col));
        table.rect_text = table
            .rect_text
            .saturating_add(row_text.saturating_mul(row.repeat));
    }
}

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

// ── xls ───────────────────────────────────────────────────────────────────────────────────

/// Tipi dei record BIFF che contano.
const BIFF_EOF: u16 = 0x000A;
const BIFF_BOF: u16 = 0x0809;
const BIFF_BOUND_SHEET: u16 = 0x0085;
const BIFF_FILE_PASS: u16 = 0x002F;
const BIFF_DIMENSIONS: u16 = 0x0200;
const BIFF_CONTINUE: u16 = 0x003C;
const BIFF_SST: u16 = 0x00FC;
const BIFF_NUMBER: u16 = 0x0203;
const BIFF_LABEL: u16 = 0x0204;
const BIFF_RSTRING: u16 = 0x00D6;
const BIFF_BOOL_ERR: u16 = 0x0205;
const BIFF_RK: u16 = 0x027E;
const BIFF_LABEL_SST: u16 = 0x00FD;
const BIFF_MUL_RK: u16 = 0x00BD;
const BIFF_FORMULA: u16 = 0x0006;
const BIFF_STRING: u16 = 0x0207;

/// Byte di una cella letta da `calamine` per un foglio xls, prima del rettangolo
/// (`Cell<Data>`: posizione e valore).
const XLS_CELL_RECORD_BYTES: u64 = 40;
/// Byte di una formula letta (`Cell<String>`).
const XLS_FORMULA_RECORD_BYTES: u64 = 32;
/// Byte UTF-8 al più per un byte di testo BIFF: un byte a 8 bit diventa un'unità UTF-16 che
/// una codifica a 8 bit legge come due caratteri.
const XLS_UTF8_PER_BYTE: u64 = 4;
/// Byte UTF-8 al più per un carattere (unità) di una stringa condivisa.
const XLS_UTF8_PER_CHAR: u64 = 6;
/// Byte del testo che `calamine` scrive al posto di una formula svuotata (il messaggio
/// "formula non riconosciuta" con la cella, con il margine della sua allocazione).
const XLS_FORMULA_TEXT_BYTES: u64 = 160;
/// Offset della lunghezza dei token (`cce`) nei dati di un record FORMULA.
const FORMULA_CCE_AT: usize = 20;

/// Un record BIFF con i dati dei record CONTINUE che lo seguono (come li unisce `calamine`).
struct BiffRecord<'a> {
    kind: u16,
    data: &'a [u8],
    /// I record CONTINUE che seguono, con le loro intestazioni.
    continues: &'a [u8],
}

/// I record BIFF di un flusso, come li legge `calamine`: ogni record si prende con i record
/// CONTINUE che lo seguono. Finisce alla fine del flusso o al primo record che esce dal
/// flusso (dove `calamine` dà errore).
struct BiffRecords<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for BiffRecords<'a> {
    type Item = BiffRecord<'a>;

    fn next(&mut self) -> Option<BiffRecord<'a>> {
        let Some((kind, data, after)) = split_record(self.rest) else {
            self.rest = &[];
            return None;
        };
        let mut tail = after;
        // `calamine` unisce i CONTINUE solo se dopo restano più di 4 byte.
        while tail.len() > 4 && u16_at(tail, 0) == Some(BIFF_CONTINUE) {
            let Some((_, _, next)) = split_record(tail) else {
                self.rest = &[];
                return None;
            };
            tail = next;
        }
        let continues = after.get(..after.len() - tail.len()).unwrap_or_default();
        self.rest = tail;
        Some(BiffRecord {
            kind,
            data,
            continues,
        })
    }
}

/// Tipo, dati e resto del flusso del record in testa a `stream`; `None` se il record esce
/// dal flusso.
fn split_record(stream: &[u8]) -> Option<(u16, &[u8], &[u8])> {
    let kind = u16_at(stream, 0)?;
    let len = usize::from(u16_at(stream, 2)?);
    let end = len.checked_add(4)?;
    let data = stream.get(4..end)?;
    Some((kind, data, stream.get(end..)?))
}

/// Svuota le formule del flusso BIFF `stream` (lunghezza dei token a zero, record della
/// stessa lunghezza) nei fogli, come li percorre `calamine`: il testo che `calamine` ne
/// ricava diventa un messaggio di lunghezza fissa, e il suo lettore di formule non tocca
/// dati non fidati. I valori delle celle con formula restano. Il programma non usa le
/// formule.
pub(crate) fn neutralize_xls_formulas(stream: &mut [u8]) {
    for start in sheet_offsets(stream) {
        let mut patches = Vec::new();
        if let Some(sheet) = stream.get(start..) {
            visit_sheet(sheet, |offset, record| {
                if record.kind == BIFF_FORMULA && record.data.len() >= FORMULA_CCE_AT + 2 {
                    patches.push(start.saturating_add(offset));
                }
            });
        }
        for data_at in patches {
            let at = data_at.saturating_add(FORMULA_CCE_AT);
            if let Some(cce) = stream.get_mut(at..at.saturating_add(2)) {
                cce.fill(0);
            }
        }
    }
}

/// Gli offset dei fogli dai record BOUNDSHEET del blocco iniziale (fino al primo EOF).
fn sheet_offsets(stream: &[u8]) -> Vec<usize> {
    let mut sheets = Vec::new();
    for record in (BiffRecords { rest: stream }) {
        match record.kind {
            BIFF_BOUND_SHEET => sheets.extend(u32_at(record.data, 0).map(to_usize)),
            BIFF_EOF => break,
            _ => {}
        }
    }
    sheets
}

/// Chiama `visit` con l'offset dei dati (da `sheet`) e il record, per ogni record del foglio
/// che inizia all'inizio di `sheet`, come `calamine`: fino al suo EOF e fuori dai blocchi
/// annidati (grafici).
fn visit_sheet<'a>(sheet: &'a [u8], mut visit: impl FnMut(usize, &BiffRecord<'a>)) {
    let mut seen_bof = false;
    let mut nested = 0_usize;
    let mut records = BiffRecords { rest: sheet };
    loop {
        let data_at = sheet.len() - records.rest.len() + 4;
        let Some(record) = records.next() else {
            return;
        };
        if nested > 0 {
            match record.kind {
                BIFF_BOF => nested += 1,
                BIFF_EOF => nested -= 1,
                _ => {}
            }
            continue;
        }
        match record.kind {
            BIFF_BOF if seen_bof => nested += 1,
            BIFF_BOF => seen_bof = true,
            BIFF_EOF => return,
            _ => visit(data_at, &record),
        }
    }
}

/// La memoria che `calamine` userebbe per il flusso BIFF `stream` di un xls (con le formule
/// già svuotate da [`neutralize_xls_formulas`]): le stringhe condivise e, per ogni foglio
/// dei record BOUNDSHEET, il rettangolo delle celle (con il loro testo) e delle formule, più
/// le celle lette prima del rettangolo (almeno quante ne riservano le dimensioni
/// dichiarate). [`ExtractError::Encrypted`] se il blocco iniziale ha un FILEPASS.
///
/// Una formula non svuotata (raggiunta per una via che lo svuotamento non ha percorso: record
/// sovrapposti) ha un testo che non si stima: errore "memoria".
pub(crate) fn measure_xls(stream: &[u8]) -> Result<WorkbookMemory, ExtractError> {
    let mut sheets: Vec<usize> = Vec::new();
    let mut strings: Vec<u64> = Vec::new();
    for record in (BiffRecords { rest: stream }) {
        match record.kind {
            BIFF_FILE_PASS if u16_at(record.data, 0) != Some(0) => {
                return Err(ExtractError::Encrypted);
            }
            BIFF_BOUND_SHEET => sheets.extend(u32_at(record.data, 0).map(to_usize)),
            BIFF_SST => strings = sst_string_bytes(&record),
            BIFF_EOF => break,
            _ => {}
        }
    }
    let mut memory = WorkbookMemory::default();
    let shared = strings
        .iter()
        .map(|bytes| bytes.saturating_add(STRING_BYTES))
        .fold(0_u64, u64::saturating_add);
    memory.add_shared(shared);
    for start in sheets {
        let Some(sheet) = stream.get(start..) else {
            continue;
        };
        let (kept, transient) = XlsSheet::measure(sheet, &strings)?;
        memory.add_sheet(kept, transient);
    }
    Ok(memory)
}

/// Un foglio xls in lettura.
#[derive(Debug, Default)]
struct XlsSheet {
    /// Rettangolo delle celle.
    cells: Extent,
    /// Rettangolo delle formule.
    formulas: Extent,
    /// Testo delle celle (stringhe, anche quelle condivise clonate per ogni cella).
    text: u64,
    /// Celle lette (record di cella, ognuna delle celle di un MULRK).
    read: u64,
    /// Celle riservate dalle dimensioni dichiarate.
    declared: u64,
    /// Formule lette.
    formula_count: u64,
    /// Posizione dell'ultima formula: lì `calamine` mette il valore del record STRING.
    formula_at: (u64, u64),
}

impl XlsSheet {
    /// Ciò che resta del foglio che inizia all'inizio di `sheet` e la parte che serve solo
    /// mentre lo si legge; `strings` sono i byte delle stringhe condivise.
    fn measure(sheet: &[u8], strings: &[u64]) -> Result<(u64, u64), ExtractError> {
        let mut state = XlsSheet::default();
        let mut unbounded = false;
        visit_sheet(sheet, |_, record| {
            unbounded |= !state.record(record, strings);
        });
        if unbounded {
            return Err(ExtractError::LimitExceeded(LimitKind::Memory));
        }
        Ok(state.memory())
    }

    /// Conta un record; falso se è una formula non svuotata.
    fn record(&mut self, record: &BiffRecord<'_>, strings: &[u64]) -> bool {
        let data = record.data;
        match record.kind {
            BIFF_DIMENSIONS => {
                self.declared = self.declared.saturating_add(declared_cells(data));
                return true;
            }
            // Il valore testuale di una formula, senza posizione propria.
            BIFF_STRING => {
                let (row, col) = self.formula_at;
                self.cell(row, col, decoded_bytes(data));
                return true;
            }
            _ => {}
        }
        let (Some(row), Some(col)) = (u16_at(data, 0), u16_at(data, 2)) else {
            return true;
        };
        let (row, col) = (u64::from(row), u64::from(col));
        match record.kind {
            BIFF_NUMBER | BIFF_BOOL_ERR | BIFF_RK => self.cell(row, col, 0),
            BIFF_LABEL | BIFF_RSTRING => {
                let text = data.get(6..).map_or(0, decoded_bytes);
                self.cell(row, col, text);
            }
            BIFF_LABEL_SST => {
                let index = u32_at(data, 6).map(to_usize);
                let text = index.and_then(|i| strings.get(i)).copied().unwrap_or(0);
                self.cell(row, col, text);
            }
            BIFF_MUL_RK => self.mul_rk(data, row, col),
            BIFF_FORMULA => {
                self.formula_at = (row, col);
                self.cell(row, col, 0);
                self.formulas.add((row, row), (col, col));
                self.formula_count = self.formula_count.saturating_add(1);
                if u16_at(data, FORMULA_CCE_AT).is_some_and(|cce| cce != 0) {
                    return false;
                }
            }
            _ => {}
        }
        true
    }

    /// Una cella letta in (`row`, `col`) con `text` byte di testo.
    fn cell(&mut self, row: u64, col: u64, text: u64) {
        self.cells.add((row, row), (col, col));
        self.read = self.read.saturating_add(1);
        self.text = self.text.saturating_add(text);
    }

    /// Le celle di un MULRK nella riga `row` dalla colonna `first`: fino all'ultima colonna
    /// scritta in fondo al record, una per ogni gruppo di 6 byte.
    fn mul_rk(&mut self, data: &[u8], row: u64, first: u64) {
        let last = data
            .len()
            .checked_sub(2)
            .and_then(|at| u16_at(data, at))
            .map_or(first, |last| u64::from(last).max(first));
        self.cells.add((row, row), (first, last));
        let count = u64::try_from(data.len().saturating_sub(6) / 6).unwrap_or(u64::MAX);
        self.read = self.read.saturating_add(count);
    }

    /// Ciò che resta e la parte transitoria (vedi [`XlsSheet::measure`]).
    fn memory(&self) -> (u64, u64) {
        let kept = self
            .cells
            .cells()
            .saturating_mul(DATA_BYTES)
            .saturating_add(self.text)
            .saturating_add(self.formulas.cells().saturating_mul(STRING_BYTES))
            .saturating_add(self.formula_count.saturating_mul(XLS_FORMULA_TEXT_BYTES));
        let read = self
            .read
            .max(self.declared)
            .saturating_mul(XLS_CELL_RECORD_BYTES)
            .saturating_add(self.formula_count.saturating_mul(XLS_FORMULA_RECORD_BYTES));
        (kept, read.max(kept))
    }
}

/// I byte di una stringa BIFF non condivisa (LABEL, STRING): `calamine` ne riserva tanti
/// caratteri quanti ne dichiara (`cch`, in testa) e ne decodifica al più i byte presenti.
fn decoded_bytes(data: &[u8]) -> u64 {
    let declared = u16_at(data, 0).map_or(0, u64::from);
    let present = u64::try_from(data.len()).unwrap_or(u64::MAX);
    declared.max(present.saturating_mul(XLS_UTF8_PER_BYTE))
}

/// Il record SST letto come lo legge `calamine`, per pezzi (i dati, poi un record CONTINUE
/// dopo l'altro).
struct SstReader<'a> {
    data: &'a [u8],
    continues: &'a [u8],
}

impl SstReader<'_> {
    /// Passa al prossimo pezzo; falso se non ce ne sono.
    fn next_piece(&mut self) -> bool {
        match split_record(self.continues) {
            Some((_, data, rest)) => {
                self.data = data;
                self.continues = rest;
                true
            }
            None => false,
        }
    }

    /// Salta `len` byte attraverso i pezzi; falso se finiscono prima.
    fn skip(&mut self, mut len: usize) -> bool {
        while len > 0 {
            if self.data.is_empty() && !self.next_piece() {
                return false;
            }
            let step = len.min(self.data.len());
            self.data = self.data.get(step..).unwrap_or_default();
            len -= step;
        }
        true
    }

    /// Salta i caratteri di una stringa di `chars` unità, a 8 o 16 bit; a ogni nuovo pezzo un
    /// byte dice di nuovo la larghezza. Falso se i pezzi finiscono prima.
    fn skip_chars(&mut self, mut chars: usize, mut wide: bool) -> bool {
        while chars > 0 {
            let unit = if wide { 2 } else { 1 };
            let step = chars.min(self.data.len() / unit);
            self.data = self.data.get(step * unit..).unwrap_or_default();
            chars -= step;
            if chars > 0 {
                if !self.next_piece() {
                    return false;
                }
                let Some((&flags, rest)) = self.data.split_first() else {
                    return false;
                };
                wide = flags & 1 != 0;
                self.data = rest;
            }
        }
        true
    }

    /// Il numero di caratteri della prossima stringa, saltandola tutta; `None` dove
    /// `calamine` darebbe errore.
    fn next_string(&mut self) -> Option<usize> {
        if self.data.is_empty() {
            // `calamine` dà una stringa vuota.
            return Some(0);
        }
        let chars = usize::from(u16_at(self.data, 0)?);
        let flags = *self.data.get(2)?;
        self.data = self.data.get(3..)?;
        let mut runs = 0;
        if flags & 0x8 != 0 {
            runs = usize::from(u16_at(self.data, 0)?);
            self.data = self.data.get(2..)?;
        }
        let mut extra = 0;
        if flags & 0x4 != 0 {
            let raw = u32_at(self.data, 0)?;
            // `calamine` legge un i32: un valore negativo è un salto impossibile.
            extra = usize::try_from(i32::from_le_bytes(raw.to_le_bytes())).ok()?;
            self.data = self.data.get(4..)?;
        }
        (self.skip_chars(chars, flags & 1 != 0) && self.skip(runs * 4) && self.skip(extra))
            .then_some(chars)
    }
}

/// I byte stimati di ogni stringa del record SST, nell'ordine (sono gli indici dei record
/// LABELSST). Ci si ferma dove `calamine` darebbe errore: allora la cartella di lavoro non
/// si legge e le stringhe non contano.
fn sst_string_bytes(record: &BiffRecord<'_>) -> Vec<u64> {
    let mut reader = SstReader {
        data: record.data.get(8..).unwrap_or_default(),
        continues: record.continues,
    };
    let mut strings = Vec::new();
    while !reader.data.is_empty() || reader.next_piece() {
        let Some(chars) = reader.next_string() else {
            break;
        };
        let chars = u64::try_from(chars).unwrap_or(u64::MAX);
        strings.push(chars.saturating_mul(XLS_UTF8_PER_CHAR));
    }
    strings
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
pub(crate) mod tests {
    use proptest::prelude::*;

    use super::*;

    const ODS_HEAD: &str = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="Fogli">"#;
    const ODS_TAIL: &str =
        "</table:table></office:spreadsheet></office:body></office:document-content>";

    /// Il `content.xml` di un ods con `rows` come righe della sua unica tabella.
    pub(crate) fn ods_content(rows: &str) -> String {
        format!("{ODS_HEAD}{rows}{ODS_TAIL}")
    }

    /// I byte stimati per le righe `rows` di un ods.
    fn ods_bytes(rows: &str) -> Option<u64> {
        measure_ods(ods_content(rows).as_bytes())
            .ok()
            .map(|m| m.bytes())
    }

    /// Vero se il `content.xml` di un ods sta nel limite di memoria.
    fn ods_fits(content: &str) -> bool {
        measure_ods(content.as_bytes()).is_ok_and(|m| m.within_limit().is_ok())
    }

    /// Vero se il flusso BIFF di un xls, con le formule svuotate, sta nel limite di memoria.
    fn xls_fits(stream: &[u8]) -> bool {
        let mut stream = stream.to_vec();
        neutralize_xls_formulas(&mut stream);
        measure_xls(&stream).is_ok_and(|m| m.within_limit().is_ok())
    }

    pub(crate) fn biff(kind: u16, data: &[u8]) -> Vec<u8> {
        let mut out = kind.to_le_bytes().to_vec();
        let len = u16::try_from(data.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    /// Un NUMBER (riga, colonna, formato 0, valore 1).
    pub(crate) fn number(row: u16, col: u16) -> Vec<u8> {
        let mut data = row.to_le_bytes().to_vec();
        data.extend_from_slice(&col.to_le_bytes());
        data.extend_from_slice(&0_u16.to_le_bytes());
        data.extend_from_slice(&1.0_f64.to_le_bytes());
        biff(BIFF_NUMBER, &data)
    }

    /// DIMENSIONS BIFF8: righe [first, last), colonne [first, last).
    pub(crate) fn dimensions(rows: (u32, u32), cols: (u16, u16)) -> Vec<u8> {
        let mut data = rows.0.to_le_bytes().to_vec();
        data.extend_from_slice(&rows.1.to_le_bytes());
        data.extend_from_slice(&cols.0.to_le_bytes());
        data.extend_from_slice(&cols.1.to_le_bytes());
        data.extend_from_slice(&0_u16.to_le_bytes());
        biff(BIFF_DIMENSIONS, &data)
    }

    /// Un flusso BIFF8 con un foglio fatto dei record `sheet`.
    pub(crate) fn workbook(sheet: &[Vec<u8>]) -> Vec<u8> {
        workbook_with(&[], sheet)
    }

    /// Un flusso BIFF8 con i record `globals` nel blocco iniziale (dopo il BOUNDSHEET) e un
    /// foglio fatto dei record `sheet`.
    fn workbook_with(globals: &[Vec<u8>], sheet: &[Vec<u8>]) -> Vec<u8> {
        let bof = |kind: u16| {
            let mut data = 0x0600_u16.to_le_bytes().to_vec();
            data.extend_from_slice(&kind.to_le_bytes());
            data.extend_from_slice(&[0; 12]);
            biff(BIFF_BOF, &data)
        };
        let name = b"Foglio";
        // BOUNDSHEET: posizione, stato, tipo, lunghezza del nome, opzioni, nome.
        let bound_len = 4 + 4 + 4 + name.len();
        let extra: usize = globals.iter().map(Vec::len).sum();
        let globals_len = bof(5).len() + bound_len + extra + 4;
        let mut bound = u32::try_from(globals_len)
            .unwrap_or(0)
            .to_le_bytes()
            .to_vec();
        bound.extend_from_slice(&[0, 0, 6, 0]);
        bound.extend_from_slice(name);
        let mut stream = bof(5);
        stream.extend(biff(BIFF_BOUND_SHEET, &bound));
        for record in globals {
            stream.extend_from_slice(record);
        }
        stream.extend(biff(BIFF_EOF, &[]));
        assert_eq!(stream.len(), globals_len);
        stream.extend(bof(0x10));
        for record in sheet {
            stream.extend_from_slice(record);
        }
        stream.extend(biff(BIFF_EOF, &[]));
        stream
    }

    /// SST con le stringhe `strings` (8 bit, senza formattazione).
    fn sst(strings: &[&str]) -> Vec<u8> {
        let count = u32::try_from(strings.len()).unwrap_or(u32::MAX);
        let mut data = count.to_le_bytes().to_vec();
        data.extend_from_slice(&count.to_le_bytes());
        for s in strings {
            let cch = u16::try_from(s.len()).unwrap_or(u16::MAX);
            data.extend_from_slice(&cch.to_le_bytes());
            data.push(0);
            data.extend_from_slice(s.as_bytes());
        }
        biff(BIFF_SST, &data)
    }

    /// LABELSST: la stringa `index` della SST in (`row`, `col`).
    fn label_sst(row: u16, col: u16, index: u32) -> Vec<u8> {
        let mut data = row.to_le_bytes().to_vec();
        data.extend_from_slice(&col.to_le_bytes());
        data.extend_from_slice(&0_u16.to_le_bytes());
        data.extend_from_slice(&index.to_le_bytes());
        biff(BIFF_LABEL_SST, &data)
    }

    /// STRING: il valore testuale della formula precedente (8 bit: solo ASCII).
    pub(crate) fn string_value(text: &str) -> Vec<u8> {
        let cch = u16::try_from(text.len()).unwrap_or(u16::MAX);
        let mut data = cch.to_le_bytes().to_vec();
        data.push(0);
        data.extend_from_slice(text.as_bytes());
        biff(BIFF_STRING, &data)
    }

    /// FORMULA in (`row`, `col`) con valore testuale (nel record STRING che segue) e una
    /// formula di un solo token.
    pub(crate) fn string_formula(row: u16, col: u16) -> Vec<u8> {
        let mut data = row.to_le_bytes().to_vec();
        data.extend_from_slice(&col.to_le_bytes());
        data.extend_from_slice(&0_u16.to_le_bytes());
        data.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0xFF, 0xFF]);
        data.extend_from_slice(&[0; 6]);
        // cce = 3: PtgInt 1.
        data.extend_from_slice(&3_u16.to_le_bytes());
        data.extend_from_slice(&[0x1E, 1, 0]);
        biff(BIFF_FORMULA, &data)
    }

    /// MULRK: `count` numeri nella riga `row` dalla colonna 0.
    pub(crate) fn mul_rk(row: u16, count: u16) -> Vec<u8> {
        let mut data = row.to_le_bytes().to_vec();
        data.extend_from_slice(&0_u16.to_le_bytes());
        for _ in 0..count {
            data.extend_from_slice(&0_u16.to_le_bytes());
            // RK: intero 1 (bit 1 acceso, valore spostato di due bit).
            data.extend_from_slice(&((1_u32 << 2) | 2).to_le_bytes());
        }
        data.extend_from_slice(&count.saturating_sub(1).to_le_bytes());
        biff(BIFF_MUL_RK, &data)
    }

    /// Le dimensioni dei tipi di `calamine` stanno nelle costanti della stima.
    #[test]
    fn calamine_sizes_fit_the_estimate() {
        let size = |bytes: usize| u64::try_from(bytes).unwrap_or(u64::MAX);
        assert!(size(size_of::<calamine::Data>()) <= DATA_BYTES);
        assert!(size(size_of::<String>()) <= STRING_BYTES);
        assert!(size(size_of::<calamine::Cell<calamine::Data>>()) <= XLS_CELL_RECORD_BYTES);
        assert!(size(size_of::<calamine::Cell<String>>()) <= XLS_FORMULA_RECORD_BYTES);
    }

    /// I rettangoli dei fogli si sommano, la parte transitoria conta solo per il più grande.
    #[test]
    fn sheets_add_their_rectangles_and_the_largest_transient() {
        let mut memory = WorkbookMemory::default();
        memory.add_sheet(100, 500);
        memory.add_sheet(200, 300);
        memory.add_shared(7);
        assert_eq!(memory.bytes(), 100 + 200 + 7 + 500);
        let limit = WorkbookMemory {
            kept: MAX_WORKBOOK_BYTES,
            peak_transient: 0,
        };
        assert!(limit.within_limit().is_ok());
        let over = WorkbookMemory {
            kept: MAX_WORKBOOK_BYTES,
            peak_transient: 1,
        };
        assert!(matches!(
            over.within_limit(),
            Err(ExtractError::LimitExceeded(LimitKind::Memory))
        ));
    }

    #[test]
    fn ods_partitions() {
        // Celle di solo stile, anche ripetute all'infinito: nulla.
        let styled = r#"<table:table-row table:number-rows-repeated="1048576"><table:table-cell table:number-columns-repeated="16384"/></table:table-row>"#;
        assert_eq!(ods_bytes(styled), Some(0));
        // Un numero ripetuto su 10 colonne e 3 righe: rettangolo 30, lette 10 (meno del
        // rettangolo: la parte transitoria è la copia del rettangolo).
        let block = r#"<table:table-row table:number-rows-repeated="3"><table:table-cell office:value="1" table:number-columns-repeated="10"/></table:table-row>"#;
        assert_eq!(ods_bytes(block), Some(2 * 30 * ODS_CELL_BYTES));
        // Prefissi diversi, stesso nome locale: contano lo stesso.
        let other_prefix = r#"<table:table-row xmlns:x="urn:altro"><table:table-cell x:value-type="float" x:number-columns-repeated="5"/></table:table-row>"#;
        assert_eq!(ods_bytes(other_prefix), Some(2 * 5 * ODS_CELL_BYTES));
        // Testo della cella: un byte per il paragrafo, tre di testo.
        let text = r#"<table:table-row><table:table-cell office:value-type="string"><text:p>abc</text:p></table:table-cell></table:table-row>"#;
        assert_eq!(ods_bytes(text), Some(2 * (ODS_CELL_BYTES + 4)));
        // Valore in un attributo.
        let attr = r#"<table:table-row><table:table-cell office:value-type="string" office:string-value="abcd"/></table:table-row>"#;
        assert_eq!(ods_bytes(attr), Some(2 * (ODS_CELL_BYTES + 4)));
        // Spazi ripetuti: contano come testo della cella.
        let spaces = r#"<table:table-row><table:table-cell office:value-type="string"><text:p><text:s text:c="3200000000"/></text:p></table:table-cell></table:table-row>"#;
        assert!(ods_bytes(spaces).is_some_and(|bytes| bytes > MAX_WORKBOOK_BYTES));
    }

    /// Righe distinte: le celle lette sono quante il rettangolo, e la parte transitoria
    /// conta una volta sola.
    #[test]
    fn ods_distinct_rows_count_reading_once() {
        let row = r#"<table:table-row><table:table-cell office:value="1" table:number-columns-repeated="4"/></table:table-row>"#;
        assert_eq!(ods_bytes(&row.repeat(3)), Some(2 * 12 * ODS_CELL_BYTES));
    }

    /// Un testo di 300 byte in 255 righe per 16.384 colonne ripetute: poche celle nel file,
    /// ma `calamine` clona il testo per ognuna (oltre 1 GB). Avviso "memoria".
    #[test]
    fn ods_repeated_long_text_is_over_the_limit() {
        let text = "a".repeat(300);
        let rows = format!(
            r#"<table:table-row table:number-rows-repeated="255"><table:table-cell office:value-type="string" table:number-columns-repeated="16384"><text:p>{text}</text:p></table:table-cell></table:table-row>"#
        );
        assert!(!ods_fits(&ods_content(&rows)));
        // Lo stesso testo in poche celle sta nel limite.
        let few = format!(
            r#"<table:table-row table:number-rows-repeated="10"><table:table-cell office:value-type="string" table:number-columns-repeated="10"><text:p>{text}</text:p></table:table-cell></table:table-row>"#
        );
        assert!(ods_fits(&ods_content(&few)));
    }

    /// Una formula lunga ripetuta conta come un testo lungo.
    #[test]
    fn ods_repeated_long_formula_is_over_the_limit() {
        let formula = format!("of:={}", "1+".repeat(150));
        let rows = format!(
            r#"<table:table-row table:number-rows-repeated="255"><table:table-cell table:formula="{formula}1" office:value-type="float" office:value="1" table:number-columns-repeated="16384"/></table:table-row>"#
        );
        assert!(!ods_fits(&ods_content(&rows)));
    }

    /// Un foglio ods denso di 60.000 righe per 40 colonne con valori brevi sta nel limite.
    #[test]
    fn dense_ods_sheet_fits() {
        let row = r#"<table:table-row><table:table-cell office:value-type="float" office:value="1" table:number-columns-repeated="40"/></table:table-row>"#;
        assert!(ods_fits(&ods_content(&row.repeat(60_000))));
    }

    #[test]
    fn xls_partitions() {
        let bytes = |stream: &[u8]| measure_xls(stream).ok().map(|m| m.bytes());
        // Rettangolo di 6 celle (2 righe per 3 colonne) e 6 celle lette, quante ne
        // riservano le dimensioni dichiarate.
        let small = workbook(&[dimensions((0, 2), (0, 3)), number(0, 0), number(1, 2)]);
        assert_eq!(
            bytes(&small),
            Some(6 * DATA_BYTES + 6 * XLS_CELL_RECORD_BYTES)
        );
        // Le celle di un blocco annidato (un grafico) non contano.
        let nested = workbook(&[
            biff(BIFF_BOF, &[0; 16]),
            number(60_000, 200),
            biff(BIFF_EOF, &[]),
            number(0, 0),
        ]);
        assert_eq!(bytes(&nested), Some(DATA_BYTES + XLS_CELL_RECORD_BYTES));
        // Cifrato: FILEPASS nel blocco iniziale.
        let mut encrypted = biff(BIFF_FILE_PASS, &[1, 0]);
        encrypted.extend(workbook(&[]));
        assert!(matches!(
            measure_xls(&encrypted),
            Err(ExtractError::Encrypted)
        ));
    }

    /// Una stringa lunga della SST in 100.000 celle: `calamine` la clona per ognuna.
    #[test]
    fn xls_shared_long_string_is_over_the_limit() {
        let long = "a".repeat(8000);
        let mut cells = Vec::new();
        for row in 0..50_000_u16 {
            cells.push(label_sst(row, 0, 0));
            cells.push(label_sst(row, 1, 0));
        }
        assert!(!xls_fits(&workbook_with(&[sst(&[&long])], &cells)));
        // Una stringa corta nelle stesse celle sta nel limite.
        assert!(xls_fits(&workbook_with(&[sst(&["città"])], &cells)));
    }

    /// Un record STRING senza FORMULA prima va in (0, 0), come in `calamine`: con un numero
    /// in (2999, 2999) il rettangolo ha 9 milioni di celle.
    #[test]
    fn xls_string_record_without_formula_counts_at_the_origin() {
        let far = workbook(&[string_value("città"), number(2999, 2999)]);
        assert!(!xls_fits(&far));
        // Dopo una FORMULA il valore va nella sua cella.
        let near = workbook(&[
            string_formula(2998, 2998),
            string_value("città"),
            number(2999, 2999),
        ]);
        assert!(xls_fits(&near));
    }

    /// Un foglio xls denso di 65.536 righe per 40 colonne (tutte le righe di BIFF8), con le
    /// dimensioni dichiarate: sta nel limite.
    #[test]
    fn dense_xls_sheet_fits() {
        let mut records = vec![dimensions((0, 65_536), (0, 40))];
        records.extend((0..=u16::MAX).map(|row| mul_rk(row, 40)));
        assert!(xls_fits(&workbook(&records)));
    }

    /// Le formule si svuotano (solo la lunghezza dei token, nei fogli); una formula non
    /// svuotata è un testo che non si stima.
    #[test]
    fn formulas_are_neutralized_before_measuring() {
        let stream = workbook(&[string_formula(1, 1), string_value("città")]);
        assert!(matches!(
            measure_xls(&stream),
            Err(ExtractError::LimitExceeded(LimitKind::Memory))
        ));
        let mut neutral = stream.clone();
        neutralize_xls_formulas(&mut neutral);
        assert_eq!(neutral.len(), stream.len());
        let changed = stream
            .iter()
            .zip(&neutral)
            .filter(|(before, after)| before != after)
            .count();
        assert_eq!(changed, 1, "solo il byte basso di cce (3) cambia");
        assert!(measure_xls(&neutral).is_ok());
    }

    /// SST con una stringa divisa da un CONTINUE (che riparte a 16 bit), formattazione e
    /// dati estesi: lunghezze e indici come in `calamine`.
    #[test]
    fn sst_strings_across_continue_records() {
        let mut data = 3_u32.to_le_bytes().to_vec();
        data.extend_from_slice(&3_u32.to_le_bytes());
        // 1: 5 caratteri a 8 bit, solo 2 qui.
        data.extend_from_slice(&5_u16.to_le_bytes());
        data.push(0);
        data.extend_from_slice(b"ab");
        let mut stream = biff(BIFF_SST, &data);
        // CONTINUE: riparte a 16 bit con i 3 caratteri che mancano, poi la stringa 2 con
        // formattazione (1 run) e dati estesi (3 byte), poi la 3.
        let mut more = vec![1];
        more.extend_from_slice(&[b'c', 0, b'd', 0, b'e', 0]);
        more.extend_from_slice(&2_u16.to_le_bytes());
        more.push(0x8 | 0x4);
        more.extend_from_slice(&1_u16.to_le_bytes());
        more.extend_from_slice(&3_i32.to_le_bytes());
        more.extend_from_slice(b"xy");
        more.extend_from_slice(&[0; 4 + 3]);
        more.extend_from_slice(&1_u16.to_le_bytes());
        more.push(0);
        more.push(b'z');
        stream.extend(biff(BIFF_CONTINUE, &more));
        stream.extend(number(0, 0));
        let record = BiffRecords { rest: &stream }.next();
        let strings = record.map(|r| sst_string_bytes(&r)).unwrap_or_default();
        assert_eq!(
            strings,
            [
                5 * XLS_UTF8_PER_CHAR,
                2 * XLS_UTF8_PER_CHAR,
                XLS_UTF8_PER_CHAR
            ]
        );
        // Dati estesi con lunghezza negativa: `calamine` si ferma, la stringa non conta.
        let mut negative = 1_u32.to_le_bytes().to_vec();
        negative.extend_from_slice(&1_u32.to_le_bytes());
        negative.extend_from_slice(&1_u16.to_le_bytes());
        negative.push(0x4);
        negative.extend_from_slice(&(-1_i32).to_le_bytes());
        negative.push(b'a');
        let stream = biff(BIFF_SST, &negative);
        let record = BiffRecords { rest: &stream }.next();
        assert_eq!(record.map(|r| sst_string_bytes(&r)), Some(Vec::new()));
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
        assert!(dims((10, 5), (0, 4)) > 1 << 32);
        // Lunghezza non prevista: nulla.
        assert_eq!(declared_cells(&[0; 12]), 0);
    }

    proptest! {
        /// Byte qualsiasi come flusso BIFF: lo svuotamento delle formule non cambia la
        /// lunghezza ed è idempotente, la stima non va mai in panic.
        #[test]
        fn any_biff_stream_is_measured_safely(
            bytes in proptest::collection::vec(any::<u8>(), 0..2048),
        ) {
            let mut once = bytes.clone();
            neutralize_xls_formulas(&mut once);
            prop_assert_eq!(once.len(), bytes.len());
            let mut twice = once.clone();
            neutralize_xls_formulas(&mut twice);
            prop_assert_eq!(&twice, &once);
            let _measured = measure_xls(&once);
        }
    }
}
