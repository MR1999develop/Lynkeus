// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Differenze tra Windows e Linux nell'app; nessun altro modulo usa `cfg`.
//!
//! - Font: i font di egui non sono inclusi (licenza di Ubuntu-Light fuori dall'allowlist).
//!   Su Windows Segoe UI Regular e Semibold da `C:\Windows\Fonts` (mai inclusi nel
//!   pacchetto); su Linux Noto Sans o DejaVu Sans se installati. `system_fonts` di eframe
//!   copre i caratteri mancanti (CJK e altri alfabeti).
//! - Cartella di configurazione (`settings.toml` e posizione della finestra).
//! - Flush della cartella dopo una rinomina durevole.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

use crate::theme::SEMIBOLD;

/// Nome della cartella dell'app sotto la cartella di configurazione dell'utente.
#[cfg(windows)]
const APP_DIR_NAME: &str = "Lynkeus Searcher";
#[cfg(not(windows))]
const APP_DIR_NAME: &str = "lynkeus-searcher";

/// La cartella di configurazione dell'app: `%APPDATA%\Lynkeus Searcher\config` su
/// Windows, `~/.config/lynkeus-searcher` su Linux. `None` se il sistema non ne indica una
/// (nessuna cartella utente): le impostazioni restano allora solo in memoria.
pub fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", APP_DIR_NAME).map(|dirs| dirs.config_dir().to_owned())
}

/// Rende durevole la rinomina appena fatta in `dir`.
///
/// Linux: `fsync` della cartella, senza il quale la rinomina può andare persa in un crash.
/// Windows: nulla; NTFS registra la rinomina nel suo journal e `FlushFileBuffers` su una
/// cartella chiederebbe permessi di backup.
#[cfg(not(windows))]
pub fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

/// Vedi la versione per Linux.
#[cfg(windows)]
pub fn sync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Carica nel contesto i font dell'interfaccia: Regular per il testo, SemiBold per titoli
/// ed evidenziati (famiglia [`SEMIBOLD`]), un monospazio se presente.
///
/// La famiglia SemiBold è sempre definita: senza il file Semibold ricade sul Regular, così
/// nessun testo resta senza font. Se non trova nemmeno il Regular lascia i font di ripiego
/// del sistema forniti da eframe e definisce comunque la famiglia SemiBold come quella
/// proporzionale.
pub fn install_fonts(ctx: &egui::Context) {
    let Some(regular) = first_readable(&regular_candidates()) else {
        let mut fonts = FontDefinitions::default();
        let proportional = fonts
            .families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default();
        fonts
            .families
            .insert(FontFamily::Name(SEMIBOLD.into()), proportional);
        ctx.set_fonts(fonts);
        return;
    };
    let semibold = first_readable(&semibold_candidates());
    let monospace = first_readable(&monospace_candidates());

    let mut fonts = FontDefinitions::empty();
    fonts
        .font_data
        .insert("ui".to_owned(), Arc::new(FontData::from_owned(regular)));
    let proportional = vec!["ui".to_owned()];
    let mut strong = Vec::new();
    if let Some(bytes) = semibold {
        fonts.font_data.insert(
            "ui-semibold".to_owned(),
            Arc::new(FontData::from_owned(bytes)),
        );
        strong.push("ui-semibold".to_owned());
    }
    strong.extend(proportional.iter().cloned());
    let mut mono = Vec::new();
    if let Some(bytes) = monospace {
        fonts
            .font_data
            .insert("mono".to_owned(), Arc::new(FontData::from_owned(bytes)));
        mono.push("mono".to_owned());
    }
    mono.extend(proportional.iter().cloned());

    fonts
        .families
        .insert(FontFamily::Proportional, proportional);
    fonts.families.insert(FontFamily::Monospace, mono);
    fonts
        .families
        .insert(FontFamily::Name(SEMIBOLD.into()), strong);
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
fn regular_candidates() -> Vec<PathBuf> {
    vec![fonts_dir().join("segoeui.ttf")]
}

#[cfg(windows)]
fn semibold_candidates() -> Vec<PathBuf> {
    vec![fonts_dir().join("seguisb.ttf")]
}

#[cfg(windows)]
fn monospace_candidates() -> Vec<PathBuf> {
    vec![fonts_dir().join("consola.ttf")]
}

#[cfg(not(windows))]
fn regular_candidates() -> Vec<PathBuf> {
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
fn semibold_candidates() -> Vec<PathBuf> {
    [
        "/usr/share/fonts/truetype/noto/NotoSans-SemiBold.ttf",
        "/usr/share/fonts/opentype/noto/NotoSans-SemiBold.ttf",
        "/usr/share/fonts/noto/NotoSans-SemiBold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
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
