// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Risorse dell'eseguibile Windows (solo per il target Windows; altrove nulla): icona
//! (`assets/icon.ico`, 16-256 px), informazioni di versione (versione da `Cargo.toml`) e
//! manifest (`windows/lynkeus-searcher.manifest`: `asInvoker`, `longPathAware`, Windows
//! 10/11, DPI PerMonitorV2). Compilate con `rc.exe` del Windows SDK e collegate solo al
//! binario `lynkeus-searcher`.

use std::io;
use std::path::PathBuf;

const ICON: &str = "../../assets/icon.ico";
const MANIFEST: &str = "windows/lynkeus-searcher.manifest";
const BINARY: &str = "lynkeus-searcher";

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-changed={MANIFEST}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }
    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR")
            .ok_or_else(|| io::Error::other("CARGO_MANIFEST_DIR assente"))?,
    );
    let text = |relative: &str| -> io::Result<String> {
        manifest_dir
            .join(relative)
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| io::Error::other("percorso non UTF-8"))
    };
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon(&text(ICON)?)
        .set_manifest_file(&text(MANIFEST)?)
        .set("ProductName", "Lynkeus Searcher")
        .set("FileDescription", "Lynkeus Searcher")
        .set("InternalName", BINARY)
        .set("OriginalFilename", &format!("{BINARY}.exe"))
        .set("LegalCopyright", "Copyright (C) 2026 MR")
        .set("Comments", "GPL-3.0-only");
    resource.compile_for(BINARY)
}
