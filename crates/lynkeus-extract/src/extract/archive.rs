// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Archivi zip, anche annidati, e le parti dei contenitori zip (docx, xlsx, pptx, ODF).
//!
//! Per ogni archivio:
//!
//! - profondità: le voci di un archivio stanno a un livello in più; se superano
//!   [`ArchiveLimits::max_depth`](crate::ArchiveLimits::max_depth) l'archivio dà l'avviso
//!   [`LimitKind::ArchiveDepth`] e non si apre;
//! - voci: contate sull'elenco prima di decomprimere, sommate su tutto il file radice;
//! - ogni voce: dimensioni dichiarate controllate subito, poi byte contati sul flusso
//!   ([`LimitedReader`]); le voci cifrate danno l'avviso "protetto da password", i
//!   collegamenti simbolici sono saltati;
//! - i nomi delle voci diventano etichette ([`entry_label`]) e decidono solo il formato:
//!   nulla si scrive su disco;
//! - le voci di testo si leggono in flusso, le altre in memoria entro il massimo per voce;
//! - quando il totale dei byte o delle voci è superato, le voci seguenti non si leggono.

use std::io::Read;

use zip::ZipArchive;
use zip::read::ZipFile;
use zip::result::ZipError;

use crate::error::ExtractError;
use crate::event::{SkipReason, Stopped, WarningKind};
use crate::extract::budget::{LimitedReader, check_declared, classify_io};
use crate::extract::{Ctx, Fail, Input, Seekable, read_document};
use crate::format::Format;
use crate::limits::LimitKind;

/// Lunghezza massima di un'etichetta, in byte.
pub(crate) const MAX_LABEL_BYTES: usize = 1024;

/// Un nome che viene dal file (voce d'archivio, foglio) reso etichetta: caratteri di
/// controllo e di direzione del testo sostituiti da U+FFFD, al più [`MAX_LABEL_BYTES`] byte.
pub(crate) fn entry_label(raw: &str) -> String {
    let mut label = String::with_capacity(raw.len().min(MAX_LABEL_BYTES));
    for c in raw.chars().map(neutralize) {
        if label.len() + c.len_utf8() > MAX_LABEL_BYTES {
            break;
        }
        label.push(c);
    }
    label
}

/// Sostituisce i caratteri che, mostrati, potrebbero ingannare (controllo, bidi).
fn neutralize(c: char) -> char {
    let bidi = matches!(
        c,
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}'
    );
    if c.is_control() || bidi {
        char::REPLACEMENT_CHARACTER
    } else {
        c
    }
}

/// Errore della libreria zip: limiti e I/O restano tali, la cifratura è "protetto da
/// password", il resto è "danneggiato".
pub(crate) fn zip_error(error: ZipError) -> ExtractError {
    match error {
        ZipError::Io(io) => classify_io(io),
        ZipError::InvalidPassword => ExtractError::Encrypted,
        ZipError::UnsupportedArchive(reason) if reason.contains("assword") => {
            ExtractError::Encrypted
        }
        _ => ExtractError::Corrupt {
            reason: "zip non valido".to_owned(),
        },
    }
}

/// Un contenitore zip aperto (archivio o documento), con la sua lunghezza in byte.
pub(crate) struct Container {
    pub zip: ZipArchive<Seekable>,
    len: u64,
}

impl Container {
    /// Apre il contenitore di `input`. Un documento con più voci del massimo per archivio
    /// dà [`LimitKind::ArchiveEntries`].
    pub fn open(ctx: &Ctx<'_>, input: Input<'_>) -> Result<Container, ExtractError> {
        let (source, len) = input.into_seekable(ctx)?;
        let zip = ZipArchive::new(source).map_err(zip_error)?;
        let entries = u64::try_from(zip.len()).unwrap_or(u64::MAX);
        if entries > ctx.limits().max_entries {
            return Err(ExtractError::LimitExceeded(LimitKind::ArchiveEntries));
        }
        Ok(Container { zip, len })
    }

    /// Vero se il contenitore ha la voce `name`.
    pub fn has(&self, name: &str) -> bool {
        self.zip.index_for_name(name).is_some()
    }

    /// Nomi delle voci, nell'ordine dell'archivio.
    pub fn names(&self) -> Vec<String> {
        self.zip
            .file_names()
            .filter_map(Result::ok)
            .map(std::borrow::Cow::into_owned)
            .collect()
    }

