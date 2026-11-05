// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Word 97-2003 (.doc): parser proprio della piece table ([MS-DOC]) sopra il file composto.
//!
//! - Flusso `WordDocument`: la FIB in testa dà la firma (`0xA5EC`), la versione, il flag
//!   "cifrato" (anche l'offuscamento XOR), quale tabella usare (`0Table` o `1Table`), la
//!   lunghezza del testo principale (`ccpText`) e la posizione del CLX nella tabella.
//! - CLX: zero o più `Prc` (formattazioni, saltate) e un `Pcdt` con la `PlcPcd`: n+1
//!   posizioni di carattere (CP) e n descrittori di pezzo. Un pezzo "compresso" ha un byte
//!   per carattere (Windows-1252) a `fc/2`, gli altri UTF-16LE a `fc`.
//! - Si legge solo il testo principale (CP da 0 a `ccpText`): note, intestazioni, commenti e
//!   caselle di testo stanno dopo e non si estraggono.
//! - Un'unità per paragrafo non vuoto ([`Location::Paragraph`], numerati da 1): fine
//!   paragrafo `\r`, fine cella o riga di tabella (0x07) e interruzione di pagina o sezione
//!   (0x0C); 0x0B è un a capo, 0x1E un trattino. Dei campi (0x13 … 0x14 … 0x15) si tiene
//!   il risultato e non il codice; gli altri caratteri di controllo (immagini, rimandi alle
//!   note) si tolgono.
//!
//! Ogni lunghezza e offset si confronta con i byte presenti prima di usarlo: nessuna
//! allocazione da numeri dichiarati. Le CP devono crescere e la somma dei byte dei pezzi non
//! può superare il flusso `WordDocument` (pezzi sovrapposti non moltiplicano il testo).
//! Word 6 e 95 (FIB più vecchia) non sono letti: avviso "danneggiato".

use std::borrow::Cow;

use lynkeus_core::Location;

use crate::error::ExtractError;
use crate::event::Stopped;
use crate::extract::cfbfile::{Compound, to_usize, u16_at, u32_at};
use crate::extract::{Ctx, Fail, Input};

const WORD_STREAM: &str = "WordDocument";
const TABLE_0: &str = "0Table";
const TABLE_1: &str = "1Table";

/// `wIdent` della FIB.
const FIB_IDENT: u16 = 0xA5EC;
/// `nFib` minimo di Word 97 (le versioni 6 e 95 hanno FIB diverse).
const MIN_NFIB: u16 = 0x00C0;
/// Byte di `FibBase`.
const FIB_BASE_BYTES: usize = 32;
const FLAGS_OFFSET: usize = 0x0A;
const FLAG_ENCRYPTED: u16 = 0x0100;
const FLAG_TABLE_1: u16 = 0x0200;
/// Indice di `ccpText` in `FibRgLw97`.
const CCP_TEXT_INDEX: usize = 3;
/// Indice della coppia `fcClx`/`lcbClx` in `FibRgFcLcb97`.
const CLX_INDEX: usize = 33;

const CLXT_PRC: u8 = 0x01;
const CLXT_PCDT: u8 = 0x02;
/// Byte di un descrittore di pezzo (PCD).
const PCD_BYTES: usize = 8;
const FC_COMPRESSED: u32 = 0x4000_0000;
const FC_MASK: u32 = 0x3FFF_FFFF;

/// Ciò che serve della FIB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Fib {
    /// Tabella `1Table` (altrimenti `0Table`).
    pub table_one: bool,
    /// Caratteri del testo principale.
    pub ccp_text: u32,
    /// Offset del CLX nella tabella.
    pub fc_clx: u32,
    /// Byte del CLX.
    pub lcb_clx: u32,
}

