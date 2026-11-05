// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Riconoscimento della codifica e lettura a blocchi dei file di testo. Il primo byte
//! sceglie la dimensione dei blocchi, il resto è il file.
//!
//! Oracoli: lo stesso file letto a blocchi piccoli e con i blocchi predefiniti dà la stessa
//! codifica e lo stesso testo; `first_line` di ogni blocco è il numero di `\n` che lo
//! precedono; un file UTF-8 riconosciuto come tale si rilegge identico. Corpus iniziale:
//! `fixtures/demo/legacy/`, `fixtures/formati/spesa.txt`, `fixtures/malevoli/riga-lunga.txt`.

#![no_main]

use std::io::Cursor;
use std::num::NonZeroUsize;

use libfuzzer_sys::fuzz_target;
use lynkeus_core::ByteSize;
use lynkeus_extract::{TextEncoding, TextReader};

/// Massimo del file: gli input del fuzzer sono piccoli.
const MAX_SIZE: ByteSize = ByteSize::from_mib(16);
/// Byte iniziali su cui il riconoscimento decide almeno, con qualunque dimensione dei
/// blocchi: entro questa lunghezza le due letture vedono lo stesso file intero.
const SNIFF_BYTES: usize = 8 * 1024;

/// Codifica e testo letti a blocchi di `block_bytes` (`None` = predefiniti), o `None` se il
/// file non è testo; controlla `first_line` di ogni blocco.
fn read_all(data: &[u8], block_bytes: Option<NonZeroUsize>) -> Option<(TextEncoding, String)> {
    let reader = match block_bytes {
        Some(bytes) => TextReader::with_block_bytes(Cursor::new(data), MAX_SIZE, bytes),
        None => TextReader::new(Cursor::new(data), MAX_SIZE),
    };
    let mut reader = reader.ok()?;
    let mut text = String::new();
    while let Ok(Some(chunk)) = reader.next_chunk() {
        let lines = text.matches('\n').count();
        assert_eq!(
            usize::try_from(chunk.first_line).ok(),
            Some(lines),
            "first_line diverso dalle righe precedenti"
        );
        text.push_str(chunk.text);
    }
    Some((reader.encoding(), text))
}

fuzz_target!(|data: &[u8]| {
    let Some((&first, file)) = data.split_first() else {
        return;
    };
    if file.len() > SNIFF_BYTES {
        return;
    }
    let block_bytes = NonZeroUsize::new(16 + usize::from(first) * 16);
    let small = read_all(file, block_bytes);
    let default = read_all(file, None);
    assert_eq!(small, default, "blocchi diversi, testo diverso");
    if let (Some((TextEncoding::Utf8, text)), Ok(original)) = (&default, std::str::from_utf8(file))
    {
        assert_eq!(text, original, "UTF-8 non riletto identico");
    }
});
