// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Limiti sulle risorse di un file radice: byte decompressi contati sul flusso, voci e
//! profondità degli archivi.
//!
//! - [`Budget`] conta byte e voci dell'intero file radice (archivi annidati e parti di docx
//!   compresi), così livelli ciascuno sotto il limite non lo superano insieme.
//! - [`LimitedReader`] avvolge ogni flusso decompresso: `Read::take` sul massimo per voce più
//!   uno (oltre non si decomprime nulla), poi a ogni lettura controlla voce, rapporto di
//!   compressione e totale. Un limite superato è un `io::Error` che porta [`LimitHit`]:
//!   attraversa le librerie (zip, quick-xml, lettore di testo) e
//!   [`classify_io`] lo riconosce.
//! - [`check_declared`] rifiuta subito le voci che già nell'intestazione dichiarano dimensioni
//!   oltre i limiti; le dimensioni dichiarate non bastano mai da sole, perché possono mentire.

use std::cell::Cell;
use std::fmt;
use std::io::{self, Read};
use std::rc::Rc;

use crate::error::ExtractError;
use crate::limits::{ArchiveLimits, LimitKind};

/// Byte decompressi oltre i quali vale il controllo del rapporto di compressione: un piccolo
/// file molto ripetitivo (un XML di poche righe) può superare 200:1 senza essere una bomba.
pub(crate) const RATIO_GRACE_BYTES: u64 = 1024 * 1024;

/// Profondità massima comunque ammessa, qualunque sia il limite configurato: ogni livello è
/// una chiamata ricorsiva.
pub(crate) const MAX_DEPTH_CEILING: u32 = 32;

/// Contatori dell'intero file radice.
#[derive(Debug)]
pub(crate) struct Budget {
    limits: ArchiveLimits,
    bytes: Cell<u64>,
    entries: Cell<u64>,
    exhausted: Cell<bool>,
}

impl Budget {
    /// Contatori a zero con i limiti `limits`.
    pub fn new(limits: ArchiveLimits) -> Budget {
        Budget {
            limits,
            bytes: Cell::new(0),
            entries: Cell::new(0),
            exhausted: Cell::new(false),
        }
    }

    /// I limiti configurati.
    pub fn limits(&self) -> &ArchiveLimits {
        &self.limits
    }

    /// Profondità massima degli archivi, al più [`MAX_DEPTH_CEILING`].
    pub fn max_depth(&self) -> u32 {
        self.limits.max_depth.min(MAX_DEPTH_CEILING)
    }

    /// Conta `n` byte decompressi; oltre il totale per archivio, [`LimitKind::ArchiveSize`].
    pub fn add_bytes(&self, n: u64) -> Result<(), LimitKind> {
        let total = self.bytes.get().saturating_add(n);
        self.bytes.set(total);
        if total > self.limits.max_archive_size.bytes() {
            self.exhausted.set(true);
            return Err(LimitKind::ArchiveSize);
        }
        Ok(())
    }

    /// Conta `n` voci d'archivio; oltre il massimo, [`LimitKind::ArchiveEntries`].
    pub fn add_entries(&self, n: u64) -> Result<(), LimitKind> {
        let total = self.entries.get().saturating_add(n);
        self.entries.set(total);
        if total > self.limits.max_entries {
            self.exhausted.set(true);
            return Err(LimitKind::ArchiveEntries);
        }
        Ok(())
    }

    /// Vero dopo che il totale dei byte o delle voci è stato superato: le voci seguenti non
    /// si leggono più.
    pub fn exhausted(&self) -> bool {
        self.exhausted.get()
    }
}

/// Il limite superato, dentro un `io::Error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LimitHit(pub LimitKind);

impl fmt::Display for LimitHit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "limite superato: {:?}", self.0)
    }
}

impl std::error::Error for LimitHit {}

/// Un `io::Error` che porta il limite superato.
pub(crate) fn limit_error(kind: LimitKind) -> io::Error {
    io::Error::other(LimitHit(kind))
}

