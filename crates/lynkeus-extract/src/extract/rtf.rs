// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! RTF: tokenizer proprio, un'unità per paragrafo non vuoto ([`Location::Paragraph`],
//! numerati da 1).
//!
//! - Gruppi `{…}` con lo stato (destinazione saltata, `\ucN`) salvato e ripristinato;
//!   profondità massima [`MAX_GROUP_DEPTH`], oltre la quale il documento è "danneggiato".
//! - Destinazioni saltate per intero: quelle che iniziano con `\*` e quelle di
//!   [`SKIPPED_DESTINATIONS`] (tabelle di font, colori, stili, informazioni, immagini, oggetti,
//!   codici dei campi…). `\binN` salta N byte grezzi.
//! - Caratteri: `\'hh` nella code page di `\ansicpg` (Windows-1252 se assente o sconosciuta;
//!   i byte consecutivi si decodificano insieme, per le code page a più byte), `\uN` in
//!   Unicode (negativo = N + 65536, coppie surrogate unite) seguito da `\ucN` caratteri di
//!   ripiego saltati (1 se non dichiarato). I byte non ASCII fuori da `\'hh` usano la stessa
//!   code page.
//! - Paragrafi: `\par`, `\sect`, `\page`, `\cell`, `\row` e `\` seguito da un a capo, fuori
//!   dalle destinazioni saltate (quelli di una nota non spezzano il paragrafo che la
//!   contiene); `\line` è un a capo dentro il paragrafo, `\tab` una tabulazione. Gli a capo
//!   del sorgente non contano.
//!
//! Il tokenizer è lineare nei byte e non alloca da numeri del file: un parametro numerico è
//! letto con saturazione, una parola di controllo oltre [`MAX_WORD_BYTES`] è ignorata.

use encoding_rs::{Encoding, WINDOWS_1252};
use lynkeus_core::Location;

use crate::error::ExtractError;
use crate::event::Stopped;
use crate::extract::{Ctx, Fail, Input};

/// Profondità massima dei gruppi.
pub(crate) const MAX_GROUP_DEPTH: usize = 256;
/// Lunghezza massima di una parola di controllo (la specifica dice 32).
const MAX_WORD_BYTES: usize = 32;
/// Cifre massime del parametro di una parola di controllo (la specifica dice 10).
const MAX_PARAM_DIGITS: usize = 10;

/// Destinazioni il cui contenuto non è testo del documento.
const SKIPPED_DESTINATIONS: &[&str] = &[
    "fonttbl",
    "colortbl",
    "stylesheet",
    "info",
    "pict",
    "object",
    "objdata",
    "fldinst",
    "listtable",
    "listoverridetable",
    "revtbl",
    "rsidtbl",
    "generator",
    "xmlnstbl",
    "filetbl",
    "themedata",
    "colorschememapping",
    "datastore",
    "latentstyles",
    "pgdsctbl",
    "header",
    "headerl",
    "headerr",
    "headerf",
    "footer",
    "footerl",
    "footerr",
    "footerf",
    "footnote",
    "annotation",
    "private",
];

/// Estrae i paragrafi di un RTF.
pub(crate) fn extract(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let bytes = input.into_bytes(ctx)?;
    let mut number = 0_u32;
    paragraphs(&bytes, &mut |text| {
        number = number.saturating_add(1);
        ctx.unit(Location::Paragraph(number), text)
    })
}

/// Stato di un gruppo.
#[derive(Debug, Clone, Copy)]
struct Group {
    /// Destinazione saltata: nessun testo.
    skip: bool,
    /// Caratteri di ripiego dopo `\uN`.
    uc: u32,
    /// Il gruppo è appena iniziato: una parola di controllo qui può essere una destinazione.
    at_start: bool,
}

/// Il tokenizer con il paragrafo in costruzione.
struct Parser<'a, 'e> {
    bytes: &'a [u8],
    at: usize,
    groups: Vec<Group>,
    group: Group,
    encoding: &'static Encoding,
    /// Byte da decodificare insieme nella code page.
    pending: Vec<u8>,
    /// Metà alta di una coppia surrogata in attesa della bassa.
    high_surrogate: Option<u16>,
    /// Caratteri di ripiego ancora da saltare dopo un `\uN`.
    fallback: u32,
    paragraph: String,
    each: &'e mut dyn FnMut(&str) -> Result<(), Stopped>,
}

