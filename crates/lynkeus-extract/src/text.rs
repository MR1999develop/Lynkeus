// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! File di testo: rilevamento dei binari e decodifica in UTF-8.
//!
//! Implementazione provvisoria (fase 0): file letto per intero, NUL nei primi 8 KB =
//! binario, BOM UTF-8 tolto, il resto decodificato come UTF-8 con sostituzione dei byte
//! non validi. La fase 2 aggiunge BOM UTF-16, UTF-16 senza BOM, `chardetng` con ripiego
//! Windows-1252, firme binarie note e lettura a blocchi di circa 8 MB.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use lynkeus_core::ByteSize;

use crate::error::ExtractError;

/// Byte iniziali in cui un NUL indica un file binario.
pub const BINARY_SNIFF_BYTES: usize = 8 * 1024;

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

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

/// Legge e decodifica il file di testo `path`, al massimo `max_size` byte.
///
/// Errori: [`ExtractError::TooLarge`] se il file supera `max_size` (anche se cresce durante
/// la lettura), [`ExtractError::Binary`] se sembra binario, gli errori di I/O classificati
/// da [`ExtractError::from_io`].
pub fn read_text(path: &Path, max_size: ByteSize) -> Result<DecodedText, ExtractError> {
    let max = max_size.bytes();
    let file = File::open(path)?;
    let declared = file.metadata()?.len();
    if declared > max {
        return Err(ExtractError::TooLarge {
            size: declared,
            max,
        });
    }
    let capacity = usize::try_from(declared).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    let read = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if read > max {
        return Err(ExtractError::TooLarge { size: read, max });
    }
    decode(bytes)
}

/// Decodifica byte già letti (uso interno e test).
fn decode(bytes: Vec<u8>) -> Result<DecodedText, ExtractError> {
    let sniff = &bytes[..bytes.len().min(BINARY_SNIFF_BYTES)];
    if sniff.contains(&0) {
        return Err(ExtractError::Binary);
    }
    let (body, encoding) = match bytes.strip_prefix(UTF8_BOM) {
        Some(rest) => (rest.to_vec(), TextEncoding::Utf8Bom),
        None => (bytes, TextEncoding::Utf8),
    };
    match String::from_utf8(body) {
        Ok(text) => Ok(DecodedText {
            text,
            encoding,
            had_errors: false,
        }),
        Err(invalid) => Ok(DecodedText {
            text: String::from_utf8_lossy(invalid.as_bytes()).into_owned(),
            encoding,
            had_errors: true,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nul_in_the_first_8_kib_means_binary() {
        assert!(matches!(
            decode(b"ab\0cd".to_vec()),
            Err(ExtractError::Binary)
        ));
        let mut late_nul = vec![b'a'; BINARY_SNIFF_BYTES];
        late_nul.push(0);
        assert!(decode(late_nul).is_ok());
    }

    #[test]
    fn utf8_with_and_without_bom() {
        let plain = decode("città".as_bytes().to_vec()).expect("testo");
        assert_eq!(
            (plain.text.as_str(), plain.encoding),
            ("città", TextEncoding::Utf8)
        );
        let mut with_bom = UTF8_BOM.to_vec();
        with_bom.extend_from_slice("città".as_bytes());
        let bom = decode(with_bom).expect("testo");
        assert_eq!(
            (bom.text.as_str(), bom.encoding),
            ("città", TextEncoding::Utf8Bom)
        );
    }

    #[test]
    fn invalid_utf8_is_replaced_and_flagged() {
        let decoded = decode(b"carit\xE0".to_vec()).expect("testo");
        assert_eq!(decoded.text, "carit\u{FFFD}");
        assert!(decoded.had_errors);
    }
}