/// Classifica un errore di lettura di un flusso d'estrazione: limite superato, dati
/// decompressi non validi o troncati ([`ExtractError::Corrupt`]), altrimenti come
/// [`ExtractError::from_io`].
pub(crate) fn classify_io(error: io::Error) -> ExtractError {
    match hit_or_corrupt(&error) {
        Some(classified) => classified,
        None => ExtractError::from_io(error),
    }
}

/// Come [`classify_io`], per un errore preso in prestito (quick-xml li tiene in un `Arc`).
pub(crate) fn classify_io_ref(error: &io::Error) -> ExtractError {
    match hit_or_corrupt(error) {
        Some(classified) => classified,
        None => ExtractError::from_io(io::Error::new(error.kind(), error.to_string())),
    }
}

fn hit_or_corrupt(error: &io::Error) -> Option<ExtractError> {
    if let Some(hit) = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<LimitHit>())
    {
        return Some(ExtractError::LimitExceeded(hit.0));
    }
    match error.kind() {
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof => Some(ExtractError::Corrupt {
            reason: format!("dati non validi: {:?}", error.kind()),
        }),
        _ => None,
    }
}

/// Riporta a una variante precisa un [`ExtractError::Io`] che porta un limite superato o dati
/// non validi (arrivato attraverso il lettore di testo o una libreria).
pub(crate) fn normalize(error: ExtractError) -> ExtractError {
    match error {
        ExtractError::Io(io) => classify_io(io),
        other => other,
    }
}

/// Rifiuta una voce le cui dimensioni dichiarate superano già i limiti: dimensione per voce,
/// oppure rapporto di compressione (oltre [`RATIO_GRACE_BYTES`]).
pub(crate) fn check_declared(
    size: u64,
    compressed: u64,
    limits: &ArchiveLimits,
) -> Result<(), LimitKind> {
    if size > limits.max_entry_size.bytes() {
        return Err(LimitKind::EntrySize);
    }
    if ratio_exceeded(size, compressed, limits) {
        return Err(LimitKind::CompressionRatio);
    }
    Ok(())
}

fn ratio_exceeded(decompressed: u64, compressed: u64, limits: &ArchiveLimits) -> bool {
    decompressed > RATIO_GRACE_BYTES
        && decompressed > compressed.saturating_mul(u64::from(limits.max_compression_ratio))
}

/// Un flusso decompresso con i limiti del [`Budget`].
pub(crate) struct LimitedReader<R> {
    inner: io::Take<R>,
    read: u64,
    max_entry: u64,
    /// Byte compressi della voce, per il rapporto; `None` se il flusso non è compresso.
    compressed: Option<u64>,
    budget: Rc<Budget>,
}

impl<R: Read> LimitedReader<R> {
    /// Avvolge `inner`; `compressed` sono i byte della voce nell'archivio (limitati dalla
    /// dimensione del contenitore), `None` per un file non compresso.
    pub fn new(inner: R, budget: Rc<Budget>, compressed: Option<u64>) -> LimitedReader<R> {
        let max_entry = budget.limits().max_entry_size.bytes();
        LimitedReader {
            inner: inner.take(max_entry.saturating_add(1)),
            read: 0,
            max_entry,
            compressed,
            budget,
        }
    }
}

impl<R: Read> Read for LimitedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        let counted = u64::try_from(n).unwrap_or(u64::MAX);
        self.read = self.read.saturating_add(counted);
        if self.read > self.max_entry {
            return Err(limit_error(LimitKind::EntrySize));
        }
        if let Some(compressed) = self.compressed
            && ratio_exceeded(self.read, compressed, self.budget.limits())
        {
            return Err(limit_error(LimitKind::CompressionRatio));
        }
        self.budget.add_bytes(counted).map_err(limit_error)?;
        Ok(n)
    }
}