/// Un pezzo del testo: le CP `[cp_start, cp_end)` stanno nel flusso a partire da `fc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Piece {
    pub cp_start: u32,
    pub cp_end: u32,
    /// Offset in byte nel flusso `WordDocument` (già diviso per 2 se compresso).
    pub fc: u32,
    /// Un byte per carattere (Windows-1252) invece di due (UTF-16LE).
    pub compressed: bool,
}

fn corrupt(reason: &str) -> ExtractError {
    ExtractError::Corrupt {
        reason: reason.to_owned(),
    }
}

/// Estrae i paragrafi di un .doc.
pub(crate) fn extract(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let mut compound = Compound::open(ctx, input)?;
    let Some(word) = compound.stream(ctx, WORD_STREAM)? else {
        // Un file composto senza testo di Word: un altro formato Office.
        return Err(ExtractError::ContentMismatch.into());
    };
    let fib = parse_fib(&word)?;
    let table_name = if fib.table_one { TABLE_1 } else { TABLE_0 };
    let Some(table) = compound.stream(ctx, table_name)? else {
        return Err(corrupt("tabella del documento assente").into());
    };
    let pieces = piece_table(&table, &fib)?;
    let mut number = 0_u32;
    main_text_paragraphs(&word, &pieces, fib.ccp_text, &mut |text| {
        number = number.saturating_add(1);
        ctx.unit(Location::Paragraph(number), text)
    })
}

/// Legge la FIB del flusso `WordDocument`. Errori: [`ExtractError::Encrypted`] per un
/// documento cifrato, [`ExtractError::Corrupt`] per una FIB non valida o di Word 6/95.
pub(crate) fn parse_fib(word: &[u8]) -> Result<Fib, ExtractError> {
    let short = || corrupt("FIB troncata");
    if u16_at(word, 0).ok_or_else(short)? != FIB_IDENT {
        return Err(corrupt("FIB senza firma di Word"));
    }
    if u16_at(word, 2).ok_or_else(short)? < MIN_NFIB {
        return Err(corrupt("Word 6/95 non supportato"));
    }
    let flags = u16_at(word, FLAGS_OFFSET).ok_or_else(short)?;
    if flags & FLAG_ENCRYPTED != 0 {
        return Err(ExtractError::Encrypted);
    }
    // FibBase, poi csw + fibRgW, cslw + fibRgLw, cbRgFcLcb + fibRgFcLcbBlob.
    let csw = usize::from(u16_at(word, FIB_BASE_BYTES).ok_or_else(short)?);
    let at_cslw = FIB_BASE_BYTES + 2 + csw * 2;
    let cslw = usize::from(u16_at(word, at_cslw).ok_or_else(short)?);
    let lw = at_cslw + 2;
    if cslw <= CCP_TEXT_INDEX {
        return Err(corrupt("FIB senza ccpText"));
    }
    let ccp_text = u32_at(word, lw + CCP_TEXT_INDEX * 4).ok_or_else(short)?;
    let at_cb = lw + cslw * 4;
    let cb = usize::from(u16_at(word, at_cb).ok_or_else(short)?);
    if cb <= CLX_INDEX {
        return Err(corrupt("FIB senza CLX"));
    }
    let clx = at_cb + 2 + CLX_INDEX * 8;
    Ok(Fib {
        table_one: flags & FLAG_TABLE_1 != 0,
        ccp_text,
        fc_clx: u32_at(word, clx).ok_or_else(short)?,
        lcb_clx: u32_at(word, clx + 4).ok_or_else(short)?,
    })
}

