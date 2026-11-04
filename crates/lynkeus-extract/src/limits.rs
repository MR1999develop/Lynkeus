// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Limiti sulle risorse durante l'estrazione (archivi, anche docx e odt).
//!
//! I byte si contano decompressi, con `Read::take`; i valori predefiniti sono quelli di
//! `.claude/skills/lynkeus-internals/references/limiti.md`.

use lynkeus_core::ByteSize;
use serde::{Deserialize, Serialize};

/// Quale limite è stato superato: diventa il motivo dell'avviso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LimitKind {
    /// Troppi archivi annidati.
    ArchiveDepth,
    /// Troppe voci in un archivio.
    ArchiveEntries,
    /// Una voce decompressa troppo grande.
    EntrySize,
    /// Byte decompressi dell'intero archivio oltre il massimo.
    ArchiveSize,
    /// Rapporto di compressione sospetto (probabile bomba).
    CompressionRatio,
    /// Memoria del processo estrattore esaurita.
    Memory,
}

/// Limiti sugli archivi, modificabili in "Avanzate".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveLimits {
    /// Profondità massima di archivi annidati (un file nella radice è a profondità 0).
    pub max_depth: u32,
    /// Voci massime per archivio.
    pub max_entries: u64,
    /// Byte decompressi massimi per voce.
    pub max_entry_size: ByteSize,
    /// Byte decompressi massimi per archivio, voci annidate comprese.
    pub max_archive_size: ByteSize,
    /// Rapporto massimo tra byte decompressi e compressi di una voce.
    pub max_compression_ratio: u32,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        ArchiveLimits {
            max_depth: 5,
            max_entries: 100_000,
            max_entry_size: ByteSize::from_mib(512),
            max_archive_size: ByteSize::from_gib(4),
            max_compression_ratio: 200,
        }
    }
}