/// Manda a `each` i paragrafi non vuoti di un RTF. Errori: [`ExtractError::Corrupt`] per
/// gruppi annidati oltre [`MAX_GROUP_DEPTH`].
pub(crate) fn paragraphs(
    bytes: &[u8],
    each: &mut dyn FnMut(&str) -> Result<(), Stopped>,
) -> Result<(), Fail> {
    let mut parser = Parser {
        bytes,
        at: 0,
        groups: Vec::new(),
        group: Group {
            skip: false,
            uc: 1,
            at_start: false,
        },
        encoding: WINDOWS_1252,
        pending: Vec::new(),
        high_surrogate: None,
        fallback: 0,
        paragraph: String::new(),
        each,
    };
    parser.run()
}

impl Parser<'_, '_> {
    fn run(&mut self) -> Result<(), Fail> {
        while let Some(&byte) = self.bytes.get(self.at) {
            self.at += 1;
            match byte {
                b'{' => {
                    self.flush_pending();
                    if self.groups.len() >= MAX_GROUP_DEPTH {
                        return Err(ExtractError::Corrupt {
                            reason: "gruppi RTF troppo annidati".to_owned(),
                        }
                        .into());
                    }
                    self.groups.push(self.group);
                    self.group.at_start = true;
                    self.fallback = 0;
                }
                b'}' => {
                    self.flush_pending();
                    self.fallback = 0;
                    if let Some(outer) = self.groups.pop() {
                        self.group = outer;
                    }
                }
                b'\\' => self.control()?,
                b'\r' | b'\n' => {}
                _ => {
                    self.group.at_start = false;
                    if self.consume_fallback() {
                        continue;
                    }
                    self.byte(byte);
                }
            }
        }
        self.flush_pending();
        self.end_paragraph()
    }

    /// Un byte di testo nella code page.
    fn byte(&mut self, byte: u8) {
        if self.group.skip {
            return;
        }
        if byte.is_ascii() && self.pending.is_empty() {
            self.push_char(char::from(byte));
        } else {
            self.pending.push(byte);
        }
    }

    /// Salta un carattere di ripiego, se ne restano.
    fn consume_fallback(&mut self) -> bool {
        if self.fallback > 0 {
            self.fallback -= 1;
            return true;
        }
        false
    }

