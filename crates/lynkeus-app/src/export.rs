// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Esportazione e copia dei risultati, senza egui: tabella di testo, CSV, TXT tabulato e
//! TSV per gli appunti.
//!
//! - CSV: UTF-8 con BOM, separatore `;` in italiano e `,` in inglese (quello che Excel si
//!   aspetta con le impostazioni regionali delle due lingue), righe CRLF, campi tra
//!   virgolette quando contengono separatore, virgolette o a capo (RFC 4180).
//! - TXT: UTF-8 con BOM, una riga per risultato, colonne separate da tabulazioni; tabulazioni
//!   e a capo dentro una cella diventano spazi.
//! - TSV per gli appunti: come il TXT, senza BOM né intestazione.
//!
//! I file esportati si aprono spesso con un foglio di calcolo: una cella che inizia con
//! `=`, `+`, `-`, `@`, tabulazione o CR verrebbe presa per una formula (iniezione di
//! formule, threat model 4.6). Ogni cella così si prefissa con un apostrofo
//! ([`neutralize_formula`]), in tutti e tre i formati.
//!
//! La scrittura su disco è atomica e durevole ([`write_export`]): temporaneo nuovo nella
//! stessa cartella, `sync_all`, rinomina. La conferma della sovrascrittura di un file
//! esistente la chiede il dialogo di salvataggio del sistema, prima.

use std::borrow::Cow;
use std::io;
use std::path::{Path, PathBuf};

use lynkeus_engine::Hit;

use crate::format;
use crate::i18n::{I18n, Language};
use crate::settings::write_durably;

/// Byte Order Mark UTF-8: fa riconoscere la codifica a Excel e al Blocco note.
pub const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Caratteri che, all'inizio di una cella, un foglio di calcolo legge come formula.
const FORMULA_TRIGGERS: [char; 6] = ['=', '+', '-', '@', '\t', '\r'];

/// Formato del file esportato.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExportFormat {
    /// Valori separati (`;` o `,` secondo la lingua).
    #[default]
    Csv,
    /// Testo con colonne separate da tabulazioni.
    Txt,
}

impl ExportFormat {
    /// Estensione del file, senza punto.
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Txt => "txt",
        }
    }
}

/// Quali righe esportare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExportScope {
    /// Tutte le righe della tabella, nell'ordine mostrato.
    #[default]
    Visible,
    /// Solo le righe selezionate, nell'ordine mostrato.
    Selected,
}

/// Ultime scelte di esportazione, salvate in `settings.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExportSettings {
    /// Formato.
    pub format: ExportFormat,
    /// Righe.
    pub scope: ExportScope,
    /// Ultima cartella di salvataggio (separata da quella da cui si aggiungono radici).
    pub last_dir: Option<PathBuf>,
}

/// Una tabella di testo pronta da scrivere: intestazione e righe con lo stesso numero di
/// celle.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextTable {
    /// Nomi delle colonne.
    pub header: Vec<String>,
    /// Le righe.
    pub rows: Vec<Vec<String>>,
}

/// Il separatore del CSV per la lingua dell'interfaccia.
pub fn csv_separator(language: Language) -> char {
    match language {
        Language::Italian => ';',
        Language::English => ',',
    }
}

/// La cella con un apostrofo davanti se inizia con un carattere che un foglio di calcolo
/// leggerebbe come formula; altrimenti la cella così com'è.
pub fn neutralize_formula(cell: &str) -> Cow<'_, str> {
    if cell.starts_with(FORMULA_TRIGGERS) {
        Cow::Owned(format!("'{cell}"))
    } else {
        Cow::Borrowed(cell)
    }
}

/// La tabella dei risultati `hits` (indici in `results`, nell'ordine dato): Parola,
/// Termine (solo con più termini), File (percorso completo), Posizione, Contesto.
pub fn results_table(
    i18n: &I18n,
    hits: &[Hit],
    terms: &[String],
    indices: impl IntoIterator<Item = usize>,
) -> TextTable {
    let with_terms = terms.len() > 1;
    let mut header = vec![i18n.tr("column-word")];
    if with_terms {
        header.push(i18n.tr("column-term"));
    }
    header.extend(["column-file", "column-position", "column-context"].map(|id| i18n.tr(id)));
    let rows = indices
        .into_iter()
        .filter_map(|index| hits.get(index))
        .map(|hit| {
            let mut row = vec![format::word(hit)];
            if with_terms {
                row.push(format::matched_terms(terms, hit.terms));
            }
            row.push(hit.doc.to_string());
            row.push(strip_isolates(&format::location(i18n, &hit.location)));
            row.push(format::context_text(&hit.snippet));
            row
        })
        .collect();
    TextTable { header, rows }
}

/// Toglie i caratteri d'isolamento bidi che Fluent mette intorno alle variabili: nello
/// schermo sono invisibili, in un file sarebbero caratteri estranei.
fn strip_isolates(text: &str) -> String {
    text.replace(['\u{2068}', '\u{2069}'], "")
}

