# Lynkeus Searcher: istruzioni del progetto

Solo ciò che è specifico di questo progetto. Le decisioni stanno in `PLAN.md`.

## Lingue

- Interfaccia in italiano e inglese (Fluent, `locales/it` e `locales/en`).
- In inglese (regola generale del 2026-11-07): identificatori, commenti, test, messaggi di commit, nomi dei rami, PR, `README`, `docs/`, `PLAN.md`, `SECURITY.md`. In italiano solo questo file, la skill del progetto (`lynkeus-internals`) e le istruzioni degli agenti (`.claude/agents/`).
- Migrazione in corso: commenti e documenti scritti prima del 2026-11-07 sono ancora in italiano (circa 160 file di codice e 15 documenti). Un file che si modifica si traduce nello stesso ramo, in un commit `tidy:` separato; `docs/` e `PLAN.md` si traducono in un blocco di lavoro dedicato. I dati di prova (`fixtures/`) restano nella loro lingua: sono input dei test.

## Comandi

- `cargo xtask check`: fmt, clippy `-Dwarnings`, test, `cargo deny check`.
- `cargo xtask check-linux`: controllo di compilazione per il target Linux.
- `cargo xtask dist`: build di rilascio, verifiche, zip e `SHA256SUMS`.
- `cargo xtask release`: come `dist`, con `cargo auditable` e SBOM (usato dalla CI di rilascio).
- `cargo xtask test-plan [--quick] [--level 1,2,3,4,5]`: piano di test automatico, rapporto in `target/test-plan/`. `--clipboard` (sovrascrive gli appunti) e `--registry` (scrive nel registro) solo su richiesta.
- Build e test uno alla volta, con `CARGO_BUILD_JOBS=6` e `RUST_TEST_THREADS=6`: build parallele in release hanno saturato la memoria della macchina.

## `unsafe`

- `#![forbid(unsafe_code)]` in core ed engine; nell'app `#![deny(unsafe_code)]` con un solo modulo ammesso, `crates/lynkeus-app/src/platform/windows_ffi.rs`.
- In extract `#![deny(unsafe_code)]` con una sola eccezione: `unsafe fn close_inherited_descriptors` (`crates/lynkeus-extract/src/platform.rs`, esposta da `worker`), che su Linux chiude i descrittori ereditati oltre 0, 1 e 2. È `unsafe` perché chiude descrittori per numero: si chiama solo all'inizio del `main` di un processo estrattore (`extract_worker` in `crates/lynkeus-app/src/main.rs` e il processo di prova del motore), mai da `worker::run`, che gira anche in un thread dei test.
- Nei test, solo `crates/lynkeus-engine/tests/misura/mod.rs` (una chiamata a `K32GetProcessMemoryInfo` per la memoria di picco), con `// SAFETY:` e interfaccia safe.

## Documenti di stato e decisioni

- `PLAN.md`: piano e checklist delle fasi; si spunta una voce solo se completata e verificata.
- `docs/`: `specifica.md`, `threat-model.md`, `dipendenze.md`, `adr/` (decisioni difficili da cambiare).
- `docs/analisi-conformita.md`: analisi di conformità del 2026-11-05 (problemi per area con ID, priorità di correzione).
- `docs/piano-di-test.md`: piano di test a cinque livelli, matrice problema → test, registro delle esecuzioni.

## Struttura del workspace

Dipendenze in una sola direzione: `app → engine → extract → core`.

- `crates/lynkeus-core`: ricerca pura, senza I/O.
- `crates/lynkeus-extract`: decodifica, estrattori, archivi, processo estrattore e protocollo.
- `crates/lynkeus-engine`: scansione, pool, cache, flusso dei risultati.
- `crates/lynkeus-app`: binario `lynkeus-searcher`, UI egui, impostazioni, integrazione col sistema.
- `xtask`: comandi di controllo e rilascio.

## Skill

- Sempre: `software-design-core`, `rust-egui-desktop-design`, `ui-design-core`.
- Per argomento:
  - `security-design`: estrattori, archivi, IPC, registro, rilascio.
  - `windows-system-design`: job, pipe, registro, file e cartelle private.
  - `durable-data-design`: cache, impostazioni, esportazione.
- Skill del progetto: `lynkeus-internals` (semantica della ricerca, limiti, protocollo e cache).

## Squadra di agenti (`.claude/agents/`)

- `verificatore` (haiku): esegue i controlli e riassume gli errori; non modifica file.
- `riparatore-lint` (sonnet): corregge in modo meccanico fmt, clippy ed errori banali senza cambiare il comportamento.
- `generatore-fixture` (sonnet): crea i file di prova (codifiche, Office/ODF, PDF, archivi, e-mail, EPUB, casi malevoli) da sorgenti e script riproducibili.
- `traduttore-fluent` (sonnet): allinea `locales/it` e `locales/en` ed esegue il test delle lingue.
- `controllore-conformita` (haiku): controlla licenza, intestazioni, autore dei commit e assenza di dati personali nei file e negli eseguibili; solo lettura.
- `registro-dipendenze` (haiku): registra in `docs/dipendenze.md` la validazione di ogni dipendenza diretta ed esegue `cargo deny check`.
- `redattore-documenti` (sonnet): scrive e aggiorna README, CLAUDE.md, `docs/` e la skill `lynkeus-internals`.
- `sviluppatore-rust` (opus): implementa una parte delimitata di una fase, nei crate e file indicati.
- `revisore-codice` (opus): rivede un diff o un ramo rispetto al piano e alle skill; solo lettura.

Orchestrazione: si lavora in parallelo solo su file che non si sovrappongono; i lavori paralleli sul codice usano un worktree ciascuno.

## GitHub

- Repository `MR1999develop/Lynkeus`, pubblico. Il ruleset di `main` vieta push diretti, force push e cancellazione: ogni modifica passa da un ramo e da una PR, unita con un merge commit quando `check (windows-latest)` è verde, senza eccezioni.
- CI dei rami: `ci.yml` parte da sola solo su `main` e sulle PR. Passati i controlli locali, l'agente pubblica il ramo di lavoro e avvia la CI con il comando delle regole generali e `--push --avvia ci.yml`. Eccezione alla regola generale che unisce il ramo a `main` in locale: qui `main` cambia solo con una PR (ruleset), che l'utente apre e unisce quando la CI del ramo è verde; il blocco successivo parte da quel ramo se ne dipende, altrimenti da `main`.
- Ubuntu è sospeso nella CI, nel rilascio e nel piano di test (decisione del 2026-11-06), finché i test Linux non passano: la PR #1 ha corretto i primi errori, il resto è da verificare. Per riattivarlo si rimette `ubuntu-latest` nelle matrici e `check (ubuntu-latest)` nel ruleset di `main`.
- Rilascio: tag `v<versione di Cargo.toml>`; `release.yml` crea una bozza, pubblicata a mano dopo il controllo. I tag `v*` non si spostano né si cancellano e le release pubblicate sono immutabili.

## Eccezioni alle regole generali

1. **Firma degli eseguibili** (permanente). Eccezione alla regola che chiede la firma SHA-256 con marca temporale di ogni EXE distribuito.
   - Motivo: progetto personale senza certificato di firma del codice.
   - Compensazione: si pubblica `SHA256SUMS` accanto allo zip e il README spiega l'avviso di SmartScreen e la verifica dell'hash.
   - Il resto della regola vale per intero.
