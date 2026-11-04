// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Differenze tra Windows e Linux nell'app; nessun altro modulo usa `cfg`.
//!
//! Provvisorio (fase 0): solo i font. I font di egui non sono inclusi (licenza di
//! Ubuntu-Light fuori dall'allowlist): si carica il font dell'interfaccia del sistema, e
//! `system_fonts` di eframe copre i caratteri mancanti (CJK e altri alfabeti). La fase 3
//! aggiunge SemiBold e Noto Sans incluso su Linux.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

/// Carica nel contesto il primo font di sistema trovato per il testo e per il monospazio.
///
/// Se non ne trova nessuno non cambia nulla: il testo userà i font di ripiego del sistema
/// forniti da eframe.
pub fn install_fonts(ctx: &egui::Context) {
    let proportional = first_readable(&proportional_candidates());
    let monospace = first_readable(&monospace_candidates());
    let Some(proportional) = proportional else {
        return;
    };
    let mut fonts = FontDefinitions::empty();
    fonts.font_data.insert(
        "ui".to_owned(),
        Arc::new(FontData::from_owned(proportional)),
    );
    let mono_name = match monospace {
        Some(bytes) => {
            fonts
                .font_data
                .insert("mono".to_owned(), Arc::new(FontData::from_owned(bytes)));
            "mono"
        }
        None => "ui",
    };
    fonts
        .families
        .insert(FontFamily::Proportional, vec!["ui".to_owned()]);
    fonts.families.insert(
        FontFamily::Monospace,
        vec![mono_name.to_owned(), "ui".to_owned()],
    );
    ctx.set_fonts(fonts);
}

/// Il contenuto del primo file leggibile tra `candidates`.
fn first_readable(candidates: &[PathBuf]) -> Option<Vec<u8>> {
    candidates.iter().find_map(|path| std::fs::read(path).ok())
}

#[cfg(windows)]
fn fonts_dir() -> PathBuf {
    std::env::var_os("WINDIR")
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join("Fonts")
}

#[cfg(windows)]
fn proportional_candidates() -> Vec<PathBuf> {
    vec![fonts_dir().join("segoeui.ttf")]
}

#[cfg(windows)]
fn monospace_candidates() -> Vec<PathBuf> {
    vec![fonts_dir().join("consola.ttf")]
}

#[cfg(not(windows))]
fn proportional_candidates() -> Vec<PathBuf> {
    [
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/opentype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    ]
    .map(PathBuf::from)
    .to_vec()
}

#[cfg(not(windows))]
fn monospace_candidates() -> Vec<PathBuf> {
    [
        "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
    ]
    .map(PathBuf::from)
    .to_vec()
}