/// Il CSV della tabella: BOM, intestazione, righe CRLF.
pub fn to_csv(table: &TextTable, separator: char) -> Vec<u8> {
    let mut out = String::new();
    for row in std::iter::once(&table.header).chain(&table.rows) {
        for (i, cell) in row.iter().enumerate() {
            if i > 0 {
                out.push(separator);
            }
            push_csv_cell(&mut out, &neutralize_formula(cell), separator);
        }
        out.push_str("\r\n");
    }
    with_bom(&out)
}

fn push_csv_cell(out: &mut String, cell: &str, separator: char) {
    let needs_quotes = cell.contains([separator, '"', '\r', '\n']);
    if needs_quotes {
        out.push('"');
        out.push_str(&cell.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(cell);
    }
}

/// Il TXT tabulato della tabella: BOM, intestazione, righe CRLF.
pub fn to_txt(table: &TextTable) -> Vec<u8> {
    let mut out = String::new();
    for row in std::iter::once(&table.header).chain(&table.rows) {
        push_tab_row(&mut out, row);
        out.push_str("\r\n");
    }
    with_bom(&out)
}

/// Le righe come TSV per gli appunti: senza intestazione, una riga per risultato.
pub fn to_tsv(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for row in rows {
        push_tab_row(&mut out, row);
        out.push('\n');
    }
    out
}

fn push_tab_row(out: &mut String, row: &[String]) {
    for (i, cell) in row.iter().enumerate() {
        if i > 0 {
            out.push('\t');
        }
        let flat = cell.replace(['\t', '\r', '\n'], " ");
        out.push_str(&neutralize_formula(&flat));
    }
}

fn with_bom(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(UTF8_BOM.len() + text.len());
    bytes.extend_from_slice(UTF8_BOM);
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

/// I byte del file per il formato scelto.
pub fn encode(table: &TextTable, format: ExportFormat, language: Language) -> Vec<u8> {
    match format {
        ExportFormat::Csv => to_csv(table, csv_separator(language)),
        ExportFormat::Txt => to_txt(table),
    }
}

/// Scrive `bytes` in `path` in modo atomico e durevole: chi legge vede il file vecchio o
/// quello nuovo intero, mai uno a metà; se la scrittura fallisce il file vecchio resta.
pub fn write_export(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_durably(path, bytes)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use lynkeus_core::{Location, Snippet, TermSet};
    use lynkeus_engine::DocPath;
    use proptest::prelude::*;

    use super::*;

    /// Legge un CSV come lo legge un foglio di calcolo (RFC 4180), per l'andata e ritorno.
    fn parse_csv(text: &str, separator: char) -> Vec<Vec<String>> {
        let mut rows = Vec::new();
        let mut row = Vec::new();
        let mut cell = String::new();
        let mut quoted = false;
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match (quoted, c) {
                (true, '"') if chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                (true, '"') => quoted = false,
                (false, '"') => quoted = true,
                (false, '\r') if chars.peek() == Some(&'\n') => {}
                (false, '\n') => {
                    row.push(std::mem::take(&mut cell));
                    rows.push(std::mem::take(&mut row));
                }
                (false, c) if c == separator => row.push(std::mem::take(&mut cell)),
                (true | false, _) => cell.push(c),
            }
        }
        rows
    }

    fn table(rows: &[&[&str]]) -> TextTable {
        TextTable {
            header: vec!["Parola".to_owned(), "Contesto".to_owned()],
            rows: rows
                .iter()
                .map(|r| r.iter().map(|c| (*c).to_owned()).collect())
                .collect(),
        }
    }

    fn without_bom(bytes: &[u8]) -> String {
        let text = bytes.strip_prefix(UTF8_BOM).expect("BOM in testa");
        String::from_utf8(text.to_vec()).expect("UTF-8")
    }

    #[test]
    fn separator_follows_the_language() {
        assert_eq!(csv_separator(Language::Italian), ';');
        assert_eq!(csv_separator(Language::English), ',');
    }

    #[test]
    fn formula_partitions() {
        for risky in ["=1+1", "+39 02", "-carta", "@SUM(A1)", "\tx", "\rx"] {
            assert_eq!(neutralize_formula(risky), format!("'{risky}"), "{risky:?}");
        }
        for safe in ["aragosta", "", " =1", "1=1", "'già"] {
            assert_eq!(neutralize_formula(safe), safe, "{safe:?}");
        }
    }

    #[test]
    fn csv_round_trip_with_quotes_separators_and_line_breaks() {
        let source = table(&[
            &["l'aragosta", "Cuocere l'aragosta; poi \"servire\""],
            &["città", "riga uno\nriga due, con virgola"],
            &["", "è"],
        ]);
        for separator in [';', ','] {
            let bytes = to_csv(&source, separator);
            let parsed = parse_csv(&without_bom(&bytes), separator);
            let mut expected = vec![source.header.clone()];
            expected.extend(source.rows.clone());
            assert_eq!(parsed, expected, "separatore {separator:?}");
        }
    }

    #[test]
    fn csv_starts_with_bom_and_uses_crlf() {
        let bytes = to_csv(&table(&[&["a", "b"]]), ';');
        assert!(bytes.starts_with(UTF8_BOM));
        assert_eq!(without_bom(&bytes), "Parola;Contesto\r\na;b\r\n");
    }

    #[test]
    fn formulas_are_neutralized_in_every_format() {
        let source = table(&[&["=HYPERLINK(\"x\")", "-2+3"]]);
        let csv = parse_csv(&without_bom(&to_csv(&source, ',')), ',');
        assert_eq!(csv[1], ["'=HYPERLINK(\"x\")", "'-2+3"]);
        let txt = without_bom(&to_txt(&source));
        assert_eq!(txt.lines().nth(1), Some("'=HYPERLINK(\"x\")\t'-2+3"));
        let tsv = to_tsv(&source.rows);
        assert_eq!(tsv, "'=HYPERLINK(\"x\")\t'-2+3\n");
    }

    #[test]
    fn txt_and_tsv_flatten_tabs_and_line_breaks() {
        let source = table(&[&["a\tb", "uno\r\ndue"]]);
        let txt = without_bom(&to_txt(&source));
        assert_eq!(txt, "Parola\tContesto\r\na b\tuno  due\r\n");
        assert_eq!(to_tsv(&source.rows), "a b\tuno  due\n");
        // Una cella che dopo l'appiattimento inizia con uno spazio non è una formula.
        assert_eq!(to_tsv(&[vec!["\t=1".to_owned()]]), " =1\n");
    }

    #[test]
    fn results_table_columns_and_order() {
        let hit = |file: &str, line: u32, word: &str, terms: TermSet| Hit {
            doc: Arc::new(DocPath::file(file)),
            root: 0,
            location: Location::Line(line),
            word: word.to_owned(),
            word_truncated_start: false,
            word_truncated_end: true,
            snippet: Snippet {
                text: format!("prima {word} dopo"),
                highlights: Vec::new(),
                truncated_start: true,
                truncated_end: false,
            },
            terms,
        };
        let hits = vec![
            hit("a.txt", 3, "carta", TermSet::single(0)),
            hit("b.txt", 12, "aragosta", TermSet::single(1)),
        ];
        let it = I18n::new(Language::Italian);
        let one_term = results_table(&it, &hits, &["ar".to_owned()], [1, 0]);
        assert_eq!(one_term.header, ["Parola", "File", "Posizione", "Contesto"]);
        assert_eq!(
            one_term.rows[0],
            ["aragosta…", "b.txt", "riga 12", "…prima aragosta dopo"]
        );
        assert_eq!(one_term.rows[1][0], "carta…");
        let two_terms = results_table(&it, &hits, &["ca".to_owned(), "ar".to_owned()], [0]);
        assert_eq!(two_terms.header.len(), 5);
        assert_eq!(two_terms.rows[0][1], "ca");
        // Indici fuori dai limiti: ignorati.
        assert!(results_table(&it, &hits, &[], [9]).rows.is_empty());
    }

    #[test]
    fn writing_replaces_an_existing_file_atomically() {
        let dir = tempfile::tempdir().expect("cartella temporanea");
        let path = dir.path().join("risultati.csv");
        std::fs::write(&path, "vecchio contenuto più lungo del nuovo").expect("scritto");
        let bytes = encode(&table(&[&["a", "b"]]), ExportFormat::Csv, Language::Italian);
        write_export(&path, &bytes).expect("esportato");
        assert_eq!(std::fs::read(&path).expect("letto"), bytes);
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .expect("cartella")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["risultati.csv"], "nessun temporaneo lasciato");
    }

    #[test]
    fn writing_into_a_missing_place_fails_and_leaves_nothing() {
        let dir = tempfile::tempdir().expect("cartella temporanea");
        // Un file al posto della cartella: la scrittura non può riuscire.
        let blocker = dir.path().join("non-cartella");
        std::fs::write(&blocker, "x").expect("scritto");
        assert!(write_export(&blocker.join("r.csv"), b"dati").is_err());
        assert_eq!(std::fs::read(&blocker).expect("letto"), b"x");
    }

    proptest! {
        /// Andata e ritorno: ogni cella torna uguale, salvo l'apostrofo davanti alle
        /// formule.
        #[test]
        fn csv_round_trip_any_cells(
            cells in proptest::collection::vec("[a-z;,\"=+@ \\n\\r\\t-]{0,8}", 1..6),
            italian in any::<bool>(),
        ) {
            let language = if italian { Language::Italian } else { Language::English };
            let separator = csv_separator(language);
            let source = TextTable { header: cells.clone(), rows: vec![cells.clone()] };
            let parsed = parse_csv(&without_bom(&to_csv(&source, separator)), separator);
            let expected: Vec<String> =
                cells.iter().map(|c| neutralize_formula(c).into_owned()).collect();
            prop_assert_eq!(parsed.len(), 2);
            prop_assert_eq!(&parsed[1], &expected);
            for cell in &parsed[1] {
                prop_assert!(!cell.starts_with(FORMULA_TRIGGERS));
            }
        }
    }
}
