// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Archivi 7z con `sevenz-rust2` (solo lettura, un thread, senza AES), con gli stessi limiti
//! degli zip.
//!
//! - Le voci si contano sull'elenco prima di decomprimere; per ogni voce la dimensione
//!   dichiarata si controlla prima di leggerla (massimo per voce e rapporto con la
//!   dimensione dell'intero archivio, l'unica che si conosce nei blocchi solidi), poi i
//!   byte si contano sul flusso.
//! - Si decomprime blocco per blocco. In un blocco solido le voci dipendono dalle
//!   precedenti: se una voce supera i limiti, il resto del blocco non si decomprime e le sue
//!   voci hanno lo stesso avviso. Un blocco cifrato (AES) dà "protetto da password" per ogni
//!   sua voce senza decomprimere nulla; un'intestazione cifrata, per l'archivio intero.
//! - Cartelle e voci "anti" si ignorano; collegamenti simbolici e reparse point sono
//!   "saltati". Le voci senza dati sono documenti vuoti.
//! - Un blocco che non si legge (CRC, dati, memoria) non ferma i seguenti; alla fine
//!   l'archivio ha l'avviso del primo errore ("danneggiato", "memoria", …).
//! - Le etichette si assegnano a tutte le voci nell'ordine dell'archivio
//!   ([`UniqueLabels`]): le voci di un blocco sono consecutive.

use std::io::{self, Read, Seek};

use sevenz_rust2::{Archive, ArchiveEntry, BlockDecoder, EncoderMethod, Error, Password};

use crate::error::ExtractError;
use crate::event::{SkipReason, Stopped, WarningKind};
use crate::extract::archive::{UniqueLabels, check_depth, entry_label, read_entry_stream};
use crate::extract::budget::{LimitedReader, check_declared, classify_io};
use crate::extract::{Ctx, Fail, Input};
use crate::format::Format;
use crate::limits::LimitKind;

/// Attributo Windows dei reparse point (collegamenti).
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
/// Attributi con il modo Unix nei 16 bit alti.
const UNIX_EXTENSION: u32 = 0x8000;
const UNIX_TYPE_MASK: u32 = 0o170_000;
const UNIX_SYMLINK: u32 = 0o120_000;

/// Estrae ogni voce di un archivio 7z come documento a sé.
pub(crate) fn extract(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    check_depth(ctx)?;
    let (mut source, len) = input.into_seekable(ctx)?;
    let password = Password::empty();
    let archive = Archive::read(&mut source, &password).map_err(sevenz_error)?;
    let count = u64::try_from(archive.files.len()).unwrap_or(u64::MAX);
    ctx.budget()
        .add_entries(count)
        .map_err(ExtractError::LimitExceeded)?;
    let labels = labels(&archive);
    let blocks = archive.blocks.len();
    let damage = read_blocks(ctx, blocks, |ctx, block| {
        read_block(ctx, &archive, &labels, &password, &mut source, block, len)
    });
    if matches!(damage, Err(Fail::Stopped)) {
        return damage;
    }
    // Le voci senza dati non stanno in nessun blocco.
    for (index, entry) in archive.files.iter().enumerate() {
        let in_block = archive
            .stream_map
            .file_block_index
            .get(index)
            .copied()
            .flatten()
            .is_some();
        let Some(Some(label)) = labels.get(index) else {
            continue;
        };
        if in_block || ctx.budget().exhausted() {
            continue;
        }
        ctx.within(label.clone(), |ctx| {
            if is_special(entry) {
                ctx.skip(SkipReason::SpecialEntry)
            } else {
                let declared = Format::for_name(ctx.current_name()).unwrap_or(Format::Text);
                read_entry_stream(ctx, declared, &mut io::empty())
            }
        })?;
    }
    damage
}

/// Le etichette uniche delle voci, per indice in `archive.files`; `None` per cartelle e
/// voci "anti", che non sono documenti.
fn labels(archive: &Archive) -> Vec<Option<String>> {
    let mut unique = UniqueLabels::default();
    archive
        .files
        .iter()
        .map(|entry| {
            (!entry.is_directory && !entry.is_anti_item)
                .then(|| unique.unique(entry_label(&entry.name)))
        })
        .collect()
}