    fn flush_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let (text, _) = self.encoding.decode_without_bom_handling(&self.pending);
        if !self.group.skip {
            let text = text.into_owned();
            for c in text.chars() {
                self.push_char(c);
            }
        }
        self.pending.clear();
    }

    fn push_char(&mut self, c: char) {
        if self.group.skip {
            return;
        }
        // Tabulazioni e a capo arrivano solo da `\tab` e `\line`: quelli del sorgente sono
        // già esclusi dal tokenizer.
        if matches!(c, '\t' | '\n') || !c.is_control() {
            self.paragraph.push(c);
        }
    }

    fn end_paragraph(&mut self) -> Result<(), Fail> {
        self.flush_pending();
        let text = std::mem::take(&mut self.paragraph);
        if !text.trim().is_empty() {
            (self.each)(&text)?;
        }
        Ok(())
    }

    /// Un segno di fine paragrafo del documento: dentro una destinazione saltata (nota,
    /// commento, intestazione) appartiene al testo saltato e non chiude il paragrafo intorno.
    fn paragraph_mark(&mut self) -> Result<(), Fail> {
        if self.group.skip {
            return Ok(());
        }
        self.end_paragraph()
    }

    /// Dopo una barra rovesciata: simbolo, `\'hh` o parola di controllo.
    fn control(&mut self) -> Result<(), Fail> {
        let Some(&next) = self.bytes.get(self.at) else {
            return Ok(());
        };
        if next.is_ascii_alphabetic() {
            return self.control_word();
        }
        self.at += 1;
        let at_start = std::mem::replace(&mut self.group.at_start, false);
        match next {
            b'\'' => {
                let hex = self.bytes.get(self.at..self.at + 2);
                let value = hex
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                if let Some(value) = value {
                    self.at += 2;
                    if !self.consume_fallback() {
                        self.byte(value);
                    }
                }
            }
            b'*' if at_start => {
                self.flush_pending();
                self.group.skip = true;
            }
            b'\r' | b'\n' => self.paragraph_mark()?,
            _ if self.consume_fallback() => {}
            b'\\' | b'{' | b'}' => self.byte(next),
            b'~' => self.text_char('\u{A0}'),
            b'_' => self.text_char('-'),
            _ => {}
        }
        Ok(())
    }

    /// Un carattere dal significato fisso (non nella code page).
    fn text_char(&mut self, c: char) {
        self.flush_pending();
        self.push_char(c);
    }

    fn control_word(&mut self) -> Result<(), Fail> {
        let start = self.at;
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_alphabetic) {
            self.at += 1;
        }
        let word_bytes = self.bytes.get(start..self.at).unwrap_or_default();
        let param = self.parameter();
        // Uno spazio dopo la parola le appartiene.
        if self.bytes.get(self.at) == Some(&b' ') {
            self.at += 1;
        }
        let at_start = std::mem::replace(&mut self.group.at_start, false);
        if word_bytes.len() > MAX_WORD_BYTES {
            return Ok(());
        }
        let word = std::str::from_utf8(word_bytes).unwrap_or_default();
        if word == "bin" {
            // Dati binari: mai testo, nemmeno come ripiego.
            let skip = usize::try_from(param.unwrap_or(0).max(0)).unwrap_or(usize::MAX);
            self.at = self.at.saturating_add(skip).min(self.bytes.len());
            return Ok(());
        }
        if at_start && SKIPPED_DESTINATIONS.contains(&word) {
            self.flush_pending();
            self.group.skip = true;
            return Ok(());
        }
        match word {
            "u" => {
                self.flush_pending();
                let unit = param.unwrap_or(0);
                // Valore a 16 bit con segno: -1 è U+FFFF.
                let unit = u16::try_from(unit.rem_euclid(65_536)).unwrap_or(0xFFFD);
                self.unicode(unit);
                self.fallback = self.group.uc;
                return Ok(());
            }
            "uc" => {
                self.group.uc = u32::try_from(param.unwrap_or(1).clamp(0, 16)).unwrap_or(1);
                return Ok(());
            }
            "ansicpg" => {
                if let Some(encoding) = param.and_then(code_page) {
                    self.flush_pending();
                    self.encoding = encoding;
                }
                return Ok(());
            }
            _ => {}
        }
        if self.consume_fallback() {
            return Ok(());
        }
        match word {
            "par" | "sect" | "page" | "cell" | "row" => self.paragraph_mark()?,
            "line" => self.text_char('\n'),
            "tab" => self.text_char('\t'),
            "emdash" => self.text_char('\u{2014}'),
            "endash" => self.text_char('\u{2013}'),
            "bullet" => self.text_char('\u{2022}'),
            "lquote" => self.text_char('\u{2018}'),
            "rquote" => self.text_char('\u{2019}'),
            "ldblquote" => self.text_char('\u{201C}'),
            "rdblquote" => self.text_char('\u{201D}'),
            _ => {}
        }
        Ok(())
    }

    /// Il parametro numerico dopo una parola di controllo, con saturazione.
    fn parameter(&mut self) -> Option<i64> {
        let negative = self.bytes.get(self.at) == Some(&b'-')
            && self.bytes.get(self.at + 1).is_some_and(u8::is_ascii_digit);
        if negative {
            self.at += 1;
        }
        let mut value: i64 = 0;
        let mut digits = 0_usize;
        while let Some(&d) = self.bytes.get(self.at).filter(|d| d.is_ascii_digit()) {
            self.at += 1;
            digits += 1;
            if digits <= MAX_PARAM_DIGITS {
                value = value.saturating_mul(10).saturating_add(i64::from(d - b'0'));
            }
        }
        if digits == 0 {
            return None;
        }
        Some(if negative { -value } else { value })
    }

    /// Un'unità UTF-16 da `\uN`, con le coppie surrogate.
    fn unicode(&mut self, unit: u16) {
        let c = match (self.high_surrogate.take(), unit) {
            (None, 0xD800..=0xDBFF) => {
                self.high_surrogate = Some(unit);
                return;
            }
            (Some(high), 0xDC00..=0xDFFF) => {
                char::decode_utf16([high, unit]).next().and_then(Result::ok)
            }
            (_, unit) => char::from_u32(u32::from(unit)),
        };
        self.push_char(c.unwrap_or(char::REPLACEMENT_CHARACTER));
    }
}

/// La codifica di una code page di Windows (`\ansicpg`), o `None` se non è nota.
fn code_page(number: i64) -> Option<&'static Encoding> {
    let label = match number {
        874 => "windows-874",
        932 => "shift_jis",
        936 => "gbk",
        949 => "euc-kr",
        950 => "big5",
        1250..=1258 => return Encoding::for_label(format!("windows-{number}").as_bytes()),
        10000 => "macintosh",
        20866 => "koi8-r",
        28591 => "iso-8859-1",
        65001 => "utf-8",
        _ => return None,
    };
    Encoding::for_label(label.as_bytes())
}

