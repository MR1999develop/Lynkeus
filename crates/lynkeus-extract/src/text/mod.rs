// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! File di testo: rilevamento dei binari, codifica e lettura a blocchi in UTF-8.
//!
//! [`open_text`] apre il file, legge il primo blocco e decide (vedi `detect`): binario
//! ([`ExtractError::Binary`]) oppure testo in una [`TextEncoding`]. Poi
//! [`TextReader::next_chunk`] restituisce il testo a blocchi di circa [`BLOCK_BYTES`] byte
//! letti, tagliati a fine riga, con il numero della loro prima riga: la memoria resta
//! limitata anche con file molto grandi.
//!
//! Una riga più lunga di un blocco si taglia dopo l'ultimo spazio (o, se non ce ne sono, su
//! un confine di carattere): una frase o una parola a cavallo del taglio può non essere
//! trovata, ma la memoria non esplode. Le righe finiscono con `\n`; un file con soli `\r`
//! (vecchio Mac) è una riga sola, tagliata come sopra.
//!
//! La codifica si decide sul primo blocco: un file ASCII per i primi 8 MB e Windows-1252
//! dopo ha i caratteri successivi sostituiti con U+FFFD ([`TextReader::had_errors`]).

mod detect;

use std::fs::File;
use std::io::{self, Read};
use std::num::NonZeroUsize;
use std::path::Path;
use std::thread;
use std::time::Duration;

use encoding_rs::{CoderResult, Decoder};
use lynkeus_core::ByteSize;

use crate::error::ExtractError;
use crate::platform;

pub use detect::BINARY_SNIFF_BYTES;
use detect::{Sniffed, sniff};

/// Byte letti dal file a ogni blocco (circa 8 MB).
pub const BLOCK_BYTES: usize = 8 * 1024 * 1024;

const DEFAULT_BLOCK: NonZeroUsize = match NonZeroUsize::new(BLOCK_BYTES) {
    Some(n) => n,
    None => NonZeroUsize::MIN,
};

/// Attesa prima dell'unico nuovo tentativo su un file in uso da un altro programma.
pub const SHARING_RETRY_DELAY: Duration = Duration::from_millis(50);

/// Byte decodificati per volta: limita la memoria riservata in eccesso per la codifica.
const DECODE_PIECE_BYTES: usize = 64 * 1024;

/// Codifica riconosciuta di un file di testo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextEncoding {
    /// UTF-8 senza BOM.
    Utf8,
    /// UTF-8 con BOM.
    Utf8Bom,
    /// UTF-16 little endian (con o senza BOM).
    Utf16Le,
    /// UTF-16 big endian (con o senza BOM).
    Utf16Be,
    /// Codifica a 8 bit rilevata, con il suo nome WHATWG (es. "windows-1252").
    Legacy(&'static str),
}

/// Il testo di un file, decodificato.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedText {
    /// Il testo in UTF-8, senza BOM; i terminatori di riga restano quelli del file.
    pub text: String,
    /// La codifica riconosciuta.
    pub encoding: TextEncoding,
    /// Vero se qualche sequenza non valida è stata sostituita con U+FFFD.
    pub had_errors: bool,
}

/// Un blocco di testo decodificato.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextChunk<'a> {
    /// Il testo in UTF-8. Finisce dopo un `\n`, salvo l'ultimo blocco del file e i tagli
    /// di una riga più lunga di un blocco.
    pub text: &'a str,
    /// Indice (0-based) nel file della riga in cui inizia `text`.
    pub first_line: u64,
}

