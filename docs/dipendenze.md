# Dipendenze

Ogni dipendenza diretta di un `Cargo.toml` ha una validazione registrata in questa tabella: versione, licenza (compatibile con GPLv3), motivo della scelta, crate che la usano, uso di `unsafe`, presenza di codice C, data di verifica ed esito. Si aggiorna a ogni aggiunta o cambio di versione, insieme a `cargo deny check`.

I crate interni del workspace (`lynkeus-core`, `lynkeus-extract`, `lynkeus-engine`) non si registrano qui: non sono dipendenze esterne.

## Tabella delle dipendenze dirette

Versioni risolte da `Cargo.lock`. Licenze da `cargo info`. Conteggio `unsafe`: occorrenze della parola `unsafe` nel sorgente `src/` del crate scaricato, commenti inclusi, quindi indicativo e non un conteggio dei blocchi. Le dipendenze di sviluppo (`dev-dependencies`) sono marcate con "(dev)".

| crate | versione | licenza | motivo | usata da | unsafe | codice C | verificata il | esito |
|---|---|---|---|---|---|---|---|---|
| anyhow | 1.0.104 | MIT OR Apache-2.0 | Errori nel binario e in xtask, con contesto e catena di cause. | lynkeus-app, xtask | sì (104 in `src`: `error`, `ptr`, `context`, `fmt`, `ensure`) | no (il `build.rs` fa solo probe del compilatore) | 2026-11-04 | ok |
| chardetng | 1.0.0 | Apache-2.0 OR MIT | Rilevamento della codifica dei file a 8 bit (non UTF-8, non BOM) in `text/detect.rs`, sui primi byte del file, con ripiego su Windows-1252 se la proposta non è adatta. | lynkeus-extract | sì (2 in `src`) | no | 2026-11-04 | ok |
| criterion | 0.8.2 | Apache-2.0 OR MIT | Benchmark di `find_words` su circa 4 MB di testo (`benches/find_words.rs`). Solo dev-dependency, senza grafici né rayon. | lynkeus-core (dev) | sì (7 in `src`) | sì solo in dev: `criterion` → `alloca` → `cc` compila `alloca.c` (vedi sezione Codice C) | 2026-11-04 | ok (nota: codice C in fase di build dei test e dei benchmark, non nel binario) |
| crossbeam-channel | 0.5.17 | MIT OR Apache-2.0 | Canale limitato (`bounded`) tra il worker di ricerca e l'interfaccia: eventi con capacità fissa e invio con timeout (`send_timeout`), `try_recv` per svuotarlo nel ciclo dell'interfaccia (`handle.rs`, `worker.rs`). | lynkeus-engine | sì (66 in `src`) | no | 2026-11-04 | ok |
| directories | 6.0.0 | MIT OR Apache-2.0 | Cartella di configurazione per piattaforma (`settings.toml` e posizione della finestra): `%APPDATA%\Lynkeus Searcher\config` su Windows, `~/.config/lynkeus-searcher` su Linux (`platform::config_dir`). | lynkeus-app | no (0) | no | 2026-11-04 | ok (nota: repository archiviato, ultima release 2025-01-12; vedi Dettagli di verifica) |
| eframe | 0.37.0 | MIT OR Apache-2.0 | Finestra e ciclo degli eventi dell'interfaccia. Senza `default_fonts` per la licenza Ubuntu Font: i font si caricano dal sistema. Feature `persistence` solo per ricordare la finestra (`persist_window` e `persistence_path` in `main.rs`): le impostazioni vanno in `settings.toml`. | lynkeus-app | sì (26 in `src`) | no diretto; su Linux indiretto (vedi sezione Codice C) | 2026-11-04 | ok |
| egui | 0.37.0 | MIT OR Apache-2.0 | Widget e disegno dell'interfaccia. Senza `default_fonts`, come eframe. | lynkeus-app | sì (4 in `src`) | no | 2026-11-04 | ok |
| egui_extras | 0.37.0 | MIT OR Apache-2.0 | Tabella virtualizzata dei risultati (`TableBuilder`): si disegnano solo le righe visibili (`ui/table.rs`). Senza feature. | lynkeus-app | sì (2 in `src`) | no | 2026-11-04 | ok |
| egui_kittest | 0.37.0 | MIT OR Apache-2.0 | Test dell'interfaccia (`tests/interfaccia.rs`): l'app `eframe::App` vera, guidata per ruolo e nome accessibile. Feature `eframe` attiva. Solo dev-dependency. | lynkeus-app (dev) | sì (2 in `src`) | no | 2026-11-04 | ok |
| encoding_rs | 0.8.44 | (Apache-2.0 OR MIT) AND BSD-3-Clause | Decodifica a blocchi dei file di testo (`Decoder`, tabelle WHATWG) in UTF-8: UTF-16 e le codifiche rilevate, come Windows-1252 (`text/mod.rs`, `text/detect.rs`). | lynkeus-extract | sì (193 in `src`) | no (Rust puro; `build.rs` solo imposta un cfg) | 2026-11-04 | ok |
| fluent-templates | 0.15.1 | MIT/Apache-2.0 | Traduzioni Fluent in `locales/it` e `locales/en`. | lynkeus-app | no (0) | no | 2026-11-04 | ok |
| ignore | 0.4.34 | Unlicense OR MIT | Scansione ricorsiva delle cartelle (`WalkBuilder`). | lynkeus-engine | no (0) | no | 2026-11-04 | ok |
| memchr | 2.8.3 | Unlicense OR MIT | Ricerca veloce del byte `\n` e `\0`: conteggio delle righe e ricerca del punto di taglio dei blocchi di testo (`textfile.rs`, `text/mod.rs`, `text/detect.rs`). | lynkeus-extract, lynkeus-engine | sì (340 in `src`, SIMD) | no | 2026-11-04 | ok (nota: molto `unsafe` per la ricerca SIMD, crate molto usato) |
| opener | 0.9.0 | MIT OR Apache-2.0 | Solo Windows. Apre un file con il programma predefinito (`opener::open`, `ShellExecuteW`) e lo mostra selezionato in Esplora risorse (`opener::reveal`, `SHOpenFolderAndSelectItems`), in `platform.rs`. Feature `reveal`, senza default. Su Linux non si usa: lì c'è il portale XDG con `zbus`. | lynkeus-app (solo Windows) | sì (17 in `src`, FFI di Shell e COM nei moduli `windows`) | no (`build = false`; `windows-sys` è solo binding) | 2026-11-04 | ok |
| proptest | 1.12.0 | MIT OR Apache-2.0 | Test di proprietà di `find_words`, `word_text` e `snippet` (`lynkeus-core/tests/proprieta.rs`), dei lettori a blocchi (`text/mod.rs`, `textfile.rs`) e dell'esportazione CSV (`export.rs`). Solo dev-dependency. | lynkeus-core (dev), lynkeus-extract (dev), lynkeus-engine (dev), lynkeus-app (dev) | sì (8 in `src`) | no | 2026-11-04 | ok |
| regex | 1.13.1 | MIT OR Apache-2.0 | Ricerca lineare dei termini; tabelle Unicode delle categorie `Mn` e `P` (`unicode.rs`). | lynkeus-core | sì (1 in `src`) | no | 2026-11-04 | ok |
| rfd | 0.17.2 | MIT | Dialoghi nativi per scegliere cartelle e file (`FileDialog`, `ui/mod.rs`). Su Linux tramite il portale XDG, senza libreria GTK; su Windows API di sistema. | lynkeus-app | sì (249 in `src`, FFI delle piattaforme) | no (il `build.rs` fa solo controlli di feature e cfg) | 2026-11-04 | ok |
| serde | 1.0.229 | MIT OR Apache-2.0 | Tipi del protocollo tra processi e impostazioni (con `derive`). | lynkeus-core, lynkeus-extract, lynkeus-engine, lynkeus-app | sì (2 in `src`) | no (il `build.rs` fa solo probe) | 2026-11-04 | ok |
| sys-locale | 0.3.2 | MIT OR Apache-2.0 | Lingua di sistema, per scegliere l'interfaccia in italiano o inglese. | lynkeus-app | sì (7 in `src`, chiamate di sistema Windows, Apple, Android) | no | 2026-11-04 | ok (nota: ultima release 2024-11-01, repository attivo) |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | Cartelle temporanee per i test di scansione (`engine/tests/scansione.rs`) e delle impostazioni (`settings.rs`). Solo dev-dependency. | lynkeus-engine (dev), lynkeus-app (dev) | sì (4 in `src`) | no | 2026-11-04 | ok |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | Errori tipizzati delle librerie. | lynkeus-core, lynkeus-extract, lynkeus-engine, lynkeus-app | no (0) | no (il `build.rs` fa solo probe) | 2026-11-04 | ok (nota: la 1.0.69 entra in transitivo tramite calloop, warning di duplicato) |
| toml | 1.2.0 (richiesta 1.1.8) | MIT OR Apache-2.0 | Lettura e scrittura di `settings.toml` (`Table`, `Value`, `to_string`) in `settings.rs`. Solo `parse`, `display`, `serde`. | lynkeus-app | sì (3 in `src`) | no | 2026-11-04 | ok |
| unicode-normalization | 0.1.26 | (MIT OR Apache-2.0) AND Unicode-3.0 | Normalizzazione NFC/NFD dei termini (`AccentMode`) e decomposizione canonica per la tabella delle varianti accentate (`unicode.rs`, `pattern.rs`). | lynkeus-core (anche in test `proprieta.rs`, dev) | sì (5 in `src`) | no | 2026-11-04 | ok |
| winit | 0.30.13 | Apache-2.0 | Solo Linux. Non è importata nel codice: serve la feature Wayland caricata a runtime e le decorazioni su GNOME senza titolo (`-notitle`), che evitano la catena ab_glyph → ttf-parser (RUSTSEC-2026-0192; verificato che `ttf-parser` non è nell'albero). | lynkeus-app (solo Linux) | sì (932 in `src`, FFI delle piattaforme) | no diretto; su Linux indiretto (vedi sezione Codice C) | 2026-11-04 | ok |
| sha2 | 0.11.1 | MIT OR Apache-2.0 | Calcolo dei SHA-256 per `SHA256SUMS` nel rilascio (`cargo xtask dist`). | xtask | sì (39 in `src`) | no | 2026-11-04 | ok |
| zbus | 5.19.0 | MIT | Solo non Windows (`cfg(not(windows))`; nei target del progetto è Linux): portale XDG `org.freedesktop.portal.OpenURI`, metodi `OpenFile` e `OpenDirectory` con il file come descrittore, in `platform.rs` (`call_open_uri_portal`). Feature `async-io` e `blocking-api`, senza default. Resta la 5 perché AccessKit la porta già nell'albero: la 6 ne aggiungerebbe una seconda copia. | lynkeus-app (non Windows) | sì (58 in `src`, socket e FFI) | no (`build = false`, nessun `build.rs`) | 2026-11-04 | ok (nota: la 6.2.1 è disponibile, vedi Dettagli di verifica) |
| zip | 9.3.0 | MIT | Creazione dello zip di rilascio (solo `deflate`, senza feature di default). | xtask | sì (48 in `src`) | no | 2026-11-04 | ok |

Nota sul `Cargo.toml` del workspace: `toml` è richiesto come `1.1.8` ma `Cargo.lock` risolve `1.2.0`, compatibile in semver.

Crate transitivi nuovi di `opener` (solo Windows): 24, oltre a `opener` stesso. Sono in `Cargo.lock` dalla fase 6 e non hanno una riga propria in tabella: `form_urlencoded`, `icu_collections`, `icu_locale_core`, `icu_normalizer`, `icu_normalizer_data`, `icu_properties`, `icu_properties_data`, `icu_provider`, `idna`, `idna_adapter`, `litemap`, `normpath`, `potential_utf`, `stable_deref_trait`, `synstructure`, `url`, `utf8_iter`, `utf_types`, `writeable`, `yoke`, `yoke-derive`, `zerofrom-derive`, `zerotrie`, `zerovec-derive`. Vengono da `url` 2.5.8 e `normpath` 1.5.2, dipendenze di `opener` su Windows, e dalla catena ICU di `idna`. Il confronto è con il `Cargo.lock` di `a74f053` (prima della fase 6). Licenze e advisory di questi crate sono coperti da `cargo deny check`, non verificati uno per uno. `zbus` non aggiunge pacchetti nuovi al lock: era già presente tramite AccessKit, e la fase 6 lo dichiara solo come dipendenza diretta di `lynkeus-app` su non Windows.

## Dettagli di verifica

Manutenzione dal repository GitHub (`pushed_at`, `archived`) e dall'ultimo aggiornamento su crates.io, entrambi verificati il 2026-11-04. Per tutti i crate la versione risolta coincide con l'ultima stabile su crates.io, tranne `zbus` (risolta 5.19.0, ultima 6.2.1).

| crate | ultima versione stabile | ultimo aggiornamento crates.io | ultimo push del repository | archiviato | note |
|---|---|---|---|---|---|
| anyhow | 1.0.104 | 2026-07-18 | 2026-11-03 | no | |
| chardetng | 1.0.0 | 2026-03-30 | 2026-03-30 | no | unica release stabile, nessun aggiornamento da allora |
| criterion | 0.8.2 | 2026-02-04 | 2026-10-13 | no | |
| crossbeam-channel | 0.5.17 | 2026-09-05 | 2026-11-01 (repository crossbeam) | no | |
| directories | 6.0.0 | 2025-01-12 | 2025-01-12 | **sì** | repository archiviato su GitHub: nessuna manutenzione né correzioni future. `cargo deny` non segnala advisory, ma è un crate da tenere d'occhio o da sostituire se serve |
| eframe | 0.37.0 | 2026-10-15 | 2026-11-02 (repository egui) | no | `persistence` aggiunge `ron` 0.12.3 (MIT OR Apache-2.0), transitiva e non registrata qui |
| egui | 0.37.0 | 2026-10-15 | 2026-11-02 | no | |
| egui_extras | 0.37.0 | 2026-10-15 | 2026-11-02 (repository egui) | no | |
| egui_kittest | 0.37.0 | 2026-10-15 | 2026-11-02 (repository egui) | no | |
| encoding_rs | 0.8.44 | 2026-10-30 | 2026-10-30 | no | |
| fluent-templates | 0.15.1 | 2026-08-05 | 2026-08-05 | no | |
| ignore | 0.4.34 | 2026-10-27 | 2026-10-27 | no | |
| memchr | 2.8.3 | 2026-07-08 | 2026-08-10 | no | |
| opener | 0.9.0 | 2026-09-29 | 2026-11-01 | no | pubblicata il 2026-09-29. La feature `reveal` su Linux porterebbe `zbus`, ma `opener` è dichiarato solo per Windows |
| proptest | 1.12.0 | 2026-10-10 | 2026-10-10 | no | |
| regex | 1.13.1 | 2026-07-15 | 2026-10-28 | no | |
| rfd | 0.17.2 | 2026-10-22 | 2026-10-25 | no | |
| serde | 1.0.229 | 2026-07-18 | 2026-10-16 | no | |
| sys-locale | 0.3.2 | 2024-11-01 | 2026-11-04 | no | nessuna release da oltre due anni: da tenere d'occhio |
| tempfile | 3.27.0 | 2026-03-11 | 2026-10-30 | no | |
| thiserror | 2.0.21 | 2026-09-23 | 2026-10-16 | no | |
| toml | 1.2.0 | 2026-11-04 | 2026-11-04 | no | |
| unicode-normalization | 0.1.26 | 2026-10-16 | 2026-10-29 | no | |
| winit | 0.30.13 | 2026-10-24 | 2026-11-03 | no | |
| sha2 | 0.11.1 | 2026-10-12 | 2026-11-03 | no | |
| zbus | 5.19.0 (ultima 6.2.1, pubblicata il 2026-11-01) | 2026-08-09 (5.19.0) | 2026-11-03 | no | repository `z-galaxy/zbus` attivo. GitHub non riconosce la licenza (`NOASSERTION`), ma il file `LICENSE` è MIT e `cargo info` riporta MIT |
| zip | 9.3.0 | 2026-11-03 | 2026-11-03 | no | |

## Feature attive

Feature risolte con `cargo tree -p <crate> --depth 0 -f "{p} | {f}"`.

| crate | feature attive |
|---|---|
| anyhow | default, std |
| chardetng | nessuna |
| criterion | cargo_bench_support (`default-features = false`) |
| crossbeam-channel | default, std |
| directories | nessuna |
| eframe | accesskit, clipboard, persistence, ron, serde, system_fonts, wayland, wgpu, wgpu_no_default_features, x11 (`persistence` porta `ron` e `serde`) |
| egui | bytemuck (senza `default_fonts`, `default-features = false`) |
| egui_extras | nessuna (`default-features = false`) |
| egui_kittest | eframe (`default-features = false`) |
| encoding_rs | alloc, default |
| fluent-templates | default, fluent-template-macros, ignore, macros |
| ignore | nessuna |
| memchr | alloc, default, std |
| opener | reveal (`default-features = false`) |
| proptest | bit-set, default, fork, regex-syntax, rusty-fork, std, tempfile, timeout |
| regex | default e tutte le feature `perf-*` e `unicode-*` che ne derivano |
| rfd | pollster, wayland, wayland-backend, wayland-client, wayland-protocols, xdg-portal (`default-features = false`) |
| serde | default, derive, serde_derive, std |
| sys-locale | nessuna |
| tempfile | default, getrandom |
| thiserror | default, std |
| toml | display, parse, serde (`default-features = false`) |
| unicode-normalization | default, std |
| winit | rwh_06, wayland-dlopen e wayland-csd-adwaita-notitle dal workspace (`default-features = false`); risolte anche ahash, bytemuck, memmap2, percent-encoding, sctk, wayland, wayland-backend, wayland-client, wayland-protocols, wayland-protocols-plasma, x11, x11-dl, x11rb |
| sha2 | alloc, default, oid |
| zbus | async-executor, async-fs, async-io, async-lock, async-process, async-task, blocking, blocking-api (`default-features = false`; nel workspace `async-io` e `blocking-api`, il resto arriva da `async-io`) |
| zip | deflate, deflate-flate2, deflate-flate2-zlib-rs, deflate-zopfli, _deflate-any |

## Codice C nell'albero

Verificato sull'albero di Windows (`x86_64-pc-windows-msvc`) e su quello di Linux (`x86_64-unknown-linux-gnu`), includendo le dipendenze di sviluppo.

- Windows, solo dev: `criterion` (dev-dependency di `lynkeus-core`) → `alloca` 0.4.0 (build script che compila `alloca.c` con `cc`). Il C si compila a ogni `cargo test` e `cargo bench` che coinvolge `lynkeus-core`, non entra nel binario. Gli altri crate `-sys` presenti (`windows-sys`, `renderdoc-sys`) sono binding Rust senza compilazione di C.
- Linux, solo dev: stesso percorso `criterion` → `alloca` → `cc`.
- Linux, `eframe` → `egui_system_font_provider` → `fontique` → `yeslogic-fontconfig-sys`: richiede la libreria di sistema `libfontconfig`, cercata con pkg-config e collegata in fase di build.
- Linux, `winit` → `wayland-backend` (build script con `cc`): compila lo shim C `log_shim.c`, lato client e server. `rfd` con `wayland` aggiunge `wayland-backend` e `wayland-client`, già presenti per winit: nessun C in più.
- Linux, `winit` → `wayland-sys` (build script con pkg-config). La feature `wayland-dlopen` carica la libreria Wayland a runtime; non ho verificato se con questa feature il build script di `wayland-sys` cerchi comunque la libreria con pkg-config.
- Windows, `opener` 0.9.0 (`reveal`): nessun C. `opener` ha `build = false`, `windows-sys` è solo binding. Il ramo Linux di `opener` (con `zbus`, opzionale) non si attiva su Windows.
- Linux, `zbus` 5.19.0: nessun C (`build = false`, nessun `build.rs`).

I crate nuovi di codice Rust puro (`encoding_rs`, `memchr`, `crossbeam-channel`, `chardetng`, `rfd` lato build script) non compilano C.

## Eccezioni agli advisory (`deny.toml`)

Tutte e tre sono advisory di tipo "unmaintained" (nessuna vulnerabilità nota) e arrivano su Linux da zbus 5. Due percorsi: AccessKit, usato da eframe 0.37 tramite accesskit_unix 0.24.1 (ultima versione), e, dalla fase 6, la dipendenza diretta di `lynkeus-app` su zbus 5.19.0 per il portale XDG. Da rivedere quando AccessKit e `lynkeus-app` non usano più zbus 5 (la 6.2.1 è disponibile dal 2026-11-01), o a ogni aggiornamento di eframe.

- **RUSTSEC-2026-0370**: `zvariant` non mantenuto. Arriva da zbus 5 su Linux (diretto e via AccessKit) senza alternative.
- **RUSTSEC-2026-0371**: `zbus_names` non mantenuto. Arriva da zbus 5 su Linux (diretto e via AccessKit) senza alternative.
- **RUSTSEC-2026-0372**: `zvariant_derive` non mantenuto. Arriva da zbus 5 su Linux (diretto e via AccessKit) senza alternative.

Nessun'altra eccezione: `exceptions = []` per le licenze. Il rischio RUSTSEC-2026-0192 (`ttf-parser`) non è ignorato in `deny.toml` perché non compare nell'albero: si evita con la feature `-notitle` di winit.

## Ultimo `cargo deny check`

Eseguito il 2026-11-04, dopo l'aggiunta di `opener` e `zbus` nella fase 6: exit code 0. Esito per sezione: advisories ok, bans ok, licenses ok, sources ok.

Warning (non bloccanti, `multiple-versions = "warn"`): versioni duplicate di calloop, calloop-wayland-source, foldhash, getrandom, linux-raw-sys, miniz_oxide, pollster (0.4.0 da rfd e 1.0.1 da eframe), rustc-hash, rustix, smithay-client-toolkit, syn, thiserror (1.0.69 e 2.0.21), thiserror-impl, hashbrown (3 versioni) e windows-sys (3 versioni).
