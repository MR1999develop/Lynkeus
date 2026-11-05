// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Archivi tar e file gzip (`.gz`, `.tar.gz`, `.tgz`), con gli stessi limiti degli zip.
//!
//! - **tar**: le voci si leggono in sequenza; ognuna è contata tra le voci del file radice
//!   quando la si incontra (il tar non ha un elenco in testa), con la dimensione dichiarata
//!   controllata prima di leggerla e i byte contati sul flusso ([`LimitedReader`]). Solo i
//!   file regolari sono documenti: cartelle e intestazioni PAX globali si ignorano,
//!   collegamenti, dispositivi e file sparsi sono "saltati". I nomi (anche quelli lunghi
//!   PAX e GNU) sono solo etichette.
//! - **gzip** (anche con più membri concatenati): il contenuto è un documento interno con
//!   il nome del file senza `.gz` (`appunti.txt.gz` → `appunti.txt`), un livello più in
//!   profondità; il suo formato viene dal nome. Il rapporto di compressione si controlla sui
//!   byte del file compresso.
//! - **`.tar.gz` e `.tgz`** (o un `.gz` il cui contenuto ha la firma tar): un solo archivio,
//!   con le voci del tar direttamente sotto il file. Il flusso decompresso del tar si
//!   limita al massimo per archivio e al rapporto di compressione; i byte li contano le
//!   voci.

use std::io::{self, BufReader, Read};

use flate2::read::MultiGzDecoder;

use crate::error::ExtractError;
use crate::event::{SkipReason, Stopped, WarningKind};
use crate::extract::archive::{UniqueLabels, check_depth, entry_label, read_entry_stream};
use crate::extract::budget::{LimitedReader, check_declared, classify_io};
use crate::extract::{Ctx, Fail, Input};
use crate::format::{Format, SNIFF_BYTES, Signature};

/// Nome del contenuto di un `.gz` che non si ricava dal nome del file.
const UNNAMED_CONTENT: &str = "contenuto";

/// Estrae ogni voce di un archivio tar come documento a sé.
pub(crate) fn extract_tar(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    check_depth(ctx)?;
    read_tar(ctx, input.into_reader())
}

/// Estrae il contenuto di un file gzip: un documento, o le voci di un tar.
pub(crate) fn extract_gz(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    check_depth(ctx)?;
    let name = content_name(ctx.current_name());
    let (source, compressed) = input.into_seekable(ctx)?;
    let decoder = GzipErrors(MultiGzDecoder::new(BufReader::new(source)));
    if Format::for_name(&name) == Some(Format::Tar) {
        let stream = LimitedReader::for_container(decoder, ctx.budget(), compressed);
        return read_tar(ctx, stream);
    }
    let mut stream = LimitedReader::new(decoder, ctx.budget(), Some(compressed));
    // Un .gz con un tar dentro ma senza ".tar" nel nome: lo si riconosce dal contenuto.
    let mut head = Vec::with_capacity(SNIFF_BYTES);
    let limit = u64::try_from(SNIFF_BYTES).unwrap_or(u64::MAX);
    (&mut stream)
        .take(limit)
        .read_to_end(&mut head)
        .map_err(classify_io)?;
    let signature = Signature::sniff(&head);
    let mut whole = io::Cursor::new(head).chain(stream);
    if signature == Signature::Tar {
        return read_tar(ctx, whole);
    }
    ctx.budget()
        .add_entries(1)
        .map_err(ExtractError::LimitExceeded)?;
    let declared = Format::for_name(&name).unwrap_or(Format::Text);
    ctx.within(entry_label(&name), |ctx| {
        read_entry_stream(ctx, declared, &mut whole)
    })?;
    Ok(())
}

/// Il nome del contenuto di un file gzip chiamato `name`.
fn content_name(name: &str) -> String {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let Some((stem, extension)) = file.rsplit_once('.') else {
        return UNNAMED_CONTENT.to_owned();
    };
    if stem.is_empty() {
        UNNAMED_CONTENT.to_owned()
    } else if extension.eq_ignore_ascii_case("tgz") {
        format!("{stem}.tar")
    } else {
        stem.to_owned()
    }
}

