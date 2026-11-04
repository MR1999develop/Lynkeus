// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Dimensioni in byte, per limiti e filtri.

use serde::{Deserialize, Serialize};

/// Una quantità di byte.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct ByteSize(u64);

impl ByteSize {
    /// `bytes` byte.
    pub const fn from_bytes(bytes: u64) -> ByteSize {
        ByteSize(bytes)
    }

    /// `mib` mebibyte (1 MiB = 1 048 576 byte), saturati a `u64::MAX`.
    pub const fn from_mib(mib: u64) -> ByteSize {
        ByteSize(mib.saturating_mul(1024 * 1024))
    }

    /// `gib` gibibyte, saturati a `u64::MAX`.
    pub const fn from_gib(gib: u64) -> ByteSize {
        ByteSize(gib.saturating_mul(1024 * 1024 * 1024))
    }

    /// Il numero di byte.
    pub const fn bytes(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_and_saturation() {
        assert_eq!(ByteSize::from_mib(200).bytes(), 209_715_200);
        assert_eq!(ByteSize::from_gib(4).bytes(), 4_294_967_296);
        assert_eq!(ByteSize::from_gib(u64::MAX).bytes(), u64::MAX);
    }
}
