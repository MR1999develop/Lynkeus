// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Modello dell'anteprima, senza egui: quali unità di testo mostrare intorno a un
//! risultato, con le corrispondenze evidenziate.
//!
//! - **File di testo** (documento non dentro un archivio, posizione [`Location::Line`]): si
//!   rilegge il file con [`lynkeus_extract::open_text`] e si tengono le righe da
//!   `riga − PREVIEW_RADIUS` a `riga + PREVIEW_RADIUS`, ognuna al massimo
//!   [`MAX_UNIT_BYTES`] byte. La lettura è a blocchi e si ferma dopo l'ultima riga utile.
//! - **Documenti e voci d'archivio**: le unità vicine le fornisce un [`DocumentUnits`]: con
//!   il processo estrattore collegato è
//!   [`PoolDocumentUnits`](crate::extraction::PoolDocumentUnits) (cache, altrimenti
//!   rilettura nel pool); senza, [`NoDocumentUnits`], e l'anteprima mostra il contesto già
//!   presente nel risultato ([`Loaded::ContextOnly`]).
//!
//! [`load`] fa I/O: l'app la chiama in un thread suo, mai in quello dell'interfaccia, con
//! un flag d'annullamento controllato tra un blocco e l'altro.
//!
//! Le corrispondenze si ricalcolano sulle unità con la stessa query della ricerca
//! ([`lynkeus_core::find_words`]), così l'anteprima evidenzia esattamente ciò che la
//! ricerca ha trovato, anche le altre parole delle righe vicine.

use std::io::Read;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lynkeus_core::{ByteSize, CompiledQuery, Location, TextUnit, find_words};
use lynkeus_engine::{DocPath, WarningKind};
use lynkeus_extract::{ExtractError, TextReader, open_text};

use crate::format::ELLIPSIS;

/// Unità mostrate prima e dopo quella del risultato.
pub const PREVIEW_RADIUS: usize = 20;

/// Byte massimi di un'unità mostrata: una riga più lunga (un file minificato, un log) si
/// tronca, così l'anteprima resta leggera da disegnare.
pub const MAX_UNIT_BYTES: usize = 4 * 1024;

/// Dimensione massima di un file di testo riletto per l'anteprima: la stessa scala del
/// filtro più largo ammesso dal modulo (1 TiB); la lettura è comunque a blocchi.
pub const MAX_PREVIEW_FILE: ByteSize = ByteSize::from_gib(1024);

/// Come si ottiene l'anteprima di un risultato.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    /// File di testo sul disco: si rilegge.
    TextFile,
    /// Documento (PDF, Office, …) o voce d'archivio: unità dal [`DocumentUnits`].
    Document,
}

/// Il tipo d'anteprima per un risultato in `doc` alla posizione `location`.
pub fn preview_kind(doc: &DocPath, location: &Location) -> PreviewKind {
    if !doc.is_inside_archive() && matches!(location, Location::Line(_)) {
        PreviewKind::TextFile
    } else {
        PreviewKind::Document
    }
}

/// La fonte delle unità di testo dei documenti (cache e processo estrattore:
/// [`PoolDocumentUnits`](crate::extraction::PoolDocumentUnits)).
pub trait DocumentUnits: Send + Sync {
    /// Le unità di `doc` vicine a `location`: al più `radius` prima e `radius` dopo, più
    /// quella in `location`, nell'ordine del documento.
    ///
    /// `Ok(None)` se la fonte non ha (ancora) il testo del documento: l'anteprima mostra
    /// allora il contesto del risultato. `cancel` diventa vero quando l'anteprima non serve
    /// più: chi implementa lo controlla tra un passo e l'altro e può restituire `Ok(None)`.
    fn units_around(
        &self,
        doc: &DocPath,
        location: &Location,
        radius: usize,
        cancel: &AtomicBool,
    ) -> Result<Option<Vec<TextUnit>>, ExtractError>;
}

/// Nessuna fonte per i documenti (processo estrattore non collegato): sempre `Ok(None)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDocumentUnits;

impl DocumentUnits for NoDocumentUnits {
    fn units_around(
        &self,
        _doc: &DocPath,
        _location: &Location,
        _radius: usize,
        _cancel: &AtomicBool,
    ) -> Result<Option<Vec<TextUnit>>, ExtractError> {
        Ok(None)
    }
}

/// Un'unità mostrata nell'anteprima.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewUnit {
    /// Dove si trova nel documento.
    pub location: Location,
    /// Il testo, senza a capo finale; una riga troncata finisce con "…".
    pub text: String,
    /// Byte delle corrispondenze in `text`: ordinati e disgiunti.
    pub highlights: Vec<Range<usize>>,
}