/// Legge e decodifica tutto il file di testo `path`, al massimo `max_size` byte.
///
/// Comodo per testi piccoli (anteprima, test); per cercare si usa [`open_text`], che tiene
/// in memoria un blocco alla volta. Errori come [`open_text`] e [`TextReader::next_chunk`].
pub fn read_text(path: &Path, max_size: ByteSize) -> Result<DecodedText, ExtractError> {
    let mut reader = open_text(path, max_size)?;
    let mut text = String::new();
    while let Some(chunk) = reader.next_chunk()? {
        text.push_str(chunk.text);
    }
    Ok(DecodedText {
        text,
        encoding: reader.encoding(),
        had_errors: reader.had_errors(),
    })
}

/// Apre il file di testo `path` e ne riconosce la codifica leggendo il primo blocco.
///
/// Il file si apre in sola lettura con condivisione ampia (gli altri programmi possono
/// continuare a scriverlo) e, su Windows, con lettura sequenziale. Se il file è in uso da
/// un altro programma si ritenta una sola volta dopo [`SHARING_RETRY_DELAY`].
///
/// Errori: [`ExtractError::TooLarge`] oltre `max_size`, [`ExtractError::Binary`] se sembra
/// binario, gli errori di I/O classificati da [`ExtractError::from_io`].
pub fn open_text(path: &Path, max_size: ByteSize) -> Result<TextReader<File>, ExtractError> {
    retry_once_if_in_use(
        || {
            let file = platform::open_for_reading(path)?;
            let declared = file.metadata()?.len();
            if declared > max_size.bytes() {
                return Err(ExtractError::TooLarge {
                    size: declared,
                    max: max_size.bytes(),
                });
            }
            TextReader::new(file, max_size)
        },
        || thread::sleep(SHARING_RETRY_DELAY),
    )
}

/// Esegue `op`; se fallisce con [`ExtractError::InUse`], chiama `wait` e ritenta una volta.
fn retry_once_if_in_use<T>(
    mut op: impl FnMut() -> Result<T, ExtractError>,
    wait: impl FnOnce(),
) -> Result<T, ExtractError> {
    match op() {
        Err(ExtractError::InUse) => {
            wait();
            op()
        }
        other => other,
    }
}

/// Lettore a blocchi di un testo, in UTF-8.
pub struct TextReader<R> {
    source: io::Take<R>,
    max: u64,
    block_bytes: usize,
    encoding: TextEncoding,
    decoder: Decoder,
    /// Byte grezzi dell'ultima lettura (buffer riusato).
    raw: Vec<u8>,
    /// Testo decodificato non ancora restituito.
    pending: String,
    /// Testo dell'ultimo blocco restituito (buffer riusato).
    chunk: String,
    raw_read: u64,
    next_line: u64,
    eof: bool,
    had_errors: bool,
}

impl<R: Read> TextReader<R> {
    /// Lettore su `reader`, al massimo `max_size` byte, a blocchi di [`BLOCK_BYTES`].
    ///
    /// Legge subito il primo blocco per riconoscere la codifica. Errori:
    /// [`ExtractError::Binary`], [`ExtractError::TooLarge`], errori di I/O.
    pub fn new(reader: R, max_size: ByteSize) -> Result<Self, ExtractError> {
        Self::with_block_bytes(reader, max_size, DEFAULT_BLOCK)
    }

    /// Come [`TextReader::new`], con blocchi di `block_bytes` byte (per i test dei tagli).
    ///
    /// Il riconoscimento usa comunque almeno [`BINARY_SNIFF_BYTES`] byte iniziali.
    pub fn with_block_bytes(
        reader: R,
        max_size: ByteSize,
        block_bytes: NonZeroUsize,
    ) -> Result<Self, ExtractError> {
        let max = max_size.bytes();
        let mut raw = Vec::new();
        let mut source = reader.take(max.saturating_add(1));
        let first_read = block_bytes.get().max(BINARY_SNIFF_BYTES);
        let read = read_up_to(&mut source, &mut raw, first_read)?;
        let eof = read < first_read;
        let Sniffed::Text {
            encoding,
            decoder,
            bom_len,
        } = sniff(&raw, eof)
        else {
            return Err(ExtractError::Binary);
        };
        let mut this = TextReader {
            source,
            max,
            block_bytes: block_bytes.get(),
            encoding,
            decoder: decoder.new_decoder_without_bom_handling(),
            raw,
            pending: String::new(),
            chunk: String::new(),
            raw_read: 0,
            next_line: 0,
            eof,
            had_errors: false,
        };
        this.account(read)?;
        this.decode_raw(bom_len.min(read));
        Ok(this)
    }