/// I pezzi del testo, dal CLX della tabella. Le CP devono crescere.
pub(crate) fn piece_table(table: &[u8], fib: &Fib) -> Result<Vec<Piece>, ExtractError> {
    let start = to_usize(fib.fc_clx);
    let end = start
        .checked_add(to_usize(fib.lcb_clx))
        .ok_or_else(|| corrupt("CLX fuori dalla tabella"))?;
    let clx = table
        .get(start..end)
        .ok_or_else(|| corrupt("CLX fuori dalla tabella"))?;
    let mut at = 0_usize;
    loop {
        match clx.get(at) {
            Some(&CLXT_PRC) => {
                // cbGrpprl è un intero con segno: negativo = danneggiato.
                let cb = u16_at(clx, at + 1).ok_or_else(|| corrupt("Prc troncato"))?;
                let cb = i16::from_le_bytes(cb.to_le_bytes());
                let cb = usize::try_from(cb).map_err(|_| corrupt("Prc non valido"))?;
                at = at
                    .checked_add(3 + cb)
                    .ok_or_else(|| corrupt("Prc non valido"))?;
            }
            Some(&CLXT_PCDT) => {
                let lcb = u32_at(clx, at + 1).ok_or_else(|| corrupt("Pcdt troncato"))?;
                let plc_start = at + 5;
                let plc_end = plc_start
                    .checked_add(to_usize(lcb))
                    .ok_or_else(|| corrupt("Pcdt troncato"))?;
                let plc = clx
                    .get(plc_start..plc_end)
                    .ok_or_else(|| corrupt("Pcdt troncato"))?;
                return parse_plc_pcd(plc);
            }
            _ => return Err(corrupt("CLX non valido")),
        }
    }
}

/// `PlcPcd`: n+1 CP (4 byte) seguite da n PCD (8 byte).
fn parse_plc_pcd(plc: &[u8]) -> Result<Vec<Piece>, ExtractError> {
    let invalid = || corrupt("PlcPcd non valida");
    let body = plc.len().checked_sub(4).ok_or_else(invalid)?;
    if body % (4 + PCD_BYTES) != 0 {
        return Err(invalid());
    }
    let count = body / (4 + PCD_BYTES);
    let descriptors = (count + 1) * 4;
    let mut pieces = Vec::with_capacity(count);
    for index in 0..count {
        let cp_start = u32_at(plc, index * 4).ok_or_else(invalid)?;
        let cp_end = u32_at(plc, (index + 1) * 4).ok_or_else(invalid)?;
        if cp_end < cp_start {
            return Err(invalid());
        }
        let raw = u32_at(plc, descriptors + index * PCD_BYTES + 2).ok_or_else(invalid)?;
        let compressed = raw & FC_COMPRESSED != 0;
        let fc = raw & FC_MASK;
        pieces.push(Piece {
            cp_start,
            cp_end,
            fc: if compressed { fc / 2 } else { fc },
            compressed,
        });
    }
    Ok(pieces)
}

/// Manda a `each` i paragrafi non vuoti del testo principale (CP da 0 a `ccp_text`).
pub(crate) fn main_text_paragraphs(
    word: &[u8],
    pieces: &[Piece],
    ccp_text: u32,
    each: &mut dyn FnMut(&str) -> Result<(), Stopped>,
) -> Result<(), Fail> {
    let mut builder = Paragraphs::default();
    let mut budget = word.len();
    for piece in pieces {
        if piece.cp_start >= ccp_text {
            break;
        }
        let chars = to_usize(piece.cp_end.min(ccp_text) - piece.cp_start);
        let width = if piece.compressed { 1 } else { 2 };
        let len = chars
            .checked_mul(width)
            .ok_or_else(|| corrupt("pezzo troppo lungo"))?;
        budget = budget
            .checked_sub(len)
            .ok_or_else(|| corrupt("pezzi oltre il flusso del testo"))?;
        let start = to_usize(piece.fc);
        let bytes = start
            .checked_add(len)
            .and_then(|end| word.get(start..end))
            .ok_or_else(|| corrupt("pezzo fuori dal flusso del testo"))?;
        let text = decode_piece(bytes, piece.compressed);
        for c in text.chars() {
            if let Some(paragraph) = builder.push(c)
                && !paragraph.trim().is_empty()
            {
                each(&paragraph)?;
            }
        }
    }
    if let Some(paragraph) = builder.finish()
        && !paragraph.trim().is_empty()
    {
        each(&paragraph)?;
    }
    Ok(())
}