/// Le unità intorno a un risultato.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewUnits {
    /// Le unità, nell'ordine del documento.
    pub units: Vec<PreviewUnit>,
    /// Indice in `units` dell'unità del risultato.
    pub focus: usize,
}

/// Esito di un caricamento riuscito.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    /// Le unità vicine, con le corrispondenze.
    Units(PreviewUnits),
    /// Nessuna unità disponibile: si mostra il contesto del risultato.
    ContextOnly,
}

/// Perché l'anteprima non si è potuta preparare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewError {
    /// Il file non si legge più: stesso motivo degli avvisi.
    Unreadable(WarningKind),
    /// Il file ora sembra binario o supera la dimensione massima: è cambiato dopo la
    /// ricerca.
    NotText,
    /// La riga del risultato non c'è più: il file è cambiato dopo la ricerca.
    Changed,
    /// Il sistema non ha avviato il thread di lettura.
    NoThread,
}

impl PreviewError {
    fn from_extract(error: &ExtractError) -> PreviewError {
        WarningKind::from_extract_error(error)
            .map_or(PreviewError::NotText, PreviewError::Unreadable)
    }
}

/// Che cosa caricare.
#[derive(Debug, Clone)]
pub struct PreviewRequest {
    /// Il documento del risultato.
    pub doc: Arc<DocPath>,
    /// La posizione del risultato.
    pub location: Location,
    /// La query della ricerca, per evidenziare.
    pub query: Arc<CompiledQuery>,
}

/// Prepara l'anteprima di `request` (fa I/O: da chiamare fuori dal thread
/// dell'interfaccia). Con `cancel` vero il risultato non conta e la lettura si ferma presto.
pub fn load(
    request: &PreviewRequest,
    documents: &dyn DocumentUnits,
    cancel: &AtomicBool,
) -> Result<Loaded, PreviewError> {
    let units = match (
        preview_kind(&request.doc, &request.location),
        &request.location,
    ) {
        (PreviewKind::TextFile, Location::Line(line)) => {
            let reader = open_text(&request.doc.path, MAX_PREVIEW_FILE)
                .map_err(|e| PreviewError::from_extract(&e))?;
            lines_around(reader, *line, PREVIEW_RADIUS, cancel)?
        }
        _ => {
            match documents.units_around(&request.doc, &request.location, PREVIEW_RADIUS, cancel) {
                Ok(Some(units)) => units,
                Ok(None) => return Ok(Loaded::ContextOnly),
                Err(error) => return Err(PreviewError::from_extract(&error)),
            }
        }
    };
    let Some(focus) = units.iter().position(|u| u.location == request.location) else {
        return Err(PreviewError::Changed);
    };
    let units = units
        .into_iter()
        .map(|unit| highlight(unit, &request.query))
        .collect();
    Ok(Loaded::Units(PreviewUnits { units, focus }))
}

/// Le righe da `line − radius` a `line + radius` (1-based) del testo letto da `reader`,
/// ognuna senza a capo finale e al più [`MAX_UNIT_BYTES`] byte (più "…" se troncata). Si ferma dopo l'ultima
/// riga utile o quando `cancel` diventa vero (restituendo quanto letto).
///
/// Se il file ha meno righe, restituisce quelle che ci sono: chi chiama controlla che la
/// riga cercata sia presente.
pub fn lines_around<R: Read>(
    mut reader: TextReader<R>,
    line: u32,
    radius: usize,
    cancel: &AtomicBool,
) -> Result<Vec<TextUnit>, PreviewError> {
    let radius = u64::try_from(radius).unwrap_or(u64::MAX);
    let first = u64::from(line).saturating_sub(radius).max(1);
    let last = u64::from(line).saturating_add(radius);
    let mut collector = LineCollector::new(first, last);
    while let Some(chunk) = reader
        .next_chunk()
        .map_err(|e| PreviewError::from_extract(&e))?
    {
        if cancel.load(Ordering::Relaxed) || collector.feed(chunk.text) {
            return Ok(collector.finish());
        }
    }
    Ok(collector.finish())
}

/// Raccoglie le righe di un intervallo da blocchi di testo che possono spezzare una riga.
struct LineCollector {
    first: u64,
    last: u64,
    /// Riga (1-based) a cui appartiene il prossimo testo.
    current: u64,
    /// Testo della riga corrente, se è nell'intervallo.
    pending: String,
    pending_truncated: bool,
    /// Vero se della riga corrente è arrivato almeno un byte.
    pending_started: bool,
    units: Vec<TextUnit>,
}