    /// La codifica riconosciuta.
    pub fn encoding(&self) -> TextEncoding {
        self.encoding
    }

    /// Vero se finora qualche sequenza non valida è stata sostituita con U+FFFD.
    pub fn had_errors(&self) -> bool {
        self.had_errors
    }

    /// Il prossimo blocco, o `None` a fine file.
    ///
    /// Errori: [`ExtractError::TooLarge`] se il file è cresciuto oltre il massimo durante la
    /// lettura, errori di I/O classificati da [`ExtractError::from_io`].
    pub fn next_chunk(&mut self) -> Result<Option<TextChunk<'_>>, ExtractError> {
        loop {
            if let Some(cut) = self.cut_point() {
                return Ok(Some(self.emit(cut)));
            }
            if self.eof {
                return Ok(None);
            }
            self.fill()?;
        }
    }

    /// Dove tagliare `pending` per il prossimo blocco; `None` se serve leggere ancora.
    fn cut_point(&self) -> Option<usize> {
        if self.pending.is_empty() {
            return None;
        }
        if self.eof {
            return Some(self.pending.len());
        }
        if let Some(newline) = memchr::memrchr(b'\n', self.pending.as_bytes()) {
            return Some(newline + 1);
        }
        (self.pending.len() >= self.block_bytes).then(|| soft_cut(&self.pending, self.block_bytes))
    }

    /// Sposta i primi `cut` byte di `pending` nel blocco restituito.
    fn emit(&mut self, cut: usize) -> TextChunk<'_> {
        if cut >= self.pending.len() {
            std::mem::swap(&mut self.chunk, &mut self.pending);
            self.pending.clear();
        } else {
            self.chunk.clear();
            self.chunk.push_str(&self.pending[..cut]);
            self.pending.drain(..cut);
        }
        let first_line = self.next_line;
        let newlines = memchr::memchr_iter(b'\n', self.chunk.as_bytes()).count();
        self.next_line = self
            .next_line
            .saturating_add(u64::try_from(newlines).unwrap_or(u64::MAX));
        TextChunk {
            text: &self.chunk,
            first_line,
        }
    }

    /// Legge il blocco successivo e lo decodifica in coda a `pending`.
    fn fill(&mut self) -> Result<(), ExtractError> {
        let read = read_up_to(&mut self.source, &mut self.raw, self.block_bytes)?;
        self.eof = read < self.block_bytes;
        self.account(read)?;
        self.decode_raw(0);
        Ok(())
    }

    /// Conta i byte letti; oltre il massimo, [`ExtractError::TooLarge`].
    fn account(&mut self, read: usize) -> Result<(), ExtractError> {
        self.raw_read = self
            .raw_read
            .saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if self.raw_read > self.max {
            return Err(ExtractError::TooLarge {
                size: self.raw_read,
                max: self.max,
            });
        }
        Ok(())
    }

    /// Decodifica `raw[skip..]` in coda a `pending`; a fine file svuota il decodificatore.
    fn decode_raw(&mut self, skip: usize) {
        let mut rest = self.raw.get(skip..).unwrap_or_default();
        loop {
            let piece = rest.len().min(DECODE_PIECE_BYTES);
            let last = self.eof && piece == rest.len();
            let room = self
                .decoder
                .max_utf8_buffer_length(piece)
                .unwrap_or(piece.saturating_mul(3));
            self.pending.reserve(room.saturating_add(4));
            let (result, read, replaced) =
                self.decoder
                    .decode_to_string(&rest[..piece], &mut self.pending, last);
            self.had_errors |= replaced;
            rest = &rest[read..];
            if rest.is_empty() && result == CoderResult::InputEmpty {
                return;
            }
        }
    }
}