/// Legge i blocchi da 0 a `count` con `read`, finché il totale dei limiti non è esaurito.
/// Un blocco che fallisce non ferma i seguenti; l'esito è il primo errore (memoria,
/// cifratura, dati non validi), da segnalare per l'archivio.
fn read_blocks(
    ctx: &mut Ctx<'_>,
    count: usize,
    mut read: impl FnMut(&mut Ctx<'_>, usize) -> Result<(), Fail>,
) -> Result<(), Fail> {
    let mut first_error = None;
    for block in 0..count {
        if ctx.budget().exhausted() {
            break;
        }
        match read(ctx, block) {
            Ok(()) => {}
            Err(Fail::Stopped) => return Err(Fail::Stopped),
            Err(Fail::Error(error)) => {
                first_error.get_or_insert(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(Fail::Error(error)),
        None => Ok(()),
    }
}

/// Come si è fermata la lettura di un blocco.
enum Halt {
    /// Il destinatario ha chiesto di fermarsi.
    Stopped,
    /// Una voce supera i limiti: il resto del blocco ha lo stesso avviso.
    Limit(usize, LimitKind),
}

/// Legge le voci del blocco `block`; `labels` sono quelle di [`labels`].
fn read_block<R: Read + Seek>(
    ctx: &mut Ctx<'_>,
    archive: &Archive,
    labels: &[Option<String>],
    password: &Password,
    source: &mut R,
    block: usize,
    archive_len: u64,
) -> Result<(), Fail> {
    let decoder = BlockDecoder::new(1, block, archive, password, source);
    // Le voci di un blocco sono consecutive in `archive.files`, a partire da questa.
    let first = archive
        .stream_map
        .block_first_file_index
        .get(block)
        .copied()
        .unwrap_or_default();
    let block_label = |position: usize| -> Option<&String> {
        labels
            .get(first.saturating_add(position))
            .and_then(Option::as_ref)
    };
    let entry_count = decoder.entry_count();
    let encrypted = archive.blocks.get(block).is_some_and(|b| {
        b.coders
            .iter()
            .any(|coder| coder.encoder_method_id() == EncoderMethod::ID_AES256_SHA256)
    });
    if encrypted {
        for position in 0..entry_count {
            warn_entry(ctx, block_label(position), WarningKind::Encrypted)?;
        }
        return Ok(());
    }
    let mut halt = None;
    let mut position = 0_usize;
    let outcome = decoder.for_each_entries(&mut |entry, reader| {
        let index = position;
        position += 1;
        let Some(label) = block_label(index) else {
            return Ok(true);
        };
        let read = ctx.within(label.clone(), |ctx| {
            read_entered(ctx, entry, reader, archive_len)
        });
        match read {
            Ok(None) => Ok(!ctx.budget().exhausted()),
            Ok(Some(kind)) => {
                halt = Some(Halt::Limit(index, kind));
                Ok(false)
            }
            Err(Stopped) => {
                halt = Some(Halt::Stopped);
                Ok(false)
            }
        }
    });
    match halt {
        Some(Halt::Stopped) => return Err(Fail::Stopped),
        Some(Halt::Limit(index, kind)) => {
            for position in index + 1..entry_count {
                warn_entry(ctx, block_label(position), WarningKind::LimitExceeded(kind))?;
            }
        }
        None => {}
    }
    outcome.map_err(sevenz_error)?;
    Ok(())
}

/// Un avviso per una voce non letta (cartelle e voci "anti", senza etichetta, escluse).
fn warn_entry(ctx: &mut Ctx<'_>, label: Option<&String>, kind: WarningKind) -> Result<(), Stopped> {
    match label {
        Some(label) => ctx.within(label.clone(), |ctx| ctx.warn(kind)),
        None => Ok(()),
    }
}

/// Legge una voce già "entrata" nel contesto; `Some` se la voce supera i limiti dichiarati
/// (e il blocco va abbandonato).
fn read_entered(
    ctx: &mut Ctx<'_>,
    entry: &ArchiveEntry,
    reader: &mut dyn Read,
    archive_len: u64,
) -> Result<Option<LimitKind>, Stopped> {
    if is_special(entry) {
        ctx.skip(SkipReason::SpecialEntry)?;
        return Ok(None);
    }
    if let Err(kind) = check_declared(entry.size, archive_len, ctx.limits()) {
        ctx.warn(WarningKind::LimitExceeded(kind))?;
        return Ok(Some(kind));
    }
    let declared = Format::for_name(ctx.current_name()).unwrap_or(Format::Text);
    let mut limited = LimitedReader::new(reader, ctx.budget(), Some(archive_len));
    read_entry_stream(ctx, declared, &mut limited)?;
    Ok(None)
}

// Vero per i collegamenti simbolici (modo Unix) e i reparse point (Windows).
fn is_special(entry: &ArchiveEntry) -> bool {
    if !entry.has_windows_attributes {
        return false;
    }
    let attributes = entry.windows_attributes;
    let unix_symlink =
        attributes & UNIX_EXTENSION != 0 && (attributes >> 16) & UNIX_TYPE_MASK == UNIX_SYMLINK;
    unix_symlink || attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// Errore di `sevenz-rust2`: cifratura, memoria e limiti hanno il loro avviso, il resto è
/// "danneggiato". Il testo dell'errore non si conserva.
fn sevenz_error(error: Error) -> ExtractError {
    match error {
        Error::PasswordRequired | Error::MaybeBadPassword(_) => ExtractError::Encrypted,
        Error::UnsupportedCompressionMethod(method) if method.contains("AES") => {
            ExtractError::Encrypted
        }
        Error::MaxMemLimited { .. } => ExtractError::LimitExceeded(LimitKind::Memory),
        Error::Io(io, _) => match classify_io(io) {
            ExtractError::Io(_) => corrupt(),
            other => other,
        },
        _ => corrupt(),
    }
}

fn corrupt() -> ExtractError {
    ExtractError::Corrupt {
        reason: "7z non valido".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Il primo errore dei blocchi resta quello dell'archivio, anche se poi ne arrivano
    /// altri; i blocchi seguenti si leggono comunque.
    #[test]
    fn first_block_error_is_kept() {
        let mut events: Vec<crate::event::ExtractEvent> = Vec::new();
        let mut ctx = Ctx::new(
            &mut events,
            crate::limits::ArchiveLimits::default(),
            String::new(),
        );
        let mut read = Vec::new();
        let result = read_blocks(&mut ctx, 3, |_, block| {
            read.push(block);
            match block {
                0 => Err(ExtractError::LimitExceeded(LimitKind::Memory).into()),
                1 => Err(corrupt().into()),
                _ => Ok(()),
            }
        });
        assert!(matches!(
            result,
            Err(Fail::Error(ExtractError::LimitExceeded(LimitKind::Memory)))
        ));
        assert_eq!(read, [0, 1, 2]);
    }

    #[test]
    fn special_entries() {
        let mut entry = ArchiveEntry::new_file("link");
        assert!(!is_special(&entry));
        entry.has_windows_attributes = true;
        entry.windows_attributes = UNIX_EXTENSION | (0o120_777 << 16);
        assert!(is_special(&entry));
        entry.windows_attributes = UNIX_EXTENSION | (0o100_644 << 16);
        assert!(!is_special(&entry));
        entry.windows_attributes = FILE_ATTRIBUTE_REPARSE_POINT;
        assert!(is_special(&entry));
    }

    #[test]
    fn error_partitions() {
        assert!(matches!(
            sevenz_error(Error::PasswordRequired),
            ExtractError::Encrypted
        ));
        assert!(matches!(
            sevenz_error(Error::UnsupportedCompressionMethod(
                "AES256_SHA256".to_owned()
            )),
            ExtractError::Encrypted
        ));
        assert!(matches!(
            sevenz_error(Error::UnsupportedCompressionMethod("PPMD".to_owned())),
            ExtractError::Corrupt { .. }
        ));
        assert!(matches!(
            sevenz_error(Error::ChecksumVerificationFailed),
            ExtractError::Corrupt { .. }
        ));
    }
}