impl LineCollector {
    fn new(first: u64, last: u64) -> LineCollector {
        LineCollector {
            first,
            last,
            current: 1,
            pending: String::new(),
            pending_truncated: false,
            pending_started: false,
            units: Vec::new(),
        }
    }

    /// Aggiunge un blocco; vero se l'intervallo è completo e non serve leggere oltre.
    fn feed(&mut self, text: &str) -> bool {
        for piece in text.split_inclusive('\n') {
            let (body, ends_line) = match piece.strip_suffix('\n') {
                Some(body) => (body, true),
                None => (piece, false),
            };
            self.append(body);
            if ends_line {
                self.end_line();
                if self.current > self.last {
                    return true;
                }
            }
        }
        false
    }

    fn append(&mut self, body: &str) {
        self.pending_started = true;
        if self.current < self.first || self.current > self.last {
            return;
        }
        let room = MAX_UNIT_BYTES.saturating_sub(self.pending.len());
        if body.len() <= room {
            self.pending.push_str(body);
        } else {
            self.pending
                .push_str(&body[..body.floor_char_boundary(room)]);
            self.pending_truncated = true;
        }
    }

    fn end_line(&mut self) {
        if (self.first..=self.last).contains(&self.current) {
            let mut text = std::mem::take(&mut self.pending);
            if self.pending_truncated {
                text.push_str(ELLIPSIS);
            } else if text.ends_with('\r') {
                text.pop();
            }
            let line = u32::try_from(self.current).unwrap_or(u32::MAX);
            let unit = TextUnit {
                location: Location::Line(line),
                text,
            };
            self.units.push(unit);
        }
        self.pending_truncated = false;
        self.pending_started = false;
        self.current = self.current.saturating_add(1);
    }

    /// Le righe raccolte; l'ultima riga senza a capo finale conta se ha almeno un byte.
    fn finish(mut self) -> Vec<TextUnit> {
        if self.pending_started {
            self.end_line();
        }
        self.units
    }
}

