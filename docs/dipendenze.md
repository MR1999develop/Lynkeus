# Dipendenze

Ogni dipendenza diretta di un `Cargo.toml` ha una validazione registrata in questa tabella: versione, licenza (compatibile con GPLv3), motivo della scelta, crate che la usano, uso di `unsafe`, presenza di codice C, data di verifica ed esito. Si aggiorna a ogni aggiunta o cambio di versione, insieme a `cargo deny check`.

I crate interni del workspace (`lynkeus-core`, `lynkeus-extract`, `lynkeus-engine`) non si registrano qui: non sono dipendenze esterne.

## Tabella delle dipendenze dirette

Versioni risolte da `Cargo.lock`. Licenze da `cargo info`. Conteggio `unsafe`: occorrenze della parola `unsafe` nel sorgente `src/` del crate scaricato, commenti inclusi, quindi indicativo e non un conteggio dei blocchi. Le dipendenze di sviluppo (`dev-dependencies`) sono marcate con "(dev)".

| crate | versione | licenza | motivo | usata da | unsafe | codice C | verificata il | esito |
|---|---|---|---|---|---|---|---|---|
| anyhow | 1.0.104 | MIT OR Apache-2.0 | Errori nel binario e in xtask, con contesto e catena di cause. | lynkeus-app, xtask | sì (104 in `src`: `error`, `ptr`, `context`, `fmt`, `ensure`) | no (il `build.rs` fa solo probe del compilatore) | 2026-11-04 | ok |
| arboard | 3.7.0 | MIT OR Apache-2.0 | (dev) Misura P-08 della copia negli appunti (`p08_copy_200k` in `crates/lynkeus-app/tests/prestazioni_interfaccia.rs`, `#[ignore]`, in release, solo con `cargo xtask test-plan --clipboard`); salva il testo degli appunti e lo rimette alla fine, anche se il test fallisce. Il crate è già nell'albero come dipendenza di `egui-winit` (eframe): la dev-dependency non aggiunge crate. `default-features = false` nel workspace; le feature attive arrivano da `egui-winit`. | lynkeus-app (dev) | sì (67 in `src`: 27 in `platform/windows.rs`, compilati su Windows; 40 in `platform/osx.rs`, non compilati; 0 su Linux) | no (`build = false`, nessun `build.rs`, nessun `cc`) | 2026-11-05 | ok |
| blake3 | 1.8.7 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | Chiave della cache del testo estratto (`cache.rs`: percorso, dimensione, data e versioni) e nome unico dei file temporanei (`unique_number` in `extraction.rs`). Feature `pure` con `std`: solo Rust, senza assembly né C. | lynkeus-engine, lynkeus-app | sì (238 in `src`; compilati su x86_64 circa 160, negli intrinseci SIMD in Rust di `rust_sse2`, `rust_sse41`, `rust_avx2` e `platform.rs`, stima dai file compilati) | no (con `pure` non compila sorgenti C; il `build.rs` usa `cc` solo per sondare il compilatore su `-mavx512f`) | 2026-11-04 | ok |
| calamine | 0.38.0 | MIT | Lettura di xlsx, xls e ods nel processo estrattore (`extract/sheet.rs`). `default-features = false`, senza feature. | lynkeus-extract | no (0 in `src`) | no (`build = false`) | 2026-11-04 | ok |
| cfb | 0.16.0 | MIT | Contenitore OLE (CFB) dei file .doc (`extract/word97.rs`) e .ppt (`extract/ppt.rs`), aperto da `extract/cfbfile.rs`, con lettura dei flussi entro i limiti. Senza feature. Il .xls non passa da cfb: lo legge calamine. | lynkeus-extract | no (0 in `src`) | no (`build = false`) | 2026-11-04 | ok |
| chardetng | 1.0.0 | Apache-2.0 OR MIT | Rilevamento della codifica dei file a 8 bit (non UTF-8, non BOM) in `text/detect.rs`, sui primi byte del file, con ripiego su Windows-1252 se la proposta non è adatta. | lynkeus-extract | sì (2 in `src`) | no | 2026-11-04 | ok |
| crc32fast | 1.5.4 | MIT OR Apache-2.0 | CRC32 dei dati compressi nell'intestazione delle voci della cache (`cache.rs`, `crc32fast::hash`). Già nell'albero tramite zip. | lynkeus-engine (anche zip, in lynkeus-extract e xtask) | sì (20 in `src`, nei moduli SIMD `specialized/`, compilati su x86_64 con scelta a runtime; `lib.rs` ha `deny(unsafe_code)`) | no (`build.rs` legge solo la versione di rustc) | 2026-11-04 | ok |
| criterion | 0.8.2 | Apache-2.0 OR MIT | Benchmark di `find_words` su circa 4 MB di testo (`benches/find_words.rs`). Solo dev-dependency, senza grafici né rayon. | lynkeus-core (dev) | sì (7 in `src`) | sì solo in dev: `criterion` → `alloca` → `cc` compila `alloca.c` (vedi sezione Codice C) | 2026-11-04 | ok (nota: codice C in fase di build dei test e dei benchmark, non nel binario) |
| crossbeam-channel | 0.5.17 | MIT OR Apache-2.0 | Canale limitato (`bounded`) tra il worker di ricerca e l'interfaccia: eventi con capacità fissa e invio con timeout (`send_timeout`), `try_recv` per svuotarlo nel ciclo dell'interfaccia (`handle.rs`, `worker.rs`). | lynkeus-engine | sì (66 in `src`) | no | 2026-11-04 | ok |
| directories | 6.0.0 | MIT OR Apache-2.0 | Cartella di configurazione per piattaforma (`settings.toml` e posizione della finestra): `%APPDATA%\Lynkeus Searcher\config` su Windows, `~/.config/lynkeus-searcher` su Linux (`platform::config_dir`). | lynkeus-app | no (0) | no | 2026-11-04 | ok (nota: repository archiviato, ultima release 2025-01-12; vedi Dettagli di verifica) |
| eframe | 0.37.0 | MIT OR Apache-2.0 | Finestra e ciclo degli eventi dell'interfaccia. Senza `default_fonts` per la licenza Ubuntu Font: i font si caricano dal sistema. Feature `persistence` solo per ricordare la finestra (`persist_window` e `persistence_path` in `main.rs`): le impostazioni vanno in `settings.toml`. | lynkeus-app | sì (26 in `src`) | no diretto; su Linux indiretto (vedi sezione Codice C) | 2026-11-04 | ok |
| egui | 0.37.0 | MIT OR Apache-2.0 | Widget e disegno dell'interfaccia. Senza `default_fonts`, come eframe. | lynkeus-app | sì (4 in `src`) | no | 2026-11-04 | ok |
| egui_extras | 0.37.0 | MIT OR Apache-2.0 | Tabella virtualizzata dei risultati (`TableBuilder`): si disegnano solo le righe visibili (`ui/table.rs`). Senza feature. | lynkeus-app | sì (2 in `src`) | no | 2026-11-04 | ok |
| egui_kittest | 0.37.0 | MIT OR Apache-2.0 | Test dell'interfaccia (`tests/interfaccia.rs`): l'app `eframe::App` vera, guidata per ruolo e nome accessibile. Feature `eframe` attiva. Solo dev-dependency. | lynkeus-app (dev) | sì (2 in `src`) | no | 2026-11-04 | ok |
| encoding_rs | 0.8.44 | (Apache-2.0 OR MIT) AND BSD-3-Clause | Decodifica a blocchi dei file di testo (`Decoder`, tabelle WHATWG) in UTF-8: UTF-16 e le codifiche rilevate, come Windows-1252 (`text/mod.rs`, `text/detect.rs`). | lynkeus-extract | sì (193 in `src`) | no (Rust puro; `build.rs` solo imposta un cfg) | 2026-11-04 | ok |
| flate2 | 1.1.10 | MIT OR Apache-2.0 | Decodifica gzip (`.gz`, `.tgz`, `.tar.gz`) in `extract/tarball.rs` (`MultiGzDecoder`). Feature di default (`rust_backend`, `runtime_detection`): il backend attivo è zlib-rs, senza C. Lo usano anche il deflate di sevenz-rust2 e zip. | lynkeus-extract (dev: `tests/formats.rs`, per costruire i .gz di prova) | sì (37 in `src`; `ffi/c.rs` con 25 non si compila, perché `any_c_zlib` non è attiva; il backend zlib-rs ha 456 occorrenze nel sottoalbero, vedi nota) | no (`build = false`; il C si attiva solo con `zlib` o `any_c_zlib`) | 2026-11-04 | ok, con `unsafe` da rivalutare (zlib-rs, vedi nota) |
| fluent-templates | 0.15.1 | MIT/Apache-2.0 | Traduzioni Fluent in `locales/it` e `locales/en`. | lynkeus-app | no (0) | no | 2026-11-04 | ok |
| getrandom | 0.3.4 (la 0.4.3 è nel lock per altre dipendenze) | MIT OR Apache-2.0 | Token casuale dell'istanza unica (`instance.rs`, `Token::random`) e suffisso casuale del nome del canale (`random_suffix`), con `getrandom::fill` dal generatore del sistema operativo; altre casualità di piattaforma in `platform/mod.rs` e `platform/windows_ffi.rs`. Nessuna feature esplicita (`default`). | lynkeus-app | sì (93 in `src`, 26 file: chiamate di sistema per piattaforma) | no (`build.rs` legge solo la versione di rustc e il sanitizer; nessun `cc`) | 2026-11-05 | ok (nota: la 0.4.3 è l'ultima; la linea 0.3 è richiesta dal progetto, e `cargo deny` segnala il duplicato) |
| ignore | 0.4.34 | Unlicense OR MIT | Scansione ricorsiva delle cartelle (`WalkBuilder`). | lynkeus-engine | no (0) | no | 2026-11-04 | ok |
| libfuzzer-sys | 0.4.13 | (MIT OR Apache-2.0) AND NCSA | Runtime di libFuzzer per i quattro target di `fuzz/` (`fuzz_targets/`). Solo nel crate `fuzz/`, fuori dal workspace: non entra nel binario. | lynkeus-fuzz (`fuzz/`) | sì (11 in `src`, nell'API FFI verso libFuzzer) | sì, solo nel build di `fuzz/`: `build.rs` con `cc` compila il C++ di libFuzzer (`build.cpp(true)`) | 2026-11-04 | problema: licenza NCSA fuori da `allow` di `deny.toml`; `cargo deny` dentro `fuzz/` fallisce (exit 4) |
| lz4_flex | 0.14.0 | MIT | Compressione delle voci della cache (`cache.rs`, `block::compress` e `block::decompress`, con dimensione dichiarata controllata prima di decomprimere). Feature `safe-encode`, `safe-decode`, `checked-decode`: il codice `unsafe` non è compilato. | lynkeus-engine | sì nel sorgente (76 in 4 file), 0 compilati con le feature del progetto (i moduli con `unsafe` sono esclusi da `cfg` o hanno `forbid(unsafe_code)`) | no (`build = false`) | 2026-11-04 | ok |
| mail-parser | 1.0.1 | Apache-2.0 OR MIT | Lettura dei messaggi .eml con allegati (`extract/eml.rs`) e riduzione dell'HTML a testo (`extract/html.rs`, `html_to_text`). Feature `full_encoding` per i charset rari tramite encoding_rs (già nell'albero). `default-features = false`. | lynkeus-extract | sì (108 in `src`; su x86_64 si compilano 82: `scan/x86.rs` 58, `scan/simd.rs` 22, `store/str.rs` 2 con `from_utf8_unchecked`; `scan/neon.rs` 26 solo su aarch64) | no (`build = false`) | 2026-11-04 | ok, con `unsafe` da rivalutare (threat model, sezione 5) |
| memchr | 2.8.3 | Unlicense OR MIT | Ricerca veloce del byte `\n` e `\0`: conteggio delle righe e ricerca del punto di taglio dei blocchi di testo (`textfile.rs`, `text/mod.rs`, `text/detect.rs`). | lynkeus-extract, lynkeus-engine | sì (340 in `src`, SIMD) | no | 2026-11-04 | ok (nota: molto `unsafe` per la ricerca SIMD, crate molto usato) |
| opener | 0.9.0 | MIT OR Apache-2.0 | Solo Windows. Apre un file con il programma predefinito (`opener::open`, `ShellExecuteW`) e lo mostra selezionato in Esplora risorse (`opener::reveal`, `SHOpenFolderAndSelectItems`), in `platform.rs`. Feature `reveal`, senza default. Su Linux non si usa: lì c'è il portale XDG con `zbus`. | lynkeus-app (solo Windows) | sì (17 in `src`, FFI di Shell e COM nei moduli `windows`) | no (`build = false`; `windows-sys` è solo binding) | 2026-11-04 | ok |
| pdf_oxide | =0.3.78 (versione bloccata) | MIT OR Apache-2.0 | Testo dei PDF riga per riga con `extract_text_lines`, senza ripiego (ADR 0002). `default-features = false` (niente `icc`), `legacy-crypto` per i PDF con password vuota. Porta `ttf-parser` (RUSTSEC-2026-0192, eccezione in `deny.toml`). | lynkeus-extract (`extract/pdf.rs`) | sì (73 in `src`; compilati: 65 in `ffi.rs`, l'interfaccia C per altri linguaggi, compilata su tutti i target non wasm; il resto è in `rendering`, feature non attiva, o nei test) | no (nessun `build.rs`; nel sottoalbero di Windows e Linux nessun `cc` né crate `-sys` con C) | 2026-11-04 | ok, con eccezione RUSTSEC-2026-0192; 144 crate transitivi su Windows, 141 su Linux (vedi sotto) |
| postcard | 1.1.3 | MIT OR Apache-2.0 | Messaggi del protocollo del processo estrattore (`protocol.rs`, `postcard::to_io` e `take_from_bytes`). Feature `use-std`, senza `heapless` né `alloc`. | lynkeus-extract | sì (12 in `src`, nei flavor di slice di `ser` e `de`, compilati) | no (`build = false`) | 2026-11-04 | ok |
| proptest | 1.12.0 | MIT OR Apache-2.0 | Test di proprietà di `find_words`, `word_text` e `snippet` (`lynkeus-core/tests/proprieta.rs`), dei lettori a blocchi (`text/mod.rs`, `textfile.rs`) e dell'esportazione CSV (`export.rs`). Solo dev-dependency. | lynkeus-core (dev), lynkeus-extract (dev), lynkeus-engine (dev), lynkeus-app (dev) | sì (8 in `src`) | no | 2026-11-04 | ok |
| quick-xml | 0.42.0 | MIT | Lettura XML di docx, pptx, ODF ed EPUB (`extract/xml.rs`, `NsReader`). Il progetto rifiuta `<!DOCTYPE>` (threat model). Nessuna feature richiesta dal workspace. | lynkeus-extract (diretta); transitiva tramite calamine, office_oxide e pdf_oxide | no (`#![forbid(unsafe_code)]`; le 9 occorrenze sono commenti) | no (`build = false`) | 2026-11-04 | ok (nota: 0.41.0 nel lock tramite `wayland-scanner` su Linux, warning di duplicato) |
| regex | 1.13.1 | MIT OR Apache-2.0 | Ricerca lineare dei termini; tabelle Unicode delle categorie `Mn` e `P` (`unicode.rs`). | lynkeus-core | sì (1 in `src`) | no | 2026-11-04 | ok |
| rfd | 0.17.2 | MIT | Dialoghi nativi per scegliere cartelle e file (`FileDialog`, `ui/mod.rs`). Su Linux tramite il portale XDG, senza libreria GTK; su Windows API di sistema. | lynkeus-app | sì (249 in `src`, FFI delle piattaforme) | no (il `build.rs` fa solo controlli di feature e cfg) | 2026-11-04 | ok |
| rlimit | 0.11.0 | MIT | Solo Unix (`cfg(unix)`): limite `RLIMIT_AS` del processo estrattore, impostato da sé all'avvio (`platform.rs`, `Resource::AS.set`). Su Windows non entra nell'albero. | lynkeus-extract (solo Unix) | sì (8 in `src`, chiamate `getrlimit`, `setrlimit` e `prlimit` tramite libc, compilate su Unix) | no (`build.rs` solo probe di cfg, nessun C) | 2026-11-04 | ok |
| rustix | 1.1.6 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Solo non Windows (`cfg(not(windows))`): permessi delle cartelle private (`fstat`, `geteuid`, `umask` in `platform/mod.rs`), apertura con `O_NOFOLLOW` e controllo del proprietario sul descrittore aperto, `SO_PEERCRED` sul socket dell'istanza unica (`platform/mod.rs`). Nel processo estrattore, lo stesso controllo del proprietario in `platform.rs` di lynkeus-engine. `default-features = false` con `std`, `fs`, `process`, `net`. | lynkeus-app, lynkeus-engine (non Windows) | sì (1.672 in `src`, 154 file; 1.188 in `src/backend`, il backend `linux_raw` è quello compilato su Linux senza `use-libc`, con istruzioni inline; su Windows non entra nell'albero) | no (`build.rs` solo probe di cfg; nessun `cc`) | 2026-11-05 | ok (nota: la 0.38.44 è nel lock per altre dipendenze, warning di duplicato in `cargo deny`) |
| serde | 1.0.229 | MIT OR Apache-2.0 | Tipi del protocollo tra processi e impostazioni (con `derive`). | lynkeus-core, lynkeus-extract, lynkeus-engine, lynkeus-app | sì (2 in `src`) | no (il `build.rs` fa solo probe) | 2026-11-04 | ok |
| sevenz-rust2 | 0.24.2 | Apache-2.0 | Lettura degli archivi 7z (`extract/sevenz.rs`), blocco per blocco, con i limiti per voce. Solo deflate: `default-features = false` con la feature `deflate`; niente compressione; le voci cifrate danno "protetto da password". | lynkeus-extract | no nel codice del crate (le 3 occorrenze sono nei messaggi "unsafe entry path"); sì nella dipendenza transitiva lzma-rust2 0.23.0, con feature `optimization`: 15 in `src`, vedi nota | no (`build = false`) | 2026-11-04 | ok, con `unsafe` transitivo da rivalutare (threat model, sezione 5) |
| subtle | 2.6.1 | BSD-3-Clause | Confronto in tempo costante del token dell'istanza unica (`instance.rs`, `Token::matches`, con `ConstantTimeEq`). `default-features = false`, feature `std`. | lynkeus-app | sì (2 in `src/lib.rs`: il `black_box` di ottimizzazione e il cast da `i8` a `Ordering`, entrambi con `SAFETY`) | no (nessun `build.rs`, nessun C) | 2026-11-05 | ok (nota: manutenzione ferma, nessuna release dal 2024-06-24 e nessun push dal 2024-08-03; da rivalutare alla prossima revisione dell'istanza unica) |
| sys-locale | 0.3.2 | MIT OR Apache-2.0 | Lingua di sistema, per scegliere l'interfaccia in italiano o inglese. | lynkeus-app | sì (7 in `src`, chiamate di sistema Windows, Apple, Android) | no | 2026-11-04 | ok (nota: ultima release 2024-11-01, repository attivo) |
| tar | 0.4.46 | MIT OR Apache-2.0 | Lettura degli archivi tar (`extract/tarball.rs`, `tar::Archive`); in test, costruzione di tar di prova (`tests/formats.rs`). `default-features = false`: senza `xattr`. | lynkeus-extract | sì (26 in `src`: intestazioni in `header.rs` 22, `entry.rs` 2, `builder.rs` 2) | no (`build = false`) | 2026-11-04 | ok |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | Cartelle temporanee per i test di scansione (`engine/tests/scansione.rs`) e delle impostazioni (`settings.rs`). Solo dev-dependency. | lynkeus-engine (dev), lynkeus-app (dev) | sì (4 in `src`) | no | 2026-11-04 | ok |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | Errori tipizzati delle librerie. | lynkeus-core, lynkeus-extract, lynkeus-engine, lynkeus-app | no (0) | no (il `build.rs` fa solo probe) | 2026-11-04 | ok (nota: la 1.0.69 entra in transitivo tramite calloop, warning di duplicato) |
| toml | 1.2.0 (richiesta 1.1.8) | MIT OR Apache-2.0 | Lettura e scrittura di `settings.toml` (`Table`, `Value`, `to_string`) in `settings.rs`. Solo `parse`, `display`, `serde`. | lynkeus-app | sì (3 in `src`) | no | 2026-11-04 | ok |
| tracing | 0.1.44 | MIT | Log strutturati con campi (`info!`, `warn!`, `error!`) nel guscio dell'app: `app.rs`, `main.rs`, `instance.rs`, `platform/mod.rs`, `private_dirs.rs`, `ui/mod.rs`, `diagnostics.rs`. Il nucleo e il motore non lo usano. `default-features = false`, feature `std`. | lynkeus-app | sì (7 in `src`, 3 file: `instrument.rs` (5, pin delle future), `lib.rs` (1, `from_utf8_unchecked`), `span.rs` (1, `unsafe impl Sync`)) | no (nessun `build.rs`, nessun C) | 2026-11-05 | ok |
| tracing-subscriber | 0.3.23 | MIT | Filtro di livello regolabile a runtime (`reload`, `LevelFilter`) e formato dei log su file e su stderr (`diagnostics.rs`: `fmt::layer`, `Registry`). `default-features = false`, feature `fmt` e `std`. | lynkeus-app | sì (28 in `src`, 6 file: `layer/mod.rs`, `layer/layered.rs`, `filter/layer_filters/mod.rs`, `reload.rs`, `fmt/mod.rs`, `fmt/fmt_layer.rs`) | no (nessun `build.rs`, nessun C) | 2026-11-05 | ok |
| tracing-appender | 0.2.5 | MIT | Log su file con rotazione giornaliera e numero massimo di file (`diagnostics.rs`, `rolling::Builder` con `Rotation::DAILY`), scritti da un thread a parte (`non_blocking`). Senza feature. | lynkeus-app | no (0 in `src`) | no (nessun `build.rs`, nessun C) | 2026-11-05 | ok |
| tracing-log | 0.2.0 | MIT | Porta nel log dell'app i messaggi del crate `log` di eframe, winit e wgpu (`LogTracer::builder()` in `diagnostics.rs`, con il livello del log scelto nelle impostazioni). `default-features = false`, feature `log-tracer` e `std`; senza `interest-cache` (`lru`, `ahash`). Il crate `log` 0.4.35 è già nell'albero. | lynkeus-app | sì (1 blocco `unsafe` in `src/lib.rs`, `LogVisitor::record_str`: cast di `&str` al lifetime dell'evento, con commento di sicurezza; il modulo non è sotto un `cfg` di feature) | no (nessun `build.rs`, nessun C) | 2026-11-05 | ok |
| unicode-normalization | 0.1.26 | (MIT OR Apache-2.0) AND Unicode-3.0 | Normalizzazione NFC/NFD dei termini (`AccentMode`) e decomposizione canonica per la tabella delle varianti accentate (`unicode.rs`, `pattern.rs`). | lynkeus-core (anche in test `proprieta.rs`, dev) | sì (5 in `src`) | no | 2026-11-04 | ok |
| valuable (transitiva) | 0.1.1 | MIT | Dipendenza di `tracing-core` 0.1.36 solo sotto `[target.'cfg(tracing_unstable)'.dependencies]` (opzionale, senza `default-features`). Nel `Cargo.lock` perché il lock include le dipendenze di ogni target, ma non è compilata: `cargo tree --target all -i valuable` non stampa nulla. Entra solo se si imposta `--cfg tracing_unstable`, che il progetto non imposta (`.cargo`, CI). | tracing-core (sotto tracing e winit) | no nel codice compilato (non compilata; nel sorgente 2 occorrenze, entrambe in commenti di documentazione, nessun blocco) | no (`build.rs` solo imposta cfg per gli atomici, nessun `cc`) | 2026-11-05 | ok (nota: se un giorno si attiva `tracing_unstable`, diventa dipendenza compilata: da rivalutare) |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 | Solo Windows: API Win32 senza involucri per il processo estrattore (`CreateProcessW`, Job Object con limite di memoria, pipe, `SetDefaultDllDirectories`), descrittori di sicurezza delle cartelle private (`Authorization`: `ConvertStringSecurityDescriptorToSecurityDescriptorW`, `GetSecurityInfo`), file (`CreateFileW`, `Storage::FileSystem`) e `AllowSetForegroundWindow` dell'istanza unica (`UI::WindowsAndMessaging`), tutto in `platform/windows_ffi.rs`. Dal 2026-11-05 anche `GetSysColor` (`Win32_Graphics_Gdi`), `HIGHCONTRASTW` (`Win32_UI_Accessibility`) e `AssocIsDangerous` (`Win32_UI_Shell`). Dal 2026-11-05 anche dev-dependency di `lynkeus-engine` e `lynkeus-app` con `Win32_System_ProcessStatus`: una chiamata a `K32GetProcessMemoryInfo` (memoria privata di picco) nelle misure del piano di test, `crates/lynkeus-engine/tests/misura/mod.rs`, inclusa anche dalle misure dell'app; non entra nel binario. | lynkeus-app (solo Windows); lynkeus-engine e lynkeus-app (dev, solo Windows) | sì nel sorgente (12.530 in 192 file: dichiarazioni FFI generate; conteggio indicativo) | no (`build = false`, solo binding) | 2026-11-05 | ok (nota: nel lock anche 0.52.0 e 0.59.0 per altre dipendenze, warning di duplicato) |
| winit | 0.30.13 | Apache-2.0 | Solo Linux. Non è importata nel codice: serve la feature Wayland caricata a runtime e le decorazioni su GNOME senza titolo (`-notitle`), che evitano la catena ab_glyph → ttf-parser. `ttf-parser` (RUSTSEC-2026-0192) entra comunque nell'albero tramite `pdf_oxide`: vedi eccezioni. | lynkeus-app (solo Linux) | sì (932 in `src`, FFI delle piattaforme) | no diretto; su Linux indiretto (vedi sezione Codice C) | 2026-11-04 | ok |
| winreg | 0.56.0 | MIT | Solo Windows: voce del clic destro in Esplora risorse, scritta sotto `HKEY_CURRENT_USER` (`platform/mod.rs`, modulo `context_menu_windows`), con la chiave aperta a ogni operazione (`RegKey` non è `Sync`). Senza feature. | lynkeus-app (solo Windows) | sì (38 in `src`: 24 in `reg_key.rs` per le chiamate ad `advapi32`, 6 in `types.rs`, 5 in `transaction.rs`, 2 in `reg_key_metadata.rs`, 1 in `common.rs`; il progetto non ha `unsafe` suo) | no (`build = false`, nessun `cc`) | 2026-11-05 | ok |
| winresource | 0.1.34 | MIT | Build-dependency di `lynkeus-app` solo per Windows (`build.rs`): icona, informazioni di versione e manifest collegati al solo binario `lynkeus-searcher`, con `rc.exe` del Windows SDK. `default-features = false`, quindi senza `toml`. | lynkeus-app (build, solo Windows) | no (0 in `lib.rs`, unico sorgente) | no (`build = false` nel crate; nessun `cc` tra le sue dipendenze) | 2026-11-05 | ok |
| sha2 | 0.11.1 | MIT OR Apache-2.0 | Calcolo dei SHA-256 per `SHA256SUMS` nel rilascio (`cargo xtask dist`). | xtask | sì (39 in `src`) | no | 2026-11-04 | ok |
| zbus | 5.19.0 | MIT | Solo non Windows (`cfg(not(windows))`; nei target del progetto è Linux): portale XDG `org.freedesktop.portal.OpenURI`, metodi `OpenFile` e `OpenDirectory` con il file come descrittore, in `platform.rs` (`call_open_uri_portal`). Feature `async-io` e `blocking-api`, senza default. Resta la 5 perché AccessKit la porta già nell'albero: la 6 ne aggiungerebbe una seconda copia. | lynkeus-app (non Windows) | sì (58 in `src`, socket e FFI) | no (`build = false`, nessun `build.rs`) | 2026-11-04 | ok (nota: la 6.2.1 è disponibile, vedi Dettagli di verifica) |
| zip | 9.3.0 | MIT | Archivi zip (`extract/archive.rs`) e contenitori docx, xlsx, pptx, ODF ed EPUB nel processo estrattore; zip di rilascio in `cargo xtask dist`. Solo `deflate`, `default-features = false`. | lynkeus-extract, xtask | sì (48 in `src`, nei moduli di lettura e scrittura; compilati in parte, non misurato quali con le feature del progetto) | no (`build = false`) | 2026-11-04 | ok |

Nota sul `Cargo.toml` del workspace: `toml` è richiesto come `1.1.8` ma `Cargo.lock` risolve `1.2.0`, compatibile in semver.

Crate transitivi di `pdf_oxide` 0.3.78 (feature del progetto: `default-features = false`, `legacy-crypto`): 144 su Windows e 141 su Linux, normali e di build, escluso il crate stesso. Il dato su Windows coincide con l'ADR 0002 (144 senza feature predefinite; 145 con `icc`). Su Windows 60 di questi non compaiono nell'albero del workspace senza `pdf_oxide`; gli altri 84 sono già condivisi con il resto del progetto (misurato rimuovendo `pdf_oxide` da una copia di lavoro del workspace). Tra i 60: `ttf-parser` 0.25.1 e `skrifa` 0.42.1 (font, RUSTSEC-2026-0192), `office_oxide` 0.1.16, `rayon` 1.12.0, `taffy` 0.14.0, `tiff` 0.11.3, `jpeg-decoder` 0.3.2, `fax`, `brotli` 9.1.1, `aes`, `cbc`, `md-5`, `chrono`, `jiff`, `env_logger`, `nom` 8.0.0, `phf` e `phf_*`, `read-fonts`, `write-fonts`, `font-types`, `subsetter`, `zune-*`, `weezl`. Nessuno di loro usa `cc` né è un crate `-sys` con C. Licenze e advisory sono coperti da `cargo deny check`, non verificati uno per uno.

Crate transitivi nuovi di `opener` (solo Windows): 24, oltre a `opener` stesso. Sono in `Cargo.lock` dalla fase 6 e non hanno una riga propria in tabella: `form_urlencoded`, `icu_collections`, `icu_locale_core`, `icu_normalizer`, `icu_normalizer_data`, `icu_properties`, `icu_properties_data`, `icu_provider`, `idna`, `idna_adapter`, `litemap`, `normpath`, `potential_utf`, `stable_deref_trait`, `synstructure`, `url`, `utf8_iter`, `utf_types`, `writeable`, `yoke`, `yoke-derive`, `zerofrom-derive`, `zerotrie`, `zerovec-derive`. Vengono da `url` 2.5.8 e `normpath` 1.5.2, dipendenze di `opener` su Windows, e dalla catena ICU di `idna`. Il confronto è con il `Cargo.lock` di `a74f053` (prima della fase 6). Licenze e advisory di questi crate sono coperti da `cargo deny check`, non verificati uno per uno. `zbus` non aggiunge pacchetti nuovi al lock: era già presente tramite AccessKit, e la fase 6 lo dichiara solo come dipendenza diretta di `lynkeus-app` su non Windows.

## Strumenti della CI

Strumenti installati nei workflow con una versione esatta. Non sono dipendenze di alcun `Cargo.toml`: sono registrati qui perché producono gli artefatti del rilascio o eseguono il fuzzing. Versione e licenza da `cargo info`; date di pubblicazione da crates.io; `unsafe` e C contati nei sorgenti pubblicati su crates.io (`src/`, `build.rs`), non nel binario installato, che arriva da `taiki-e/install-action` (fissato a `v2.90.1` con SHA).

| strumento | versione | licenza | motivo | usato da | unsafe | codice C | verificata il | esito |
|---|---|---|---|---|---|---|---|---|
| cargo-auditable | 0.7.7 | MIT OR Apache-2.0 | Compila il binario di rilascio con l'elenco delle dipendenze incorporato (`cargo auditable build`, in `xtask/src/dist.rs`), leggibile da `cargo audit bin`. | `release.yml` (job di rilascio) | no (0 in `src`) | no (nessun `build.rs`; dipendenze transitive non verificate) | 2026-11-05 | ok (pubblicata il 2026-10-02; repository `rust-secure-code/cargo-auditable`, ultimo push non verificato) |
| cargo-cyclonedx | 0.5.9 | Apache-2.0 | Genera lo SBOM CycloneDX (`<pacchetto>.cdx.json`) accanto a ogni zip di rilascio (`generate_sbom` in `xtask/src/dist.rs`). | `release.yml` (job di rilascio) | no (0 in `src`) | no (nessun `build.rs`; dipendenze transitive non verificate) | 2026-11-05 | ok (pubblicata il 2026-03-19; repository `CycloneDX/cyclonedx-rust-cargo`, ultimo push non verificato) |
| cargo-fuzz | 0.13.2 | MIT OR Apache-2.0 | Esegue i target di `fuzz/` con libFuzzer (`cargo fuzz run`) nel fuzzing pianificato dei parser. | `fuzz.yml` (toolchain nightly) | 1 occorrenza in `src/main.rs`, in un commento; nessun blocco | no (nessun `build.rs`; il C di libFuzzer è in `libfuzzer-sys`, registrato sopra) | 2026-11-05 | ok (pubblicata il 2026-06-09; repository `rust-fuzz/cargo-fuzz`, ultimo push non verificato) |

## Dettagli di verifica

Manutenzione dal repository GitHub (`pushed_at`, `archived`) e dall'ultimo aggiornamento su crates.io, entrambi verificati il 2026-11-04. Per tutti i crate la versione risolta coincide con l'ultima stabile su crates.io, tranne `zbus` (risolta 5.19.0, ultima 6.2.1) e `getrandom` (risolta 0.3.4, ultima 0.4.3). Per i crate della fase 7 la manutenzione è verificata il 2026-11-05.

| crate | ultima versione stabile | ultimo aggiornamento crates.io | ultimo push del repository | archiviato | note |
|---|---|---|---|---|---|
| anyhow | 1.0.104 | 2026-07-18 | 2026-11-03 | no | |
| arboard | 3.7.0 | 2026-10-15 | 2026-11-04 | no | repository `1Password/arboard`, attivo (open issue 39); MIT OR Apache-2.0 |
| blake3 | 1.8.7 | 2026-08-20 | 2026-10-01 | no | repository `BLAKE3-team/BLAKE3`, attivo; Apache-2.0 sul repository, CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception su crates.io |
| calamine | 0.38.0 | 2026-11-02 | 2026-11-04 | no | repository `tafia/calamine`, attivo; MIT |
| cfb | 0.16.0 | 2026-10-31 | 2026-10-31 | no | repository `mdsteele/rust-cfb`, attivo; MIT |
| chardetng | 1.0.0 | 2026-03-30 | 2026-03-30 | no | unica release stabile, nessun aggiornamento da allora |
| crc32fast | 1.5.4 | 2026-11-01 | 2026-11-01 | no | repository `srijs/rust-crc32fast`; Apache-2.0 sul repository, MIT OR Apache-2.0 su crates.io |
| criterion | 0.8.2 | 2026-02-04 | 2026-10-13 | no | |
| crossbeam-channel | 0.5.17 | 2026-09-05 | 2026-11-01 (repository crossbeam) | no | |
| directories | 6.0.0 | 2025-01-12 | 2025-01-12 | **sì** | repository archiviato su GitHub: nessuna manutenzione né correzioni future. `cargo deny` non segnala advisory, ma è un crate da tenere d'occhio o da sostituire se serve |
| eframe | 0.37.0 | 2026-10-15 | 2026-11-02 (repository egui) | no | `persistence` aggiunge `ron` 0.12.3 (MIT OR Apache-2.0), transitiva e non registrata qui |
| egui | 0.37.0 | 2026-10-15 | 2026-11-02 | no | |
| egui_extras | 0.37.0 | 2026-10-15 | 2026-11-02 (repository egui) | no | |
| egui_kittest | 0.37.0 | 2026-10-15 | 2026-11-02 (repository egui) | no | |
| encoding_rs | 0.8.44 | 2026-10-30 | 2026-10-30 | no | |
| flate2 | 1.1.10 | 2026-11-02 | 2026-11-02 | no | versione pubblicata il 2026-08-28; repository `rust-lang/flate2-rs`, attivo; Apache-2.0 sul repository, MIT OR Apache-2.0 su crates.io |
| fluent-templates | 0.15.1 | 2026-08-05 | 2026-08-05 | no | |
| getrandom | 0.4.3 (risolta 0.3.4) | 2026-06-17 | 2026-11-02 | no | repository `rust-random/getrandom`, attivo; MIT OR Apache-2.0. La linea 0.3 è quella richiesta dal progetto: il passaggio alla 0.4 è un aggiornamento da pianificare |
| ignore | 0.4.34 | 2026-10-27 | 2026-10-27 | no | |
| libfuzzer-sys | 0.4.13 | 2026-06-04 | 2026-06-04 | no | versione pubblicata il 2026-06-04; repository `rust-fuzz/libfuzzer`, attivo; Apache-2.0 sul repository, `(MIT OR Apache-2.0) AND NCSA` su crates.io |
| lz4_flex | 0.14.0 | 2026-07-14 | 2026-10-25 | no | repository `PSeitz/lz4_flex`, attivo; MIT |
| mail-parser | 1.0.1 | 2026-11-02 | 2026-11-02 | no | versione pubblicata il 2026-11-02 (due giorni fa): da seguire nei primi aggiornamenti. Repository `stalwartlabs/mail-parser`, attivo; GitHub non riconosce la licenza, crates.io riporta Apache-2.0 OR MIT |
| memchr | 2.8.3 | 2026-07-08 | 2026-08-10 | no | |
| opener | 0.9.0 | 2026-09-29 | 2026-11-01 | no | pubblicata il 2026-09-29. La feature `reveal` su Linux porterebbe `zbus`, ma `opener` è dichiarato solo per Windows |
| pdf_oxide | 0.3.78 | 2026-09-08 | 2026-10-31 | no | repository `yfedoseev/pdf_oxide`, attivo, 222 issue aperte; Apache-2.0 sul repository, MIT OR Apache-2.0 su crates.io. Rilasci molto frequenti (ADR 0002): la versione resta bloccata per scelta |
| postcard | 1.1.3 | 2025-07-24 | 2026-10-21 | no | repository `jamesmunns/postcard`, attivo; Apache-2.0 sul repository, MIT OR Apache-2.0 su crates.io. Nessuna release da oltre un anno e mezzo: da tenere d'occhio |
| proptest | 1.12.0 | 2026-10-10 | 2026-10-10 | no | |
| quick-xml | 0.42.0 | 2026-08-22 | 2026-11-02 | no | repository `tafia/quick-xml`, attivo; MIT |
| regex | 1.13.1 | 2026-07-15 | 2026-10-28 | no | |
| rfd | 0.17.2 | 2026-10-22 | 2026-10-25 | no | |
| rlimit | 0.11.0 | 2026-02-01 | 2026-11-01 | no | repository `Nugine/rlimit`, attivo; MIT |
| rustix | 1.1.6 | 2026-10-27 | 2026-10-27 | no | repository `bytecodealliance/rustix`, attivo; Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| serde | 1.0.229 | 2026-07-18 | 2026-10-16 | no | |
| sevenz-rust2 | 0.24.2 | 2026-10-30 | 2026-11-01 | no | versione pubblicata il 2026-10-30; repository `hasenbanck/sevenz-rust`, attivo; Apache-2.0 |
| subtle | 2.6.1 | 2024-06-24 | 2024-08-03 | no | repository `dalek-cryptography/subtle`, non archiviato ma fermo: nessuna release né push da oltre due anni. `cargo deny` non segnala advisory. Da tenere d'occhio |
| sys-locale | 0.3.2 | 2024-11-01 | 2026-11-04 | no | nessuna release da oltre due anni: da tenere d'occhio |
| tar | 0.4.46 | 2026-05-18 | 2026-09-21 | no | versione pubblicata il 2026-05-18; repository `composefs/tar-rs`, attivo; Apache-2.0 sul repository, MIT OR Apache-2.0 su crates.io |
| tempfile | 3.27.0 | 2026-03-11 | 2026-10-30 | no | |
| thiserror | 2.0.21 | 2026-09-23 | 2026-10-16 | no | |
| toml | 1.2.0 | 2026-11-04 | 2026-11-04 | no | |
| tracing | 0.1.44 | 2025-12-18 | 2026-10-29 (repository `tokio-rs/tracing`) | no | MIT; stesso repository per `tracing-subscriber` e `tracing-appender` |
| tracing-appender | 0.2.5 | 2026-04-17 | 2026-10-29 (repository `tokio-rs/tracing`) | no | MIT |
| tracing-log | 0.2.0 | 2023-10-25 | non verificato (GitHub non raggiungibile dalla sessione) | no | MIT su crates.io; repository `tokio-rs/tracing` (come `tracing`). Nessuna release successiva a 0.2.0 su crates.io: da tenere d'occhio |
| valuable | 0.1.1 | 2025-01-17 | non verificato (GitHub non raggiungibile dalla sessione) | no | MIT su crates.io; repository `tokio-rs/valuable`. Transitiva non compilata, vedi tabella |
| tracing-subscriber | 0.3.23 | 2026-03-13 | 2026-10-29 (repository `tokio-rs/tracing`) | no | MIT |
| unicode-normalization | 0.1.26 | 2026-10-16 | 2026-10-29 | no | |
| windows-sys | 0.61.2 | 2025-10-06 | 2026-11-04 | no | repository `microsoft/windows-rs`, attivo; Apache-2.0 sul repository, MIT OR Apache-2.0 su crates.io |
| winreg | 0.56.0 | 2026-03-14 | 2026-03-14 | no | repository `gentoo90/winreg-rs`, attivo; MIT |
| winresource | 0.1.34 | 2026-10-12 | 2026-10-12 | no | repository `BenjaminRi/winresource`, attivo; MIT |
| winit | 0.30.13 | 2026-10-24 | 2026-11-03 | no | |
| sha2 | 0.11.1 | 2026-10-12 | 2026-11-03 | no | |
| zbus | 5.19.0 (ultima 6.2.1, pubblicata il 2026-11-01) | 2026-08-09 (5.19.0) | 2026-11-03 | no | repository `z-galaxy/zbus` attivo. GitHub non riconosce la licenza (`NOASSERTION`), ma il file `LICENSE` è MIT e `cargo info` riporta MIT |
| zip | 9.3.0 | 2026-11-03 | 2026-11-03 | no | |

## Feature attive

Feature risolte con `cargo tree -p <crate> --depth 0 -f "{p} | {f}"`.

| crate | feature attive |
|---|---|
| anyhow | default, std |
| arboard | core-graphics, image, image-data, wayland-data-control, windows-sys, wl-clipboard-rs (`default-features = false` nel workspace; le feature le attiva `egui-winit` tramite eframe) |
| blake3 | pure, std (`default-features = false`) |
| calamine | nessuna (`default-features = false`) |
| cfb | nessuna |
| chardetng | nessuna |
| crc32fast | default, std |
| criterion | cargo_bench_support (`default-features = false`) |
| crossbeam-channel | default, std |
| directories | nessuna |
| eframe | accesskit, clipboard, persistence, ron, serde, system_fonts, wayland, wgpu, wgpu_no_default_features, x11 (`persistence` porta `ron` e `serde`) |
| egui | bytemuck (senza `default_fonts`, `default-features = false`) |
| egui_extras | nessuna (`default-features = false`) |
| egui_kittest | eframe (`default-features = false`) |
| encoding_rs | alloc, default |
| flate2 | default (`rust_backend`, `runtime_detection`); risolte anche `miniz_oxide` e `zlib-rs` (il backend attivo è zlib-rs, attivato anche da zip e sevenz-rust2) |
| fluent-templates | default, fluent-template-macros, ignore, macros |
| getrandom | nessuna (`default`, senza feature esplicite) |
| ignore | nessuna |
| libfuzzer-sys | default (`link_libfuzzer`) |
| lz4_flex | alloc, checked-decode, safe-decode, safe-encode, std (`default-features = false`; `frame` e `nightly` non attive) |
| mail-parser | full_encoding (`default-features = false`) |
| memchr | alloc, default, std |
| opener | reveal (`default-features = false`) |
| pdf_oxide | legacy-crypto (`default-features = false`; `icc` non attiva) |
| postcard | alloc, use-std (`default-features = false`) |
| proptest | bit-set, default, fork, regex-syntax, rusty-fork, std, tempfile, timeout |
| quick-xml | default (vuota); `encoding`, `encoding_rs`, `serde`, `serialize` sono attivate da altri crate dell'albero, non dal workspace |
| regex | default e tutte le feature `perf-*` e `unicode-*` che ne derivano |
| rfd | pollster, wayland, wayland-backend, wayland-client, wayland-protocols, xdg-portal (`default-features = false`) |
| rlimit | nessuna |
| rustix | std, fs, process, net dal workspace (`default-features = false`); risolte anche alloc, default, event, pipe, shm, system, time per altri crate dell'albero |
| serde | default, derive, serde_derive, std |
| sevenz-rust2 | deflate (`default-features = false`) |
| subtle | std (`default-features = false`) |
| sys-locale | nessuna |
| tar | nessuna (`default-features = false`) |
| tempfile | default, getrandom |
| thiserror | default, std |
| toml | display, parse, serde (`default-features = false`) |
| tracing | std (`default-features = false`) |
| tracing-appender | nessuna |
| tracing-log | log-tracer, std (`default-features = false`; `interest-cache` non attiva) |
| tracing-subscriber | alloc, fmt, registry, sharded-slab, std, thread_local (dal workspace `fmt` e `std` con `default-features = false`) |
| unicode-normalization | default, std |
| valuable | nessuna attiva nella build: nel lock solo sotto `cfg(tracing_unstable)` |
| windows-sys | dal workspace: Win32_Foundation, Win32_Graphics_Gdi, Win32_Security, Win32_Security_Authorization, Win32_Storage_FileSystem, Win32_System_IO, Win32_System_JobObjects, Win32_System_LibraryLoader, Win32_System_Pipes, Win32_System_SystemServices, Win32_System_Threading, Win32_UI_Accessibility, Win32_UI_Shell, Win32_UI_WindowsAndMessaging; nei test (dev) di engine e app anche Win32_System_ProcessStatus. Con l'unificazione l'albero Windows attiva molte altre feature (UI, Shell, COM, Storage) |
| winreg | nessuna |
| winresource | nessuna (`default-features = false`, quindi senza `toml`) |
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
- Windows e Linux, `blake3` 1.8.7 (`pure`): il `build.rs` usa `cc` (build-dependency) per sondare il compilatore su `-mavx512f` e `-mavx512vl`, anche con `pure`; con `pure` non compila sorgenti C e il binario non contiene C di BLAKE3. Il sondaggio è l'unico passo che tocca il compilatore C.
- Windows e Linux, `pdf_oxide` 0.3.78: nessun `cc` né crate `-sys` con C nel sottoalbero (verificato). `cc` compare nel sottoalbero solo su macOS (`core-foundation-sys`), che non è un target del progetto.
- Windows e Linux, `rlimit` 0.11.0 (Linux): `build.rs` solo probe di cfg; nessun C.
- Windows e Linux, `-sys` nell'albero del workspace: `dirs-sys`, `renderdoc-sys`, `windows-sys`, `linux-raw-sys`, `wayland-sys`, `yeslogic-fontconfig-sys`. Sono già trattati nelle sezioni precedenti; `windows-sys` e `linux-raw-sys` sono binding senza C. Nessuno è introdotto da `pdf_oxide`.
- `calamine`, `cfb`, `crc32fast`, `flate2`, `lz4_flex`, `mail-parser`, `postcard`, `quick-xml`, `sevenz-rust2`, `tar`, `zip`: `build = false` o `build.rs` senza C (`crc32fast` legge solo la versione di rustc). `zlib-rs`, `lzma-rust2`, `uuid` e `filetime` (transitivi di questi crate) non hanno `build.rs`.
- Windows e Linux, dipendenze della fase 7: `subtle`, `getrandom` 0.3.4, `rustix` 1.1.6, `winreg`, `tracing`, `tracing-subscriber` e `tracing-appender` non hanno `build.rs` con `cc` né codice C. `getrandom` e `rustix` hanno un `build.rs` che solo sonda rustc o i cfg. `winresource` 0.1.34 (build-dependency di `lynkeus-app`) non compila C: il suo `build.rs` invoca `rc.exe` del Windows SDK solo sul target Windows, mentre su Linux il crate si compila ma il suo `build.rs` non fa nulla.
- Solo `fuzz/` (fuori dal binario): `libfuzzer-sys` 0.4.13 → build script con `cc` compila il C++ di libFuzzer (`build.cpp(true)`). Non entra nell'albero di `lynkeus-app` né di `lynkeus-extract`.
- Windows e Linux, `tracing-log` 0.2.0 e `valuable` 0.1.1 (questa non compilata nella build normale): nessun `cc` né crate `-sys`; `valuable` ha un `build.rs` che imposta solo cfg per gli atomici.
- Windows e Linux, `arboard` 3.7.0: `build = false`, nessun `build.rs` e nessun `cc`; già nell'albero tramite `egui-winit`.
- Strumenti della CI (vedi sezione dedicata): nessun `build.rs` nei crate `cargo-auditable`, `cargo-cyclonedx` e `cargo-fuzz`; le loro dipendenze transitive non sono state risolte.

I crate nuovi di codice Rust puro (`encoding_rs`, `memchr`, `crossbeam-channel`, `chardetng`, `rfd` lato build script, `calamine`, `lz4_flex`, `postcard`, `quick-xml`, `zip`, `pdf_oxide`) non compilano C.

## Eccezioni agli advisory (`deny.toml`)

Data di revisione di tutte e quattro le eccezioni, come in `deny.toml`: **2027-05-01**. Dopo quella data `cargo xtask check` (passo `deny-review`) fallisce finché l'eccezione non si toglie o si rinnova con un motivo aggiornato.

Le prime tre sono advisory di tipo "unmaintained" (nessuna vulnerabilità nota) e arrivano su Linux da zbus 5. La quarta, RUSTSEC-2026-0192, arriva da `pdf_oxide` su tutti i target (Windows e Linux): vedi l'ultimo punto. Due percorsi: AccessKit, usato da eframe 0.37 tramite accesskit_unix 0.24.1 (ultima versione), e, dalla fase 6, la dipendenza diretta di `lynkeus-app` su zbus 5.19.0 per il portale XDG. Da rivedere quando AccessKit e `lynkeus-app` non usano più zbus 5 (la 6.2.1 è disponibile dal 2026-11-01), o a ogni aggiornamento di eframe.

- **RUSTSEC-2026-0370**: `zvariant` non mantenuto. Arriva da zbus 5 su Linux (diretto e via AccessKit) senza alternative. Revisione entro 2027-05-01.
- **RUSTSEC-2026-0371**: `zbus_names` non mantenuto. Arriva da zbus 5 su Linux (diretto e via AccessKit) senza alternative. Revisione entro 2027-05-01.
- **RUSTSEC-2026-0372**: `zvariant_derive` non mantenuto. Arriva da zbus 5 su Linux (diretto e via AccessKit) senza alternative. Revisione entro 2027-05-01.

- **RUSTSEC-2026-0192**: `ttf-parser` 0.25.1 non mantenuto. Arriva da `pdf_oxide` (ADR 0002), per i font incorporati nei PDF, su Windows e Linux, senza alternativa finché `pdf_oxide` non passa a `skrifa` (come prevede l'ADR). Solo "unmaintained", nessuna vulnerabilità nota. Il decoder PDF gira nel processo estrattore isolato (Job Object o `RLIMIT_AS`). Da rivedere a ogni aggiornamento di `pdf_oxide`. Il motivo è riportato in `deny.toml`. Prima della fase 4 questo advisory non compariva nell'albero: la feature `-notitle` di winit evita solo la catena di `winit`, non quella di `pdf_oxide`. Revisione entro 2027-05-01.

Nessun'altra eccezione: `exceptions = []` per le licenze.

## Ultimo `cargo deny check`

Eseguito il 2026-11-05 sul workspace dopo la registrazione della dev-dependency `windows-sys` (`Win32_System_ProcessStatus`) di engine e app: exit code 0, advisories, bans, licenses e sources ok, gli stessi 21 warning di duplicato. Esecuzione precedente, dopo la registrazione di `arboard` (dev), `winreg` e `winresource`: exit code 0, con advisories, bans, licenses e sources ok e gli stessi 21 warning di duplicato elencati sotto. Esecuzione precedente, dopo `tracing-log`, `valuable` (transitiva, non compilata) e le feature nuove di `windows-sys`: exit code 0. Verifica precedente della fase 7 (`subtle`, `getrandom`, `rustix`, `winreg`, `winresource`, `tracing`, `tracing-subscriber`, `tracing-appender`): exit code 0. Esito per sezione: advisories ok, bans ok, licenses ok, sources ok. Nessun warning di duplicato in più rispetto all'elenco sotto. Il `deny.toml` vieta `pdf-extract` e `lopdf` in `[bans]` (ADR 0002): divieto presente e attivo.

Crate `fuzz/` (workspace proprio, fuori dal `cargo deny check` della radice): `cargo deny check` dentro `fuzz/` con il `deny.toml` del progetto esce con codice 4. Advisories, bans e sources ok; `licenses` FAILED: `libfuzzer-sys` 0.4.13 ha licenza `(MIT OR Apache-2.0) AND NCSA` e NCSA non è in `allow`. Da decidere: aggiungere NCSA all'allowlist o sostituire il crate. `deny.toml` non è stato modificato.

Warning (non bloccanti, `multiple-versions = "warn"`, 21 voci): calloop, calloop-wayland-source, fax, foldhash, font-types, getrandom, hashbrown (3 versioni), linux-raw-sys, miniz_oxide, pollster (0.4.0 da rfd e 1.0.1 da eframe), quick-xml (0.41.0 da wayland-scanner, 0.42.0 nel progetto), read-fonts, rustc-hash, rustix, skrifa, smithay-client-toolkit, syn, thiserror (1.0.69 e 2.0.21), thiserror-impl, weezl, windows-sys (3 versioni: 0.52.0, 0.59.0, 0.61.2).

## Dipendenze dirette di fase 7

Registrate il 2026-11-05 in tabella: `subtle`, `getrandom`, `rustix`, `winreg`, `winresource`, `tracing`, `tracing-subscriber` e `tracing-appender`, più le feature nuove di `windows-sys` (`Win32_Security_Authorization`, `Win32_Storage_FileSystem`, `Win32_UI_WindowsAndMessaging`). Usi per target: `rustix` in `lynkeus-app` e `lynkeus-engine` solo su non Windows; `winreg` e `windows-sys` solo su Windows; `winresource` è una build-dependency di `lynkeus-app`. `lynkeus-engine` non usa le altre nuove dipendenze.

Nota sull'`unsafe` delle nuove dipendenze, verificato sui sorgenti scaricati:

- `rustix` 1.1.6: 1.672 occorrenze in `src`, quasi tutte nel backend `linux_raw` (istruzioni inline, attivo su Linux senza `use-libc`). Il codice del progetto che la usa (`platform/mod.rs`, `platform.rs` dell'engine) non contiene `unsafe`.
- `winreg` 0.56.0: 38 occorrenze in `src`, nelle chiamate a `advapi32` (`reg_key.rs`, 24) e nei `unsafe impl Send` di `RegKey`. Il progetto la usa solo nella voce del clic destro.
- `getrandom` 0.3.4: 93 occorrenze in `src`, nelle chiamate di sistema per piattaforma. Il progetto chiama solo `getrandom::fill`.
- `subtle` 2.6.1: 2 blocchi `unsafe` (`black_box` come barriera di ottimizzazione, cast di `Ordering`), con commento `SAFETY`. Nessuno nel progetto.
- `tracing` 0.1.44: 7 occorrenze in `src` (pin delle future in `instrument.rs`, `from_utf8_unchecked` in `lib.rs`, `unsafe impl Sync` in `span.rs`). `tracing-subscriber` 0.3.23: 28 occorrenze in 6 file, nei layer (`layer/`), nei filtri e in `reload.rs`. Il progetto non ha `unsafe` nel guscio dei log.
- `tracing-appender` 0.2.5, `winresource` 0.1.34: nessuna occorrenza.

Il `cargo deny check` del 2026-11-05 non segnala advisory né licenze fuori elenco per queste dipendenze; `getrandom` e `rustix` compaiono tra i duplicati già elencati (0.4.3 e 0.38.44 nel lock per altre dipendenze).

## Controllo del 2026-11-05 (dopo il commit 2958788)

Confronto con `git diff 2958788..HEAD` sui `Cargo.toml` del workspace (radice, `crates/*`, `xtask/`, `fuzz/`; `experiments/` non è nel workspace). L'unica dipendenza diretta nuova è `arboard` 3.7.0, dev-dependency di `lynkeus-app` (test `copy_200k`); era già nel `Cargo.lock` dal commit 2958788, tramite `egui-winit`. Il controllo ha anche rilevato `winreg` e `winresource`, dipendenze dirette già in uso ma assenti dalla tabella principale: la sezione della fase 7 le dava per registrate, ora hanno la riga. Le tre voci sono registrate sopra con lo stesso schema.

## Dipendenze dirette di fase 5

Registrate il 2026-11-04 in tabella: `cfb`, `flate2`, `mail-parser`, `sevenz-rust2` e `tar` (dipendenze di `lynkeus-extract`), e `libfuzzer-sys` (dipendenza di `fuzz/`, fuori dal workspace).

Nota sull'`unsafe` dei decoder di terzi (threat model, sezione 5), verificato sui sorgenti scaricati:

- `mail-parser` 1.0.1: 108 occorrenze in `src`. Su x86_64 (Windows e Linux) si compilano 82: `scan/x86.rs` e `scan/simd.rs`, scansione SIMD dei delimitatori, e `store/str.rs`, conversione UTF-8 senza controllo. `scan/neon.rs` (26) si compila solo su aarch64.
- `sevenz-rust2` 0.24.2: nessun blocco `unsafe` nel crate. Le 3 occorrenze sono nel testo dei messaggi di errore. Il `unsafe` del percorso 7z è nella dipendenza transitiva `lzma-rust2` 0.23.0 (Apache-2.0, feature `optimization` attiva da sevenz-rust2): 15 occorrenze in `src`, nel decoder (`range_dec.rs`, `lz/mod.rs`) e nei percorsi SIMD dell'encoder (`lz/lz_encoder.rs`). La 0.25.0 è più recente, ma non è dipendenza diretta.
- `flate2` 1.1.10: il backend attivo è zlib-rs 0.6.8 (Rust, licenza Zlib), con 456 occorrenze di `unsafe` in 30 file. Non ha `build.rs`.
- Il threat model (`docs/threat-model.md`, righe 171 e 179) dice che `sevenz-rust2` contiene `unsafe` nel proprio codice. Per la 0.24.2 non è così: il `unsafe` è in `lzma-rust2`. Il threat model va corretto; questo registro non lo modifica.