/// Legge tutto `reader` in memoria (i limiti li impone il lettore stesso).
pub(crate) fn read_all(mut reader: impl Read) -> Result<Vec<u8>, ExtractError> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(classify_io)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use lynkeus_core::ByteSize;

    use super::*;

    fn limits(entry: u64, archive: u64) -> ArchiveLimits {
        ArchiveLimits {
            max_entry_size: ByteSize::from_bytes(entry),
            max_archive_size: ByteSize::from_bytes(archive),
            ..ArchiveLimits::default()
        }
    }

    fn read_through(
        data: &[u8],
        budget: &Rc<Budget>,
        compressed: Option<u64>,
    ) -> Result<Vec<u8>, ExtractError> {
        read_all(LimitedReader::new(data, Rc::clone(budget), compressed))
    }

    #[test]
    fn entry_limit_partitions() {
        let budget = Rc::new(Budget::new(limits(10, 1000)));
        // Sotto, sul limite, appena oltre.
        assert_eq!(read_through(&[1; 9], &budget, None).ok(), Some(vec![1; 9]));
        assert_eq!(
            read_through(&[1; 10], &budget, None).ok(),
            Some(vec![1; 10])
        );
        assert!(matches!(
            read_through(&[1; 11], &budget, None),
            Err(ExtractError::LimitExceeded(LimitKind::EntrySize))
        ));
    }

    #[test]
    fn archive_total_spans_entries() {
        let budget = Rc::new(Budget::new(limits(100, 150)));
        assert!(read_through(&[0; 100], &budget, None).is_ok());
        assert!(!budget.exhausted());
        assert!(matches!(
            read_through(&[0; 51], &budget, None),
            Err(ExtractError::LimitExceeded(LimitKind::ArchiveSize))
        ));
        assert!(budget.exhausted());
    }

    #[test]
    fn ratio_applies_only_beyond_grace() {
        let big = u64::MAX / 2;
        let budget = Rc::new(Budget::new(limits(big, big)));
        let grace = usize::try_from(RATIO_GRACE_BYTES).unwrap_or(usize::MAX);
        // Un megabyte da 10 byte compressi: ammesso (sotto la soglia).
        assert!(read_through(&vec![0; grace], &budget, Some(10)).is_ok());
        // Un byte in più: oltre 200:1.
        assert!(matches!(
            read_through(&vec![0; grace + 1], &budget, Some(10)),
            Err(ExtractError::LimitExceeded(LimitKind::CompressionRatio))
        ));
        // Stesso flusso, ma compresso abbastanza poco.
        assert!(read_through(&vec![0; grace + 1], &budget, Some(grace as u64)).is_ok());
    }

    #[test]
    fn declared_partitions() {
        let l = limits(1000, u64::MAX);
        assert_eq!(check_declared(1000, 1000, &l), Ok(()));
        assert_eq!(check_declared(1001, 1001, &l), Err(LimitKind::EntrySize));
        let l = ArchiveLimits::default();
        // bomba.zip: 5 MiB dichiarati in 5110 byte.
        assert_eq!(
            check_declared(5_242_880, 5110, &l),
            Err(LimitKind::CompressionRatio)
        );
        assert_eq!(check_declared(5_242_880, 30_000, &l), Ok(()));
    }

    #[test]
    fn entries_and_depth() {
        let l = ArchiveLimits {
            max_entries: 3,
            max_depth: 1000,
            ..ArchiveLimits::default()
        };
        let budget = Budget::new(l);
        assert_eq!(budget.add_entries(3), Ok(()));
        assert_eq!(budget.add_entries(1), Err(LimitKind::ArchiveEntries));
        assert_eq!(budget.max_depth(), MAX_DEPTH_CEILING);
    }

    #[test]
    fn classification() {
        assert!(matches!(
            classify_io(limit_error(LimitKind::Memory)),
            ExtractError::LimitExceeded(LimitKind::Memory)
        ));
        assert!(matches!(
            classify_io(io::Error::from(io::ErrorKind::InvalidData)),
            ExtractError::Corrupt { .. }
        ));
        assert!(matches!(
            classify_io(io::Error::from(io::ErrorKind::PermissionDenied)),
            ExtractError::AccessDenied
        ));
        assert!(matches!(
            normalize(ExtractError::Io(limit_error(LimitKind::EntrySize))),
            ExtractError::LimitExceeded(LimitKind::EntrySize)
        ));
    }
}