/// Il testo di un pezzo: Windows-1252 se compresso, altrimenti UTF-16LE (sostituendo le
/// sequenze non valide).
fn decode_piece(bytes: &[u8], compressed: bool) -> Cow<'_, str> {
    if compressed {
        return encoding_rs::WINDOWS_1252
            .decode_without_bom_handling(bytes)
            .0;
    }
    let units = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair));
    Cow::Owned(
        char::decode_utf16(units)
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect(),
    )
}

/// Parte di un campo in cui ci si trova.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldPart {
    /// Tra 0x13 e 0x14: il codice del campo, escluso.
    Code,
    /// Tra 0x14 e 0x15: il risultato, incluso.
    Result,
}

/// Raccoglie i caratteri del testo in paragrafi.
#[derive(Debug, Default)]
struct Paragraphs {
    current: String,
    fields: Vec<FieldPart>,
    /// Campi aperti ancora nella parte del codice: il testo si esclude se è più di zero.
    /// Tenuto aggiornato a ogni cambio di `fields`, così ogni carattere costa O(1).
    code_fields: usize,
}

impl Paragraphs {
    /// Aggiunge un carattere; restituisce il paragrafo appena finito.
    fn push(&mut self, c: char) -> Option<String> {
        match c {
            '\u{13}' => {
                self.fields.push(FieldPart::Code);
                self.code_fields += 1;
            }
            '\u{14}' => {
                if let Some(top) = self.fields.last_mut()
                    && *top == FieldPart::Code
                {
                    *top = FieldPart::Result;
                    self.code_fields -= 1;
                }
            }
            '\u{15}' => {
                if self.fields.pop() == Some(FieldPart::Code) {
                    self.code_fields -= 1;
                }
            }
            _ if self.code_fields > 0 => {}
            '\r' | '\u{7}' | '\u{c}' => return Some(std::mem::take(&mut self.current)),
            '\u{b}' => self.current.push('\n'),
            '\t' => self.current.push('\t'),
            '\u{1e}' => self.current.push('-'),
            c if c.is_control() => {}
            c => self.current.push(c),
        }
        None
    }

    /// L'ultimo paragrafo, se il testo non finisce con un segno di paragrafo.
    fn finish(self) -> Option<String> {
        (!self.current.is_empty()).then_some(self.current)
    }
}