/// Punto d'ingresso per il fuzzing: non deve mai andare in panic. Oracoli: due letture danno
/// gli stessi paragrafi; ogni paragrafo ha testo e nessun carattere di controllo (salvo
/// tabulazione e a capo); in tutto al più un carattere per byte dell'input (ogni carattere
/// viene da almeno un byte: testo, `\'hh`, `\uN`, parola di controllo).
#[cfg(any(test, feature = "fuzzing"))]
pub(crate) fn fuzz(data: &[u8]) {
    let read = || {
        let mut out = Vec::new();
        let ended = paragraphs(data, &mut |p| {
            out.push(p.to_owned());
            Ok(())
        });
        (out, ended.is_ok())
    };
    let first = read();
    assert_eq!(first, read(), "due letture diverse");
    let violation = super::paragraph_violation(&first.0, data.len());
    assert_eq!(violation, None, "{:?}", first.0);
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn paragraphs_of(rtf: &[u8]) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        paragraphs(rtf, &mut |p| {
            out.push(p.to_owned());
            Ok(())
        })
        .map_err(|e| format!("{e:?}"))?;
        Ok(out)
    }

    #[test]
    fn fixture_like_letter() {
        let rtf = b"{\\rtf1\\ansi\\ansicpg1252\\deff0{\\fonttbl{\\f0 Arial;}}\\pard Prima riga di prova.\\par\r\n\
La citt\\'e0 \\u232? bella, perch\\u233? c'\\u232? il mare.\\par\r\n\
Stasera si mangia l'aragosta.\\par\r\n}";
        assert_eq!(
            paragraphs_of(rtf),
            Ok(vec![
                "Prima riga di prova.".to_owned(),
                "La città è bella, perché c'è il mare.".to_owned(),
                "Stasera si mangia l'aragosta.".to_owned()
            ])
        );
    }

    #[test]
    fn destinations_are_skipped() {
        let rtf = b"{\\rtf1{\\colortbl;\\red0;}{\\stylesheet{\\s0 Normal;}}{\\info{\\title Segreto}}\
{\\*\\generator Writer;}{\\pict\\pngblip 89504e47}{\\field{\\*\\fldinst HYPERLINK x}{\\fldrslt link}}\
testo\\par}";
        assert_eq!(paragraphs_of(rtf), Ok(vec!["linktesto".to_owned()]));
    }

    /// I segni di fine paragrafo dentro una destinazione saltata (nota, commento,
    /// intestazione) non chiudono il paragrafo del testo intorno.
    #[test]
    fn paragraph_marks_inside_skipped_destinations() {
        let one = Ok(vec!["La cena di aragosta".to_owned(), "Poi".to_owned()]);
        for rtf in [
            &b"{\\rtf1 La cena{\\footnote\\pard nota\\par} di aragosta\\par Poi\\par}"[..],
            b"{\\rtf1 La cena{\\*\\annotation commento\\par altro\\cell\\row} di aragosta\\par Poi}",
            b"{\\rtf1 La cena{\\header\\pard testata\\sect\\page\\\n} di aragosta\\par Poi}",
        ] {
            assert_eq!(paragraphs_of(rtf), one, "{}", String::from_utf8_lossy(rtf));
        }
    }

    #[test]
    fn unicode_fallback_and_uc() {
        // \uc2: due caratteri di ripiego; \'hh conta come uno.
        let rtf = b"{\\rtf1\\uc2 a\\u8364\\'80\\'80b\\par}";
        assert_eq!(paragraphs_of(rtf), Ok(vec!["a€b".to_owned()]));
        // \uc0: nessun ripiego. Parametro negativo: -3913 + 65536 = U+F0B7.
        let rtf = b"{\\rtf1\\uc0\\u233\\u-3913 x\\par}";
        assert_eq!(paragraphs_of(rtf), Ok(vec!["é\u{F0B7}x".to_owned()]));
        // Coppia surrogata (U+1F600) con ripiego "?".
        let rtf = b"{\\rtf1\\u-10179?\\u-8704?\\par}";
        assert_eq!(paragraphs_of(rtf), Ok(vec!["\u{1F600}".to_owned()]));
        // Il ripiego finisce con il gruppo.
        let rtf = b"{\\rtf1{\\uc3\\u233}x\\par}";
        assert_eq!(paragraphs_of(rtf), Ok(vec!["éx".to_owned()]));
    }

    #[test]
    fn code_pages() {
        let rtf = b"{\\rtf1\\ansi\\ansicpg1251 \\'cf\\'f0\\'e8\\par}";
        assert_eq!(paragraphs_of(rtf), Ok(vec!["При".to_owned()]));
        // Shift_JIS: due byte per carattere.
        let rtf = b"{\\rtf1\\ansicpg932 \\'93\\'fa\\'96\\'7b\\par}";
        assert_eq!(paragraphs_of(rtf), Ok(vec!["日本".to_owned()]));
        // Code page sconosciuta: Windows-1252.
        let rtf = b"{\\rtf1\\ansicpg9999 citt\\'e0\\par}";
        assert_eq!(paragraphs_of(rtf), Ok(vec!["città".to_owned()]));
    }

    #[test]
    fn symbols_lines_and_cells() {
        let rtf = b"{\\rtf1 a\\{b\\}c\\\\d\\~e\\_f\\tab g\\line h\\cell i\\row\\\nj}";
        assert_eq!(
            paragraphs_of(rtf),
            Ok(vec![
                "a{b}c\\d\u{A0}e-f\tg\nh".to_owned(),
                "i".to_owned(),
                "j".to_owned()
            ])
        );
    }

    #[test]
    fn binary_data_is_never_text() {
        let rtf = b"{\\rtf1 a\\bin5 xy}zz b\\par}";
        assert_eq!(paragraphs_of(rtf), Ok(vec!["a b".to_owned()]));
        // Lunghezza oltre la fine: si ferma lì.
        assert_eq!(
            paragraphs_of(b"{\\rtf1 a\\bin99999999999999 b}"),
            Ok(vec!["a".to_owned()])
        );
    }

    #[test]
    fn group_depth_is_limited() {
        let mut deep = b"{\\rtf1 ".to_vec();
        deep.extend(std::iter::repeat_n(b'{', MAX_GROUP_DEPTH + 1));
        assert!(paragraphs_of(&deep).is_err());
        let mut ok = b"{\\rtf1 ".to_vec();
        ok.extend(std::iter::repeat_n(b'{', MAX_GROUP_DEPTH - 1));
        ok.extend_from_slice(b"x");
        assert_eq!(paragraphs_of(&ok), Ok(vec!["x".to_owned()]));
    }

    #[test]
    fn truncated_and_unbalanced_input() {
        assert_eq!(paragraphs_of(b""), Ok(Vec::new()));
        assert_eq!(paragraphs_of(b"{\\rtf1 abc"), Ok(vec!["abc".to_owned()]));
        assert_eq!(paragraphs_of(b"{\\rtf1 a}}}b"), Ok(vec!["ab".to_owned()]));
        assert_eq!(paragraphs_of(b"{\\rtf1 a\\"), Ok(vec!["a".to_owned()]));
        assert_eq!(paragraphs_of(b"{\\rtf1 a\\'e"), Ok(vec!["ae".to_owned()]));
        let long_word = format!("{{\\rtf1 a\\{} b}}", "x".repeat(100));
        assert_eq!(
            paragraphs_of(long_word.as_bytes()),
            Ok(vec!["ab".to_owned()])
        );
    }

    proptest! {
        #[test]
        fn never_panics_on_arbitrary_bytes(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
            fuzz(&data);
        }

        /// Testo fatto di pezzi RTF plausibili: nessun panic e nessun testo più lungo di
        /// tre volte l'input (ogni byte dà al più un carattere UTF-8 da 3 byte).
        #[test]
        fn output_is_bounded(pieces in proptest::collection::vec(
            prop_oneof![
                Just("{".to_owned()), Just("}".to_owned()), Just("\\par ".to_owned()),
                Just("\\'e0".to_owned()), Just("\\u233?".to_owned()), Just("\\uc2 ".to_owned()),
                Just("{\\*\\x ".to_owned()), Just("\\bin3 ".to_owned()), Just("\\ansicpg932 ".to_owned()),
                "[a-z ]{0,8}",
            ],
            0..200,
        )) {
            let rtf = pieces.concat();
            let mut total = 0_usize;
            let _ignored = paragraphs(rtf.as_bytes(), &mut |p| {
                total += p.len();
                Ok(())
            });
            prop_assert!(total <= rtf.len() * 3);
            // Gli oracoli del fuzzing su input plausibili, che raggiungono più codice.
            fuzz(rtf.as_bytes());
        }
    }
}