/// Le voci di un tar letto da `reader`.
fn read_tar(ctx: &mut Ctx<'_>, reader: impl Read) -> Result<(), Fail> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(tar_error)?;
    let mut labels = UniqueLabels::default();
    for entry in entries {
        let mut entry = entry.map_err(tar_error)?;
        let kind = entry.header().entry_type();
        if kind.is_dir() || kind.is_pax_global_extensions() {
            continue;
        }
        ctx.budget()
            .add_entries(1)
            .map_err(ExtractError::LimitExceeded)?;
        let label = labels.unique(entry_label(&String::from_utf8_lossy(&entry.path_bytes())));
        let regular = kind.is_file() || kind.is_contiguous();
        let size = entry.size();
        ctx.within(label, |ctx| {
            if regular {
                read_tar_entry(ctx, &mut entry, size)
            } else {
                ctx.skip(SkipReason::SpecialEntry)
            }
        })?;
        if ctx.budget().exhausted() {
            break;
        }
    }
    Ok(())
}

/// Legge una voce regolare già "entrata" nel contesto; `size` è la dimensione dichiarata.
fn read_tar_entry(ctx: &mut Ctx<'_>, entry: &mut dyn Read, size: u64) -> Result<(), Stopped> {
    // Il tar non comprime: il rapporto non si applica alla voce.
    if let Err(kind) = check_declared(size, size, ctx.limits()) {
        return ctx.warn(WarningKind::LimitExceeded(kind));
    }
    let declared = Format::for_name(ctx.current_name()).unwrap_or(Format::Text);
    let mut reader = LimitedReader::new(entry, ctx.budget(), None);
    read_entry_stream(ctx, declared, &mut reader)
}

/// Errore della libreria tar: limiti e I/O restano tali, il resto è "danneggiato".
fn tar_error(error: io::Error) -> ExtractError {
    match classify_io(error) {
        ExtractError::Io(_) => ExtractError::Corrupt {
            reason: "tar non valido".to_owned(),
        },
        other => other,
    }
}

/// Riporta gli errori di intestazione di gzip (`InvalidInput`) a dati non validi.
struct GzipErrors<R>(R);

impl<R: Read> Read for GzipErrors<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf).map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidInput {
                io::Error::new(io::ErrorKind::InvalidData, "gzip non valido")
            } else {
                error
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::LimitKind;

    #[test]
    fn content_name_partitions() {
        assert_eq!(content_name("appunti.txt.gz"), "appunti.txt");
        assert_eq!(content_name("archivio.tar.gz"), "archivio.tar");
        assert_eq!(content_name("archivio.TGZ"), "archivio.tar");
        assert_eq!(content_name("cartella/log.gz"), "log");
        assert_eq!(content_name(".gz"), UNNAMED_CONTENT);
        assert_eq!(content_name("senza-estensione"), UNNAMED_CONTENT);
    }

    /// Due voci con lo stesso nome in un tar (la libreria zip invece ne tiene una sola):
    /// documenti distinti, con le stesse etichette a ogni lettura.
    #[test]
    fn duplicate_tar_entries_are_distinct_documents() {
        let mut builder = tar::Builder::new(Vec::new());
        for data in [&b"uno"[..], b"due"] {
            let mut header = tar::Header::new_gnu();
            header.set_size(u64::try_from(data.len()).unwrap_or(0));
            header.set_mode(0o644);
            header.set_cksum();
            assert!(builder.append_data(&mut header, "nota.txt", data).is_ok());
        }
        let bytes = builder.into_inner().unwrap_or_default();
        for _ in 0..2 {
            let mut events = Vec::new();
            let result = crate::extract::extract_bytes(
                "doppio.tar",
                bytes.clone(),
                &crate::extract::ExtractOptions::default(),
                &mut events,
            );
            assert_eq!(result, Ok(()));
            let documents: Vec<Vec<String>> = events
                .iter()
                .filter_map(|event| match event {
                    crate::event::ExtractEvent::DocumentStart { inner, .. } => Some(inner.clone()),
                    _ => None,
                })
                .collect();
            assert_eq!(
                documents,
                [vec!["nota.txt".to_owned()], vec!["nota (2).txt".to_owned()]]
            );
        }
    }

    #[test]
    fn limit_kind_is_kept() {
        assert!(matches!(
            tar_error(crate::extract::budget::limit_error(LimitKind::ArchiveSize)),
            ExtractError::LimitExceeded(LimitKind::ArchiveSize)
        ));
        assert!(matches!(
            tar_error(io::Error::other("x")),
            ExtractError::Corrupt { .. }
        ));
    }
}