    /// Il flusso limitato della voce `name`, o `None` se non c'è.
    pub fn part(
        &mut self,
        ctx: &Ctx<'_>,
        name: &str,
    ) -> Result<Option<LimitedReader<ZipFile<'_, Seekable>>>, ExtractError> {
        match self.zip.index_for_name(name) {
            Some(index) => self.entry(ctx, index).map(Some),
            None => Ok(None),
        }
    }

    /// Il flusso limitato della voce `index`, dopo i controlli sulle dimensioni dichiarate.
    fn entry(
        &mut self,
        ctx: &Ctx<'_>,
        index: usize,
    ) -> Result<LimitedReader<ZipFile<'_, Seekable>>, ExtractError> {
        let meta = self.zip.by_index_data(index).map_err(zip_error)?;
        if meta.encrypted() {
            return Err(ExtractError::Encrypted);
        }
        // Una voce non può occupare più byte dell'intero contenitore.
        let compressed = meta.compressed_size().min(self.len);
        check_declared(meta.size(), compressed, ctx.limits())
            .map_err(ExtractError::LimitExceeded)?;
        let file = self.zip.by_index(index).map_err(zip_error)?;
        Ok(LimitedReader::new(file, ctx.budget(), Some(compressed)))
    }

    /// Restituisce la sorgente riportata all'inizio (per chi la rilegge, come calamine).
    pub fn into_source(self) -> Result<Seekable, ExtractError> {
        use std::io::{Seek, SeekFrom};
        let mut source = self.zip.into_inner();
        source
            .seek(SeekFrom::Start(0))
            .map_err(ExtractError::from_io)?;
        Ok(source)
    }
}

/// Metadati di una voce, letti prima di decomprimere.
struct EntryInfo {
    label: String,
    is_dir: bool,
    is_symlink: bool,
    encrypted: bool,
    size: u64,
    compressed: u64,
}

fn entry_info(container: &Container, index: usize) -> Result<EntryInfo, ExtractError> {
    let meta = container.zip.by_index_data(index).map_err(zip_error)?;
    let label = match meta.name() {
        Ok(name) => entry_label(&name),
        Err(_) => entry_label(&String::from_utf8_lossy(meta.name_raw())),
    };
    Ok(EntryInfo {
        label,
        is_dir: meta.is_dir(),
        is_symlink: meta.is_symlink(),
        encrypted: meta.encrypted(),
        size: meta.size(),
        compressed: meta.compressed_size().min(container.len),
    })
}

/// Estrae ogni voce di un archivio zip come documento a sé.
pub(crate) fn extract_zip(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let max_depth = usize::try_from(ctx.budget().max_depth()).unwrap_or(usize::MAX);
    if ctx.depth() >= max_depth {
        return Err(ExtractError::LimitExceeded(LimitKind::ArchiveDepth).into());
    }
    let mut container = Container::open(ctx, input)?;
    let count = container.zip.len();
    ctx.budget()
        .add_entries(u64::try_from(count).unwrap_or(u64::MAX))
        .map_err(ExtractError::LimitExceeded)?;
    for index in 0..count {
        let info = entry_info(&container, index)?;
        if info.is_dir {
            continue;
        }
        ctx.enter(info.label.clone());
        let result = read_entry(ctx, &mut container, index, &info);
        ctx.leave();
        result?;
        if ctx.budget().exhausted() {
            break;
        }
    }
    Ok(())
}

/// Legge una voce già "entrata" nel contesto; gli errori diventano avvisi della voce.
fn read_entry(
    ctx: &mut Ctx<'_>,
    container: &mut Container,
    index: usize,
    info: &EntryInfo,
) -> Result<(), Stopped> {
    if info.is_symlink {
        return ctx.skip(SkipReason::SpecialEntry);
    }
    if info.encrypted {
        return ctx.warn(WarningKind::Encrypted);
    }
    if let Err(kind) = check_declared(info.size, info.compressed, ctx.limits()) {
        return ctx.warn(WarningKind::LimitExceeded(kind));
    }
    let declared = Format::for_name(&info.label).unwrap_or(Format::Text);
    let mut reader = match container.entry(ctx, index) {
        Ok(reader) => reader,
        Err(error) => return ctx.report(error),
    };
    if declared == Format::Text {
        return read_document(ctx, declared, Input::Stream(&mut reader));
    }
    let mut bytes = Vec::new();
    if let Err(error) = reader.read_to_end(&mut bytes) {
        return ctx.report(classify_io(error));
    }
    drop(reader);
    read_document(ctx, declared, Input::Bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_neutralized_and_bounded() {
        assert_eq!(entry_label("../../evil.txt"), "../../evil.txt");
        assert_eq!(entry_label("a\u{0}b\nc"), "a\u{FFFD}b\u{FFFD}c");
        assert_eq!(
            entry_label("fattura\u{202E}fdp.exe"),
            "fattura\u{FFFD}fdp.exe"
        );
        let long = "è".repeat(MAX_LABEL_BYTES);
        let label = entry_label(&long);
        assert!(label.len() <= MAX_LABEL_BYTES);
        assert!(label.len() > MAX_LABEL_BYTES - 2);
    }
}
