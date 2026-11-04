// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! File composti OLE (CFB) dei formati Office 97-2003 (.doc, .ppt): apertura con `cfb` e
//! lettura dei flussi entro i limiti.
//!
//! Il file si legge in memoria entro il massimo per voce; ogni flusso si copia in memoria con
//! un [`LimitedReader`] (massimo per voce e totale del file radice), mai fidandosi della
//! lunghezza dichiarata nella directory. Un file composto non valido è "danneggiato".

use std::io::Cursor;

use cfb::CompoundFile;

use crate::error::ExtractError;
use crate::extract::budget::{LimitedReader, read_all};
use crate::extract::{Ctx, Input};

/// Un file composto aperto.
pub(crate) struct Compound {
    file: CompoundFile<Cursor<Vec<u8>>>,
}

impl Compound {
    /// Apre il file composto di `input`.
    pub fn open(ctx: &Ctx<'_>, input: Input<'_>) -> Result<Compound, ExtractError> {
        let bytes = input.into_bytes(ctx)?;
        let file = CompoundFile::open(Cursor::new(bytes)).map_err(|_| ExtractError::Corrupt {
            reason: "file composto non valido".to_owned(),
        })?;
        Ok(Compound { file })
    }

    /// Il flusso `name` (dalla radice) in memoria, o `None` se non c'è o non è un flusso.
    pub fn stream(&mut self, ctx: &Ctx<'_>, name: &str) -> Result<Option<Vec<u8>>, ExtractError> {
        if !self.file.is_stream(name) {
            return Ok(None);
        }
        let stream = self
            .file
            .open_stream(name)
            .map_err(|_| ExtractError::Corrupt {
                reason: "flusso del file composto non valido".to_owned(),
            })?;
        read_all(LimitedReader::new(stream, ctx.budget(), None)).map(Some)
    }
}

/// L'intero senza segno a 16 bit little-endian all'offset `at`, o `None` se i byte mancano.
pub(crate) fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    let end = at.checked_add(2)?;
    let raw: [u8; 2] = bytes.get(at..end)?.try_into().ok()?;
    Some(u16::from_le_bytes(raw))
}

/// L'intero senza segno a 32 bit little-endian all'offset `at`, o `None` se i byte mancano.
pub(crate) fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let end = at.checked_add(4)?;
    let raw: [u8; 4] = bytes.get(at..end)?.try_into().ok()?;
    Some(u32::from_le_bytes(raw))
}

/// Un `u32` letto dal file come indice o lunghezza in memoria.
pub(crate) fn to_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn little_endian_reads_are_bounded() {
        let bytes = [0x34, 0x12, 0x78, 0x56];
        assert_eq!(u16_at(&bytes, 0), Some(0x1234));
        assert_eq!(u16_at(&bytes, 2), Some(0x5678));
        assert_eq!(u16_at(&bytes, 3), None);
        assert_eq!(u32_at(&bytes, 0), Some(0x5678_1234));
        assert_eq!(u32_at(&bytes, 1), None);
        assert_eq!(u32_at(&bytes, usize::MAX), None);
    }
}