impl<R> std::fmt::Debug for TextReader<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextReader")
            .field("encoding", &self.encoding)
            .field("raw_read", &self.raw_read)
            .field("next_line", &self.next_line)
            .field("eof", &self.eof)
            .finish_non_exhaustive()
    }
}

/// Legge al massimo `wanted` byte in `buf` (svuotato prima); meno di `wanted` = fine file.
fn read_up_to(source: &mut impl Read, buf: &mut Vec<u8>, wanted: usize) -> io::Result<usize> {
    buf.clear();
    let limit = u64::try_from(wanted).unwrap_or(u64::MAX);
    source.take(limit).read_to_end(buf)
}

/// Dove tagliare `text` per non superare `max_bytes`: dopo l'ultimo spazio (White_Space
/// Unicode) entro il limite, altrimenti sull'ultimo confine di carattere entro il limite.
///
/// Restituisce sempre almeno un carattere se `text` non è vuoto, anche se `max_bytes` è più
/// piccolo del primo carattere; 0 solo per un testo vuoto.
pub fn soft_cut(text: &str, max_bytes: usize) -> usize {
    if text.len() <= max_bytes {
        return text.len();
    }
    let mut limit = max_bytes;
    while !text.is_char_boundary(limit) {
        limit -= 1;
    }
    if let Some((i, c)) = text[..limit]
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
    {
        return i + c.len_utf8();
    }
    if limit > 0 {
        return limit;
    }
    text.chars().next().map_or(0, char::len_utf8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const BIG: ByteSize = ByteSize::from_mib(64);

    fn block(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap_or(NonZeroUsize::MIN)
    }

    /// Tutti i blocchi di `bytes` letti a blocchi di `block_bytes`: (testo, prima riga).
    fn chunks(bytes: &[u8], block_bytes: usize) -> Result<Vec<(String, u64)>, ExtractError> {
        let mut reader = TextReader::with_block_bytes(bytes, BIG, block(block_bytes))?;
        let mut out = Vec::new();
        while let Some(chunk) = reader.next_chunk()? {
            out.push((chunk.text.to_owned(), chunk.first_line));
        }
        Ok(out)
    }

    fn whole(bytes: &[u8]) -> Result<(String, TextEncoding, bool), ExtractError> {
        let mut reader = TextReader::new(bytes, BIG)?;
        let mut text = String::new();
        while let Some(chunk) = reader.next_chunk()? {
            text.push_str(chunk.text);
        }
        Ok((text, reader.encoding(), reader.had_errors()))
    }

    #[test]
    fn decodes_each_encoding() {
        let utf8 = whole("città\n".as_bytes()).expect("testo");
        assert_eq!(utf8, ("città\n".to_owned(), TextEncoding::Utf8, false));

        let bom = whole(b"\xEF\xBB\xBFcitt\xC3\xA0").expect("testo");
        assert_eq!(bom, ("città".to_owned(), TextEncoding::Utf8Bom, false));

        let mut le = b"\xFF\xFE".to_vec();
        le.extend("Una carta\r\n".encode_utf16().flat_map(u16::to_le_bytes));
        let le = whole(&le).expect("testo");
        assert_eq!(
            le,
            ("Una carta\r\n".to_owned(), TextEncoding::Utf16Le, false)
        );

        let be: Vec<u8> = "carità".encode_utf16().flat_map(u16::to_be_bytes).collect();
        let be = whole(&be).expect("testo");
        assert_eq!(be, ("carità".to_owned(), TextEncoding::Utf16Be, false));

        let legacy = whole(b"Un atto di carit\xE0.\n").expect("testo");
        assert_eq!(
            legacy,
            (
                "Un atto di carità.\n".to_owned(),
                TextEncoding::Legacy("windows-1252"),
                false
            )
        );
    }

    #[test]
    fn binary_and_empty() {
        assert!(matches!(whole(b"ab\0cd"), Err(ExtractError::Binary)));
        assert_eq!(
            whole(b"").expect("vuoto"),
            (String::new(), TextEncoding::Utf8, false)
        );
        assert!(chunks(b"", 16).expect("vuoto").is_empty());
    }

    #[test]
    fn utf8_valid_at_start_and_invalid_later_is_replaced_and_flagged() {
        let mut bytes = vec![b'a'; 20];
        bytes.extend_from_slice(b"\nperch\xE9\n");
        let mut reader = TextReader::with_block_bytes(&bytes[..], BIG, block(8)).expect("testo");
        // Il primo blocco letto per il riconoscimento è di almeno 8 KB: qui tutto il file,
        // quindi UTF-8 non valido → codifica a 8 bit.
        assert!(matches!(reader.encoding(), TextEncoding::Legacy(_)));
        assert!(reader.next_chunk().expect("blocco").is_some());

        let mut late = vec![b'a'; BINARY_SNIFF_BYTES * 2];
        late.extend_from_slice(b"\nperch\xE9\n");
        let (text, encoding, had_errors) = {
            let mut reader =
                TextReader::with_block_bytes(&late[..], BIG, block(BINARY_SNIFF_BYTES))
                    .expect("testo");
            let mut text = String::new();
            while let Some(chunk) = reader.next_chunk().expect("blocco") {
                text.push_str(chunk.text);
            }
            (text, reader.encoding(), reader.had_errors())
        };
        assert_eq!(encoding, TextEncoding::Utf8);
        assert!(had_errors);
        assert!(text.ends_with("\nperch\u{FFFD}\n"));
    }

    #[test]
    fn chunks_end_at_line_ends_with_line_numbers() {
        let text = "uno\ndue\ntre\nquattro\ncinque\n";
        let found = chunks(text.as_bytes(), 1).expect("testo");
        // Il primo blocco (almeno 8 KB) contiene tutto il file.
        assert_eq!(found, [(text.to_owned(), 0)]);

        let mut long = String::new();
        for i in 0..3000 {
            long.push_str(&format!("riga {i}\n"));
        }
        let found = chunks(long.as_bytes(), 4096).expect("testo");
        assert!(found.len() > 1);
        let mut line = 0;
        for (chunk, first_line) in &found {
            assert!(chunk.ends_with('\n'));
            assert_eq!(*first_line, line);
            let first = chunk.lines().next().unwrap_or_default();
            assert_eq!(first, format!("riga {line}"));
            line += u64::try_from(chunk.matches('\n').count()).unwrap_or(0);
        }
        assert_eq!(line, 3000);
    }

    #[test]
    fn a_line_longer_than_a_block_is_cut_after_a_space() {
        let mut text = "parola ".repeat(5000);
        text.push_str("fine\nsecondo\n");
        let found = chunks(text.as_bytes(), 4096).expect("testo");
        assert!(found.len() > 2);
        for (chunk, _) in &found[..found.len() - 1] {
            assert!(
                chunk.len() <= 2 * 4096 + BINARY_SNIFF_BYTES,
                "{}",
                chunk.len()
            );
            assert!(chunk.ends_with(' ') || chunk.ends_with('\n'));
        }
        // I pezzi della riga lunga restano sulla riga 0.
        assert!(found.iter().filter(|(_, line)| *line == 0).count() > 1);
        let joined: String = found.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(joined, text);
    }

    #[test]
    fn a_line_without_spaces_is_cut_on_a_char_boundary() {
        let text = "à".repeat(20_000);
        let found = chunks(text.as_bytes(), 4097).expect("testo");
        assert!(found.len() > 1);
        let joined: String = found.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(joined, text);
    }

    #[test]
    fn growth_beyond_the_maximum_is_too_large() {
        let bytes = [b'a'; 100];
        let result = TextReader::new(&bytes[..], ByteSize::from_bytes(99));
        assert!(matches!(
            result,
            Err(ExtractError::TooLarge { size: 100, max: 99 })
        ));
        assert!(TextReader::new(&bytes[..], ByteSize::from_bytes(100)).is_ok());
    }

    #[test]
    fn soft_cut_partitions() {
        assert_eq!(soft_cut("", 0), 0);
        assert_eq!(soft_cut("abc", 10), 3);
        assert_eq!(soft_cut("ab cd ef", 6), 6);
        assert_eq!(soft_cut("ab cd ef", 5), 3);
        assert_eq!(soft_cut("abcdef", 4), 4);
        // Confine di carattere: "à" occupa 2 byte.
        assert_eq!(soft_cut("ààà", 3), 2);
        // Limite più piccolo del primo carattere: si prende comunque un carattere.
        assert_eq!(soft_cut("àb", 1), 2);
        assert_eq!(soft_cut("ab", 0), 1);
    }

    #[test]
    fn in_use_is_retried_once() {
        let mut calls = 0;
        let mut waited = false;
        let result = retry_once_if_in_use(
            || {
                calls += 1;
                if calls == 1 {
                    Err(ExtractError::InUse)
                } else {
                    Ok(calls)
                }
            },
            || waited = true,
        );
        assert_eq!(result.ok(), Some(2));
        assert!(waited);

        let mut calls = 0;
        let result: Result<(), _> = retry_once_if_in_use(
            || {
                calls += 1;
                Err(ExtractError::InUse)
            },
            || {},
        );
        assert!(matches!(result, Err(ExtractError::InUse)));
        assert_eq!(calls, 2);

        let mut calls = 0;
        let result: Result<(), _> = retry_once_if_in_use(
            || {
                calls += 1;
                Err(ExtractError::AccessDenied)
            },
            || {},
        );
        assert!(matches!(result, Err(ExtractError::AccessDenied)));
        assert_eq!(calls, 1);
    }

    fn text_strategy() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            prop_oneof![
                Just("a".to_owned()),
                Just(" ".to_owned()),
                Just("\n".to_owned()),
                Just("\r\n".to_owned()),
                Just("à".to_owned()),
                Just("€".to_owned()),
                Just("😀".to_owned()),
                "[a-z]{1,40}",
            ],
            0..8000,
        )
        .prop_map(|parts| parts.concat())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// Blocchi = intero: concatenati danno il testo, le prime righe sono coerenti.
        #[test]
        fn chunks_concatenate_to_the_whole_text(text in text_strategy(), size in 1usize..300) {
            for bytes in [
                text.as_bytes().to_vec(),
                {
                    let mut le = b"\xFF\xFE".to_vec();
                    le.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
                    le
                },
            ] {
                let found = chunks(&bytes, size).expect("testo");
                let mut joined = String::new();
                for (chunk, first_line) in &found {
                    prop_assert!(!chunk.is_empty());
                    let expected = u64::try_from(joined.matches('\n').count()).unwrap_or(0);
                    prop_assert_eq!(*first_line, expected);
                    joined.push_str(chunk);
                }
                prop_assert_eq!(&joined, &text);
            }
        }

        #[test]
        fn soft_cut_is_a_char_boundary_within_the_limit(text in text_strategy(), max in 0usize..500) {
            let cut = soft_cut(&text, max);
            prop_assert!(text.is_char_boundary(cut));
            prop_assert!(cut <= text.len());
            if !text.is_empty() {
                prop_assert!(cut > 0);
            }
            let first = text.chars().next().map_or(0, char::len_utf8);
            prop_assert!(cut <= max.max(first));
        }
    }
}
