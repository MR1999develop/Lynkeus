# Dipendenze

Ogni dipendenza diretta di un `Cargo.toml` ha una validazione registrata in questa tabella: versione, licenza (compatibile con GPLv3), motivo della scelta, crate che la usano, uso di `unsafe`, presenza di codice C, data di verifica ed esito. Si aggiorna a ogni aggiunta o cambio di versione, insieme a `cargo deny check`.

I crate interni del workspace (`lynkeus-core`, `lynkeus-extract`, `lynkeus-engine`) non si registrano qui: non sono dipendenze esterne.

## Tabella delle dipendenze dirette

Versioni risolte da `Cargo.lock`. Licenze da `cargo info`. Conteggio `unsafe`: occorrenze della parola `unsafe` nel sorgente `src/` del crate scaricato, commenti inclusi, quindi indicativo e non un conteggio dei blocchi.

| crate | versione | licenza | motivo | usata da | unsafe | codice C | verificata il | esito |
|---|---|---|---|---|---|---|---|---|
| anyhow | 1.0.104 | MIT OR Apache-2.0 | Errori nel binario e in xtask, con contesto e catena di cause. | lynkeus-app, xtask | sì (104 in `src`: `error`, `ptr`, `context`, `fmt`, `ensure`) | no (il `build.rs` fa solo probe del compilatore) | 2026-11-04 | ok |
| eframe | 0.37.0 | MIT OR Apache-2.0 | Finestra e ciclo degli eventi dell'interfaccia. Senza `default_fonts` per la licenza Ubuntu Font: i font si caricano dal sistema. | lynkeus-app | sì (26 in `src`) | no diretto; su Linux indiretto (vedi sezione Codice C) | 2026-11-04 | ok |
| egui | 0.37.0 | MIT OR Apache-2.0 | Widget e disegno dell'interfaccia. Senza `default_fonts`, come eframe. | lynkeus-app | sì (4 in `src`) | no | 2026-11-04 | ok |
| fluent-templates | 0.15.1 | MIT/Apache-2.0 | Traduzioni Fluent in `locales/it` e `locales/en`. | lynkeus-app | no (0) | no | 2026-11-04 | ok |
| ignore | 0.4.34 | Unlicense OR MIT | Scansione ricorsiva delle cartelle (`WalkBuilder`). | lynkeus-engine | no (0) | no | 2026-11-04 | ok |
| regex | 1.13.1 | MIT OR Apache-2.0 | Ricerca lineare dei termini. | lynkeus-core | sì (1 in `src`) | no | 2026-11-04 | ok |
| serde | 1.0.229 | MIT OR Apache-2.0 | Tipi del protocollo tra processi e impostazioni (con `derive`). | lynkeus-core, lynkeus-extract, lynkeus-engine, lynkeus-app | sì (2 in `src`) | no (il `build.rs` fa solo probe) | 2026-11-04 | ok |
| sys-locale | 0.3.2 | MIT OR Apache-2.0 | Lingua di sistema, per scegliere l'interfaccia in italiano o inglese. | lynkeus-app | sì (7 in `src`, chiamate di sistema Windows, Apple, Android) | no | 2026-11-04 | ok (nota: ultima release 2024-11-01, repository attivo) |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | Errori tipizzati delle librerie. | lynkeus-core, lynkeus-extract, lynkeus-engine, lynkeus-app | no (0) | no (il `build.rs` fa solo probe) | 2026-11-04 | ok (nota: la 1.0.69 entra in transitivo tramite calloop, warning di duplicato) |
| winit | 0.30.13 | Apache-2.0 | Solo Linux. Non è importata nel codice: serve la feature Wayland caricata a runtime e le decorazioni su GNOME senza titolo (`-notitle`), che evitano la catena ab_glyph → ttf-parser (RUSTSEC-2026-0192; verificato che `ttf-parser` non è nell'albero). | lynkeus-app (solo Linux) | sì (932 in `src`, FFI delle piattaforme) | no diretto; su Linux indiretto (vedi sezione Codice C) | 2026-11-04 | ok |
| sha2 | 0.11.1 | MIT OR Apache-2.0 | Calcolo dei SHA-256 per `SHA256SUMS` nel rilascio (`cargo xtask dist`). | xtask | sì (39 in `src`) | no | 2026-11-04 | ok |
| zip | 9.3.0 | MIT | Creazione dello zip di rilascio (solo `deflate`, senza feature di default). | xtask | sì (48 in `src`) | no | 2026-11-04 | ok |

## Dettagli di verifica

Manutenzione dal repository GitHub (`pushed_at`, `archived`) e dall'ultimo aggiornamento su crates.io, entrambi verificati il 2026-11-04.

| crate | ultima versione stabile | ultimo aggiornamento crates.io | ultimo push del repository | archiviato | note |
|---|---|---|---|---|---|
| anyhow | 1.0.104 | 2026-07-18 | 2026-11-03 | no | |
| eframe | 0.37.0 | 2026-10-15 | 2026-11-02 (repository egui) | no | |
| egui | 0.37.0 | 2026-10-15 | 2026-11-02 | no | |
| fluent-templates | 0.15.1 | 2026-08-05 | 2026-08-05 | no | |
| ignore | 0.4.34 | 2026-10-27 | 2026-10-27 | no | |
| regex | 1.13.1 | 2026-07-15 | 2026-10-28 | no | |
| serde | 1.0.229 | 2026-07-18 | 2026-10-16 | no | |
| sys-locale | 0.3.2 | 2024-11-01 | 2026-11-04 | no | nessuna release da oltre due anni: da tenere d'occhio |
| thiserror | 2.0.21 | 2026-09-23 | 2026-10-16 | no | |
| winit | 0.30.13 | 2026-10-24 | 2026-11-03 | no | |
| sha2 | 0.11.1 | 2026-10-12 | 2026-11-03 | no | |
| zip | 9.3.0 | 2026-11-03 | 2026-11-03 | no | |

## Feature attive

Feature risolte con `cargo tree -p <crate> --depth 0 -f "{p} | {f}"`.

| crate | feature attive |
|---|---|
| anyhow | default, std |
| eframe | accesskit, clipboard, system_fonts, wayland, wgpu, wgpu_no_default_features, x11 |
| egui | bytemuck (senza `default_fonts`, `default-features = false`) |
| fluent-templates | default, fluent-template-macros, ignore, macros |
| ignore | nessuna |
| regex | default e tutte le feature `perf-*` e `unicode-*` che ne derivano |
| serde | default, derive, serde_derive, std |
| sys-locale | nessuna |
| thiserror | default, std |
| winit | rwh_06, wayland-dlopen e wayland-csd-adwaita-notitle dal workspace (`default-features = false`); risolte anche ahash, bytemuck, memmap2, percent-encoding, sctk, wayland, wayland-backend, wayland-client, wayland-protocols, wayland-protocols-plasma, x11, x11-dl, x11rb |
| sha2 | alloc, default, oid |
| zip | deflate, deflate-flate2, deflate-flate2-zlib-rs, deflate-zopfli, _deflate-any |

## Codice C nell'albero

Verificato sull'albero di Windows (`x86_64-pc-windows-msvc`) e su quello di Linux (`x86_64-unknown-linux-gnu`).

- Windows: nessun crate che compili C. I crate `-sys` presenti (`windows-sys`, `renderdoc-sys`) sono binding Rust senza compilazione di C.
- Linux, `eframe` → `egui_system_font_provider` → `fontique` → `yeslogic-fontconfig-sys`: richiede la libreria di sistema `libfontconfig`, cercata con pkg-config e collegata in fase di build.
- Linux, `winit` → `wayland-backend` (build script con `cc`): compila lo shim C `log_shim.c`, lato client e server.
- Linux, `winit` → `wayland-sys` (build script con pkg-config). La feature `wayland-dlopen` carica la libreria Wayland a runtime; non ho verificato se con questa feature il build script di `wayland-sys` cerchi comunque la libreria con pkg-config.

## Eccezioni agli advisory (`deny.toml`)

Tutte e tre sono advisory di tipo "unmaintained" (nessuna vulnerabilità nota) e arrivano su Linux da AccessKit, usato da eframe 0.37 tramite accesskit_unix 0.24.1 (ultima versione) e zbus 5. Da rivedere quando AccessKit passa a zbus 6 o a ogni aggiornamento di eframe.

- **RUSTSEC-2026-0370**: `zvariant` non mantenuto. Arriva da AccessKit su Linux senza alternative.
- **RUSTSEC-2026-0371**: `zbus_names` non mantenuto. Arriva da AccessKit su Linux senza alternative.
- **RUSTSEC-2026-0372**: `zvariant_derive` non mantenuto. Arriva da AccessKit su Linux senza alternative.

Nessun'altra eccezione: `exceptions = []` per le licenze. Il rischio RUSTSEC-2026-0192 (`ttf-parser`) non è ignorato in `deny.toml` perché non compare nell'albero: si evita con la feature `-notitle` di winit.

## Ultimo `cargo deny check`

Eseguito il 2026-11-04: exit code 0. Esito per sezione: advisories ok, bans ok, licenses ok, sources ok.

Warning (non bloccanti, `multiple-versions = "warn"`): versioni duplicate di calloop, calloop-wayland-source, foldhash, getrandom, linux-raw-sys, miniz_oxide, rustc-hash, rustix, smithay-client-toolkit, syn, thiserror (1.0.69 e 2.0.21), thiserror-impl, hashbrown (3 versioni) e windows-sys (3 versioni).
