// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Testo semplice (le voci degli archivi senza un formato del registro): il modulo
//! [`text`](crate::text), con le stesse regole dei file di testo del processo principale.
//! Un'unità per blocco, con la posizione della sua prima riga ([`Location::Line`]).

use std::num::NonZeroUsize;

use lynkeus_core::Location;

use crate::extract::{Ctx, Fail, Input};
use crate::text::TextReader;

/// Byte letti per blocco: con il testo decodificato (al più circa tre volte tanto) un blocco
/// sta comodo in un messaggio del protocollo.
const TEXT_BLOCK_BYTES: NonZeroUsize = match NonZeroUsize::new(1024 * 1024) {
    Some(n) => n,
    None => NonZeroUsize::MIN,
};

/// Estrae un testo: binario → saltato, codifica riconosciuta come nei file di testo.
pub(crate) fn extract(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let max = ctx.limits().max_entry_size;
    let mut reader = TextReader::with_block_bytes(input.into_reader(), max, TEXT_BLOCK_BYTES)?;
    while let Some(chunk) = reader.next_chunk()? {
        let line = u32::try_from(chunk.first_line.saturating_add(1)).unwrap_or(u32::MAX);
        ctx.unit(Location::Line(line), chunk.text)?;
    }
    Ok(())
}
