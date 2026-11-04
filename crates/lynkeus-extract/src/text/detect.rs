// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Riconoscimento di binari e codifiche dai primi byte di un file.
//!
//! Ordine delle regole, dalla più sicura alla più incerta:
//!
//! 1. BOM: UTF-32 (non supportato: binario), UTF-8, UTF-16 LE, UTF-16 BE;
//! 2. firme di formati binari noti (PNG, PDF, ZIP, ...);
//! 3. NUL nei primi [`BINARY_SNIFF_BYTES`]: UTF-16 senza BOM se lo schema dei byte nulli lo
//!    spiega, altrimenti binario;
//! 4. UTF-8 valido;
//! 5. `chardetng`, con ripiego Windows-1252 se propone una codifica inadatta.
//!
//! Funzioni pure, senza I/O: il lettore passa i byte già letti.

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};

use super::TextEncoding;

/// Byte iniziali in cui un NUL indica un file binario (salvo schema UTF-16).
pub const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// Byte massimi passati a `chardetng`: bastano per decidere e limitano il tempo.
const DETECTOR_MAX_BYTES: usize = 1024 * 1024;

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";
const UTF16LE_BOM: &[u8] = b"\xFF\xFE";
const UTF16BE_BOM: &[u8] = b"\xFE\xFF";
const UTF32LE_BOM: &[u8] = b"\xFF\xFE\x00\x00";

/// Firme di formati binari che non possono essere l'inizio di un testo.
///
/// Solo firme lunghe o con byte non stampabili: "BM" o "MZ" potrebbero aprire un testo.
const BINARY_SIGNATURES: &[&[u8]] = &[
    b"\x89PNG\r\n\x1A\n",
    b"GIF87a",
    b"GIF89a",
    b"\xFF\xD8\xFF",
    b"%PDF-",
    b"PK\x03\x04",
    b"PK\x05\x06",
    b"PK\x07\x08",
    b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1",
    b"7z\xBC\xAF\x27\x1C",
    b"\x1F\x8B",
    b"\xFD7zXZ\x00",
    b"Rar!\x1A\x07",
    b"\x7FELF",
    b"OggS\x00",
    b"fLaC",
    b"\x00\x00\x01\x00",
    b"wOFF",
    b"wOF2",
    b"\x28\xB5\x2F\xFD",
    b"BZh91AY&SY",
];

/// Esito del riconoscimento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sniffed {
    /// Binario: si salta.
    Binary,
    /// Testo nella codifica `encoding`; i primi `bom_len` byte sono il BOM, da saltare.
    Text {
        encoding: TextEncoding,
        decoder: &'static Encoding,
        bom_len: usize,
    },
}

/// Riconosce binario o codifica dai primi byte `head` del file.
///
/// `complete` dice se `head` è il file intero: se no, una sequenza UTF-8 tagliata alla fine
/// di `head` non rende il testo invalido.
pub(crate) fn sniff(head: &[u8], complete: bool) -> Sniffed {
    if head.starts_with(UTF32LE_BOM) {
        return Sniffed::Binary;
    }
    if head.starts_with(UTF8_BOM) {
        return text(TextEncoding::Utf8Bom, UTF_8, UTF8_BOM.len());
    }
    if head.starts_with(UTF16LE_BOM) {
        return text(TextEncoding::Utf16Le, UTF_16LE, UTF16LE_BOM.len());
    }
    if head.starts_with(UTF16BE_BOM) {
        return text(TextEncoding::Utf16Be, UTF_16BE, UTF16BE_BOM.len());
    }
    if BINARY_SIGNATURES.iter().any(|sig| head.starts_with(sig)) {
        return Sniffed::Binary;
    }
    let sample = &head[..head.len().min(BINARY_SNIFF_BYTES)];
    if memchr::memchr(0, sample).is_some() {
        return match utf16_without_bom(sample) {
            Some(Endian::Little) => text(TextEncoding::Utf16Le, UTF_16LE, 0),
            Some(Endian::Big) => text(TextEncoding::Utf16Be, UTF_16BE, 0),
            None => Sniffed::Binary,
        };
    }
    match std::str::from_utf8(head) {
        Ok(_) => text(TextEncoding::Utf8, UTF_8, 0),
        // Sequenza tagliata solo dalla fine del campione: il resto del file la completa.
        Err(error) if error.error_len().is_none() && !complete => {
            text(TextEncoding::Utf8, UTF_8, 0)
        }
        Err(_) => {
            let legacy = guess_legacy(head);
            text(TextEncoding::Legacy(legacy.name()), legacy, 0)
        }
    }
}

fn text(encoding: TextEncoding, decoder: &'static Encoding, bom_len: usize) -> Sniffed {
    Sniffed::Text {
        encoding,
        decoder,
        bom_len,
    }
}