/// Punto d'ingresso per il fuzzing: i primi 4 byte (LE) sono la lunghezza del flusso
/// `WordDocument`, il resto la tabella. Non deve mai andare in panic. Oracoli: due letture
/// danno gli stessi paragrafi; ogni paragrafo ha testo e nessun carattere di controllo
/// (salvo tabulazione e a capo); in tutto al più `ccp_text` caratteri (un carattere per CP).
#[cfg(any(test, feature = "fuzzing"))]
pub(crate) fn fuzz(data: &[u8]) {
    let Some(split) = u32_at(data, 0) else {
        return;
    };
    let rest = data.get(4..).unwrap_or_default();
    let split = to_usize(split).min(rest.len());
    let (word, table) = rest.split_at(split);
    let Ok(fib) = parse_fib(word) else {
        return;
    };
    let Ok(pieces) = piece_table(table, &fib) else {
        return;
    };
    let read = || {
        let mut paragraphs = Vec::new();
        let ended = main_text_paragraphs(word, &pieces, fib.ccp_text, &mut |p| {
            paragraphs.push(p.to_owned());
            Ok(())
        });
        (paragraphs, ended.is_ok())
    };
    let first = read();
    assert_eq!(first, read(), "due letture diverse");
    let violation = super::paragraph_violation(&first.0, to_usize(fib.ccp_text));
    assert_eq!(violation, None, "{:?}", first.0);
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// Una FIB minima: csw = 0, cslw = 4, cbRgFcLcb = 34.
    fn fib_bytes(flags: u16, ccp_text: u32, fc_clx: u32, lcb_clx: u32) -> Vec<u8> {
        let mut fib = vec![0_u8; FIB_BASE_BYTES];
        fib[0..2].copy_from_slice(&FIB_IDENT.to_le_bytes());
        fib[2..4].copy_from_slice(&0x00C1_u16.to_le_bytes());
        fib[FLAGS_OFFSET..FLAGS_OFFSET + 2].copy_from_slice(&flags.to_le_bytes());
        fib.extend_from_slice(&0_u16.to_le_bytes()); // csw
        fib.extend_from_slice(&4_u16.to_le_bytes()); // cslw
        for value in [0_u32, 0, 0, ccp_text] {
            fib.extend_from_slice(&value.to_le_bytes());
        }
        fib.extend_from_slice(&34_u16.to_le_bytes()); // cbRgFcLcb
        for index in 0..34_usize {
            let (fc, lcb) = if index == CLX_INDEX {
                (fc_clx, lcb_clx)
            } else {
                (0, 0)
            };
            fib.extend_from_slice(&fc.to_le_bytes());
            fib.extend_from_slice(&lcb.to_le_bytes());
        }
        fib
    }

    /// Un CLX con i pezzi `(cp_start, cp_end, fc grezzo)`.
    fn clx(pieces: &[(u32, u32, u32)]) -> Vec<u8> {
        let mut plc = Vec::new();
        for (start, _, _) in pieces {
            plc.extend_from_slice(&start.to_le_bytes());
        }
        if let Some((_, end, _)) = pieces.last() {
            plc.extend_from_slice(&end.to_le_bytes());
        }
        for (_, _, fc) in pieces {
            plc.extend_from_slice(&[0, 0]);
            plc.extend_from_slice(&fc.to_le_bytes());
            plc.extend_from_slice(&[0, 0]);
        }
        let mut out = vec![CLXT_PRC, 2, 0, 0xAA, 0xBB, CLXT_PCDT];
        let len = u32::try_from(plc.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&plc);
        out
    }

    /// Un documento: testo compresso `ansi` all'offset 1024, testo UTF-16 `wide` dopo.
    fn document(ansi: &[u8], wide: &str) -> (Vec<u8>, Vec<u8>, Fib) {
        let ansi_len = u32::try_from(ansi.len()).unwrap_or(0);
        let wide_units: Vec<u16> = wide.encode_utf16().collect();
        let wide_len = u32::try_from(wide_units.len()).unwrap_or(0);
        let total = ansi_len + wide_len;
        let ansi_at = 1024_u32;
        let wide_at = ansi_at + ansi_len;
        let table = clx(&[
            (0, ansi_len, (ansi_at * 2) | FC_COMPRESSED),
            (ansi_len, total, wide_at),
        ]);
        let lcb = u32::try_from(table.len()).unwrap_or(0);
        let mut word = fib_bytes(0, total, 0, lcb);
        word.resize(1024, 0);
        word.extend_from_slice(ansi);
        for unit in wide_units {
            word.extend_from_slice(&unit.to_le_bytes());
        }
        let fib = parse_fib(&word).unwrap_or(Fib {
            table_one: false,
            ccp_text: 0,
            fc_clx: 0,
            lcb_clx: 0,
        });
        (word, table, fib)
    }

    fn paragraphs_of(word: &[u8], table: &[u8], fib: &Fib) -> Result<Vec<String>, String> {
        let pieces = piece_table(table, fib).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        main_text_paragraphs(word, &pieces, fib.ccp_text, &mut |p| {
            out.push(p.to_owned());
            Ok(())
        })
        .map_err(|e| format!("{e:?}"))?;
        Ok(out)
    }

    /// Molti campi aperti e mai chiusi (già nella parte del risultato): ogni carattere costa
    /// O(1), non O(campi aperti).
    #[test]
    fn many_open_fields_stay_linear() {
        const FIELDS: usize = 100_000;
        let started = std::time::Instant::now();
        let mut paragraphs = Paragraphs::default();
        for _ in 0..FIELDS {
            assert_eq!(paragraphs.push('\u{13}'), None);
            assert_eq!(paragraphs.push('\u{14}'), None);
        }
        for _ in 0..FIELDS {
            assert_eq!(paragraphs.push('a'), None);
        }
        // Il risultato dei campi fa parte del testo.
        assert_eq!(paragraphs.finish().map(|text| text.len()), Some(FIELDS));
        let elapsed = started.elapsed();
        assert!(elapsed < std::time::Duration::from_secs(2), "{elapsed:?}");
    }

    /// Campi annidati: codice escluso, risultato incluso, anche dentro un altro campo.
    #[test]
    fn nested_fields_keep_only_results() {
        let mut paragraphs = Paragraphs::default();
        for c in "a\u{13}X\u{13}Y\u{14}r1\u{15}Z\u{14}r2\u{15}b".chars() {
            assert_eq!(paragraphs.push(c), None);
        }
        assert_eq!(paragraphs.finish().as_deref(), Some("ar2b"));
    }

    #[test]
    fn compressed_and_unicode_pieces() {
        let (word, table, fib) = document(b"Prima\rLa citt\xE0\r", "Stasera l'aragosta \u{20AC}\r");
        assert_eq!(
            paragraphs_of(&word, &table, &fib),
            Ok(vec![
                "Prima".to_owned(),
                "La città".to_owned(),
                "Stasera l'aragosta €".to_owned()
            ])
        );
    }

    #[test]
    fn controls_fields_and_cells() {
        // Campo con codice e risultato, cella di tabella, a capo manuale, immagine (0x01).
        let ansi = b"Vedi \x13 PAGEREF x \x142\x15 qui\rA\x07B\x07\x07riga\x0Bdue\x01\r\r";
        let (word, table, fib) = document(ansi, "");
        assert_eq!(
            paragraphs_of(&word, &table, &fib),
            Ok(vec![
                "Vedi 2 qui".to_owned(),
                "A".to_owned(),
                "B".to_owned(),
                "riga\ndue".to_owned()
            ])
        );
    }

    #[test]
    fn text_after_ccp_text_is_not_main_text() {
        let (word, table, mut fib) = document(b"uno\rdue\r", "");
        fib.ccp_text = 4;
        assert_eq!(
            paragraphs_of(&word, &table, &fib),
            Ok(vec!["uno".to_owned()])
        );
    }

    #[test]
    fn fib_partitions() {
        let good = fib_bytes(FLAG_TABLE_1, 10, 20, 30);
        assert_eq!(
            parse_fib(&good).ok(),
            Some(Fib {
                table_one: true,
                ccp_text: 10,
                fc_clx: 20,
                lcb_clx: 30
            })
        );
        let encrypted = fib_bytes(FLAG_ENCRYPTED, 10, 20, 30);
        assert!(matches!(
            parse_fib(&encrypted),
            Err(ExtractError::Encrypted)
        ));
        let mut word95 = good.clone();
        word95[2..4].copy_from_slice(&0x0065_u16.to_le_bytes());
        assert!(matches!(
            parse_fib(&word95),
            Err(ExtractError::Corrupt { .. })
        ));
        let mut bad_ident = good.clone();
        bad_ident[0] = 0;
        assert!(matches!(
            parse_fib(&bad_ident),
            Err(ExtractError::Corrupt { .. })
        ));
        for len in [0, 1, 11, 33, 40, good.len() - 1] {
            assert!(
                matches!(parse_fib(&good[..len]), Err(ExtractError::Corrupt { .. })),
                "{len}"
            );
        }
    }

    #[test]
    fn piece_table_rejects_bad_tables() {
        let fib = |lcb: usize| Fib {
            table_one: false,
            ccp_text: 10,
            fc_clx: 0,
            lcb_clx: u32::try_from(lcb).unwrap_or(0),
        };
        // CLX oltre la tabella, tipo sconosciuto, Prc negativo, PlcPcd di lunghezza strana.
        assert!(piece_table(&[CLXT_PCDT], &fib(100)).is_err());
        assert!(piece_table(&[0x07, 0, 0], &fib(3)).is_err());
        assert!(piece_table(&[CLXT_PRC, 0xFF, 0xFF], &fib(3)).is_err());
        let odd = [CLXT_PCDT, 5, 0, 0, 0, 1, 2, 3, 4, 5];
        assert!(piece_table(&odd, &fib(odd.len())).is_err());
        // CP che decrescono.
        let down = clx(&[(5, 2, 0)]);
        assert!(piece_table(&down, &fib(down.len())).is_err());
        // Un Prc e un Pcdt validi.
        let good = clx(&[(0, 2, 0), (2, 4, 8)]);
        assert_eq!(
            piece_table(&good, &fib(good.len())).map(|p| p.len()).ok(),
            Some(2)
        );
    }

    #[test]
    fn overlapping_pieces_cannot_multiply_text() {
        // Mille pezzi che puntano agli stessi byte: oltre la lunghezza del flusso.
        let pieces: Vec<(u32, u32, u32)> = (0..1000_u32)
            .map(|i| (i * 100, (i + 1) * 100, 2048 | FC_COMPRESSED))
            .collect();
        let table = clx(&pieces);
        let lcb = u32::try_from(table.len()).unwrap_or(0);
        let mut word = fib_bytes(0, 100_000, 0, lcb);
        word.resize(1100, b'a');
        let Ok(fib) = parse_fib(&word) else {
            panic!("FIB non valida");
        };
        assert!(paragraphs_of(&word, &table, &fib).is_err());
    }

    #[test]
    fn piece_outside_the_stream_is_corrupt() {
        let table = clx(&[(0, 10, 5000)]);
        let lcb = u32::try_from(table.len()).unwrap_or(0);
        let word = fib_bytes(0, 10, 0, lcb);
        let Ok(fib) = parse_fib(&word) else {
            panic!("FIB non valida");
        };
        assert!(paragraphs_of(&word, &table, &fib).is_err());
    }

    proptest! {
        /// Qualsiasi input: nessun panic.
        #[test]
        fn never_panics_on_arbitrary_bytes(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
            fuzz(&data);
        }

        /// Un documento valido troncato o alterato in un punto: nessun panic, e il testo
        /// letto non supera mai il flusso.
        #[test]
        fn never_panics_on_damaged_documents(
            cut in 0_usize..2048,
            flip in 0_usize..2048,
            value in any::<u8>(),
        ) {
            let (mut word, mut table, _) = document(b"Prima riga\rseconda\r", "terza riga\r");
            if let Some(byte) = word.get_mut(flip) {
                *byte = value;
            }
            let at = flip % table.len().max(1);
            if let Some(byte) = table.get_mut(at) {
                *byte ^= value;
            }
            word.truncate(cut.max(1));
            if let Ok(fib) = parse_fib(&word)
                && let Ok(pieces) = piece_table(&table, &fib)
            {
                let mut total = 0_usize;
                let _ignored = main_text_paragraphs(&word, &pieces, fib.ccp_text, &mut |p| {
                    total += p.len();
                    Ok(())
                });
                prop_assert!(total <= word.len() * 3);
            }
            // Gli oracoli del fuzzing su input plausibili, che raggiungono più codice.
            let mut input = u32::try_from(word.len()).unwrap_or(u32::MAX).to_le_bytes().to_vec();
            input.extend_from_slice(&word);
            input.extend_from_slice(&table);
            fuzz(&input);
        }
    }
}