/// L'unità con le corrispondenze della query.
fn highlight(unit: TextUnit, query: &CompiledQuery) -> PreviewUnit {
    let highlights = find_words(query, &unit.text)
        .into_iter()
        .flat_map(|found| found.highlights)
        .collect();
    PreviewUnit {
        location: unit.location,
        text: unit.text,
        highlights,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use lynkeus_core::{MatchOptions, Query, Term, WildcardMode};

    use super::*;

    fn reader(text: &str) -> TextReader<Cursor<Vec<u8>>> {
        TextReader::new(Cursor::new(text.as_bytes().to_vec()), ByteSize::from_mib(1))
            .expect("testo")
    }

    fn numbered(lines: u32) -> String {
        (1..=lines).map(|n| format!("riga {n}\n")).collect()
    }

    fn texts(units: &[TextUnit]) -> Vec<&str> {
        units.iter().map(|u| u.text.as_str()).collect()
    }

    fn not_cancelled() -> AtomicBool {
        AtomicBool::new(false)
    }

    fn query(term: &str) -> Arc<CompiledQuery> {
        let term = Term::parse(term, WildcardMode::Enabled).expect("termine");
        let query =
            Query::new(vec![term], MatchOptions::default(), Default::default()).expect("query");
        Arc::new(query.compile().expect("compilata"))
    }

    #[test]
    fn window_in_the_middle_at_the_start_and_at_the_end() {
        let text = numbered(100);
        let middle = lines_around(reader(&text), 50, 2, &not_cancelled()).expect("righe");
        assert_eq!(
            texts(&middle),
            ["riga 48", "riga 49", "riga 50", "riga 51", "riga 52"]
        );
        assert_eq!(middle[2].location, Location::Line(50));
        let start = lines_around(reader(&text), 1, 2, &not_cancelled()).expect("righe");
        assert_eq!(texts(&start), ["riga 1", "riga 2", "riga 3"]);
        let end = lines_around(reader(&text), 100, 2, &not_cancelled()).expect("righe");
        assert_eq!(texts(&end), ["riga 98", "riga 99", "riga 100"]);
    }

    #[test]
    fn crlf_last_line_without_newline_and_empty_lines() {
        let units = lines_around(reader("a\r\n\r\nc"), 2, 20, &not_cancelled()).expect("righe");
        assert_eq!(texts(&units), ["a", "", "c"]);
        assert_eq!(units[2].location, Location::Line(3));
        // Il file finisce con un a capo: nessuna riga vuota in più.
        let units = lines_around(reader("a\nb\n"), 1, 20, &not_cancelled()).expect("righe");
        assert_eq!(texts(&units), ["a", "b"]);
        // File vuoto: nessuna riga.
        let units = lines_around(reader(""), 1, 20, &not_cancelled()).expect("righe");
        assert!(units.is_empty());
    }

    #[test]
    fn a_line_beyond_the_end_is_missing() {
        let units = lines_around(reader(&numbered(3)), 10, 2, &not_cancelled()).expect("righe");
        assert!(units.iter().all(|u| u.location != Location::Line(10)));
    }

    #[test]
    fn long_lines_are_cut_on_a_char_boundary() {
        let long = "è".repeat(MAX_UNIT_BYTES);
        let text = format!("prima\n{long}\ndopo\n");
        let units = lines_around(reader(&text), 2, 1, &not_cancelled()).expect("righe");
        assert_eq!(texts(&units)[0], "prima");
        assert_eq!(texts(&units)[2], "dopo");
        let cut = units[1]
            .text
            .strip_suffix(ELLIPSIS)
            .expect("troncata con …");
        assert!(cut.len() <= MAX_UNIT_BYTES);
        assert!(cut.chars().all(|c| c == 'è'));
    }

    #[test]
    fn lines_split_across_blocks_are_joined() {
        let text = numbered(500);
        let small = std::num::NonZeroUsize::new(64).expect("non zero");
        let blocks = TextReader::with_block_bytes(
            Cursor::new(text.as_bytes().to_vec()),
            ByteSize::from_mib(1),
            small,
        )
        .expect("testo");
        let units = lines_around(blocks, 300, 1, &not_cancelled()).expect("righe");
        assert_eq!(texts(&units), ["riga 299", "riga 300", "riga 301"]);
    }

    #[test]
    fn cancel_stops_early() {
        let cancel = AtomicBool::new(true);
        let units = lines_around(reader(&numbered(100)), 50, 2, &cancel).expect("righe");
        assert!(units.len() <= 5);
    }

    #[test]
    fn kind_partitions() {
        let file = DocPath::file("a.txt");
        let entry = DocPath {
            path: "a.zip".into(),
            inner: vec!["b.txt".to_owned()],
        };
        assert_eq!(
            preview_kind(&file, &Location::Line(1)),
            PreviewKind::TextFile
        );
        assert_eq!(
            preview_kind(&entry, &Location::Line(1)),
            PreviewKind::Document
        );
        assert_eq!(
            preview_kind(&file, &Location::Paragraph(1)),
            PreviewKind::Document
        );
    }

    #[test]
    fn documents_without_a_source_show_the_context() {
        let request = PreviewRequest {
            doc: Arc::new(DocPath::file("nota.docx")),
            location: Location::Paragraph(3),
            query: query("ar"),
        };
        let loaded = load(&request, &NoDocumentUnits, &not_cancelled());
        assert_eq!(loaded, Ok(Loaded::ContextOnly));
    }

    /// Una fonte finta, come la collegherà la fase 4B.
    struct FakeUnits;

    impl DocumentUnits for FakeUnits {
        fn units_around(
            &self,
            _doc: &DocPath,
            _location: &Location,
            _radius: usize,
            _cancel: &AtomicBool,
        ) -> Result<Option<Vec<TextUnit>>, ExtractError> {
            Ok(Some(
                (1..=3)
                    .map(|p| TextUnit {
                        location: Location::Paragraph(p),
                        text: format!("paragrafo {p} con la carta"),
                    })
                    .collect(),
            ))
        }
    }

    #[test]
    fn document_units_are_highlighted_with_the_query() {
        let request = PreviewRequest {
            doc: Arc::new(DocPath::file("nota.docx")),
            location: Location::Paragraph(2),
            query: query("ar"),
        };
        let Ok(Loaded::Units(preview)) = load(&request, &FakeUnits, &not_cancelled()) else {
            panic!("attese le unità");
        };
        assert_eq!(preview.focus, 1);
        let unit = &preview.units[1];
        let marked: Vec<&str> = unit
            .highlights
            .iter()
            .map(|r| &unit.text[r.clone()])
            .collect();
        assert_eq!(marked, ["ar", "ar"], "paragrafo e carta");
    }

    #[test]
    fn a_missing_file_is_unreadable() {
        let request = PreviewRequest {
            doc: Arc::new(DocPath::file(
                std::env::temp_dir().join("lynkeus-anteprima-non-esiste.txt"),
            )),
            location: Location::Line(1),
            query: query("ar"),
        };
        assert_eq!(
            load(&request, &NoDocumentUnits, &not_cancelled()),
            Err(PreviewError::Unreadable(WarningKind::NotFound))
        );
    }
}