/// Codifica a 8 bit secondo `chardetng`; Windows-1252 se la proposta non è adatta a un file
/// di testo locale (codifiche non compatibili con ASCII o la codifica "replacement").
fn guess_legacy(head: &[u8]) -> &'static Encoding {
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Deny);
    let sample = &head[..head.len().min(DETECTOR_MAX_BYTES)];
    // `last = false`: il campione può essere solo l'inizio del file. Il valore restituito
    // (se c'era un byte non ASCII) non serve: qui ce n'è certamente uno.
    detector.feed(sample, false);
    let guess = detector.guess(None, Utf8Detection::Deny);
    if guess.is_ascii_compatible() && guess.output_encoding() == guess && guess != UTF_8 {
        guess
    } else {
        WINDOWS_1252
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Endian {
    Little,
    Big,
}

/// UTF-16 senza BOM: lo schema dei byte nulli in `sample` (che contiene almeno un NUL).
///
/// Regole, per un'endianness: nessuna unità U+0000; almeno tre quarti delle unità nell'intervallo
/// Latin-1 (è ciò che spiega i byte nulli); caratteri di controllo (salvo tab, a capo,
/// ritorno carrello e salto pagina) al massimo una unità su 64. Gli alfabeti non latini
/// senza BOM non si riconoscono: non hanno byte nulli, quindi non arrivano qui.
fn utf16_without_bom(sample: &[u8]) -> Option<Endian> {
    [Endian::Little, Endian::Big]
        .into_iter()
        .find(|&endian| looks_like_utf16(sample, endian))
}

fn looks_like_utf16(sample: &[u8], endian: Endian) -> bool {
    let units = sample.as_chunks::<2>().0.iter().map(|pair| match endian {
        Endian::Little => u16::from_le_bytes([pair[0], pair[1]]),
        Endian::Big => u16::from_be_bytes([pair[0], pair[1]]),
    });
    let (mut total, mut latin1, mut control) = (0usize, 0usize, 0usize);
    for unit in units {
        if unit == 0 {
            return false;
        }
        total += 1;
        if unit < 0x100 {
            latin1 += 1;
        }
        if unit < 0x20 && !matches!(unit, 0x09 | 0x0A | 0x0C | 0x0D) {
            control += 1;
        }
    }
    total > 0 && latin1 * 4 >= total * 3 && control * 64 <= total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoding_of(head: &[u8], complete: bool) -> Option<TextEncoding> {
        match sniff(head, complete) {
            Sniffed::Binary => None,
            Sniffed::Text { encoding, .. } => Some(encoding),
        }
    }

    fn utf16le(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn utf16be(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_be_bytes).collect()
    }

    #[test]
    fn boms() {
        let bom_len = |head: &[u8]| match sniff(head, true) {
            Sniffed::Text { bom_len, .. } => bom_len,
            Sniffed::Binary => usize::MAX,
        };
        assert_eq!(
            encoding_of(b"\xEF\xBB\xBFcitt\xC3\xA0", true),
            Some(TextEncoding::Utf8Bom)
        );
        assert_eq!(bom_len(b"\xEF\xBB\xBFa"), 3);
        let mut le = UTF16LE_BOM.to_vec();
        le.extend(utf16le("carta"));
        assert_eq!(encoding_of(&le, true), Some(TextEncoding::Utf16Le));
        assert_eq!(bom_len(&le), 2);
        let mut be = UTF16BE_BOM.to_vec();
        be.extend(utf16be("carta"));
        assert_eq!(encoding_of(&be, true), Some(TextEncoding::Utf16Be));
        assert_eq!(encoding_of(b"\xFF\xFE\x00\x00a\x00\x00\x00", true), None);
    }

    #[test]
    fn utf16_without_bom_by_pattern() {
        let le = utf16le("Una carta d'identità\r\n");
        assert_eq!(encoding_of(&le, true), Some(TextEncoding::Utf16Le));
        let be = utf16be("Una carta d'identità\r\n");
        assert_eq!(encoding_of(&be, true), Some(TextEncoding::Utf16Be));
        // Un solo carattere basta.
        assert_eq!(encoding_of(b"a\x00", true), Some(TextEncoding::Utf16Le));
    }

    #[test]
    fn nul_without_utf16_pattern_is_binary() {
        assert_eq!(encoding_of(b"ab\0cd", true), None);
        // Unità U+0000: non è testo.
        assert_eq!(encoding_of(b"a\x00\x00\x00b\x00", true), None);
        // Troppi caratteri di controllo.
        assert_eq!(encoding_of(b"\x01\x00\x02\x00\x03\x00a\x00", true), None);
    }

    #[test]
    fn nul_after_the_sniffed_bytes_is_ignored() {
        let mut late_nul = vec![b'a'; BINARY_SNIFF_BYTES];
        late_nul.push(0);
        assert_eq!(encoding_of(&late_nul, true), Some(TextEncoding::Utf8));
        let mut at_limit = vec![b'a'; BINARY_SNIFF_BYTES - 1];
        at_limit.push(0);
        assert_eq!(encoding_of(&at_limit, true), None);
    }

    #[test]
    fn known_signatures_are_binary() {
        assert_eq!(encoding_of(b"\x89PNG\r\n\x1A\n rest", true), None);
        assert_eq!(encoding_of(b"%PDF-1.7\n", true), None);
        assert_eq!(encoding_of(b"PK\x03\x04", true), None);
        // Testo che inizia come una firma corta ma non la completa.
        assert_eq!(
            encoding_of(b"%PD non e' un pdf", true),
            Some(TextEncoding::Utf8)
        );
    }

    #[test]
    fn utf8_valid_and_truncated_at_the_end_of_the_sample() {
        assert_eq!(
            encoding_of("città".as_bytes(), true),
            Some(TextEncoding::Utf8)
        );
        assert_eq!(encoding_of(b"", true), Some(TextEncoding::Utf8));
        let cut = &"città".as_bytes()[..5];
        assert_eq!(encoding_of(cut, false), Some(TextEncoding::Utf8));
        // Se il campione è il file intero, la sequenza tagliata è un errore: codifica a 8 bit.
        assert!(matches!(
            encoding_of(cut, true),
            Some(TextEncoding::Legacy(_))
        ));
    }

    #[test]
    fn legacy_italian_text_is_windows_1252() {
        let text = b"Fatelo perch\xE9 \xE8 giusto.\nUn atto di carit\xE0.\n";
        assert_eq!(
            encoding_of(text, true),
            Some(TextEncoding::Legacy("windows-1252"))
        );
    }
}
