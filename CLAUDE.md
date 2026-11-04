# Lynkeus Searcher: istruzioni del progetto

Solo ciò che è specifico di questo progetto. Le decisioni stanno in `PLAN.md`.

## Lingue

- Interfaccia in italiano e inglese (Fluent, `locales/it` e `locales/en`).
- Identificatori nel codice in inglese; commenti, documenti e messaggi di commit in italiano.

## Comandi

- `cargo xtask check`: fmt, clippy `-Dwarnings`, test, `cargo deny check`.
- `cargo xtask check-linux`: controllo di compilazione per il target Linux.
- `cargo xtask dist`: build di rilascio, verifiche, zip e `SHA256SUMS`.

## Documenti di stato e decisioni

- `PLAN.md`: piano e checklist delle fasi; si spunta una voce solo se completata e verificata.
- `docs/`: `specifica.md`, `threat-model.md`, `dipendenze.md`, `adr/` (decisioni difficili da cambiare).

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

## Eccezioni alle regole generali

1. **Firma degli eseguibili** (permanente). Eccezione alla regola che chiede la firma SHA-256 con marca temporale di ogni EXE distribuito.
   - Motivo: progetto personale senza certificato di firma del codice.
   - Compensazione: si pubblica `SHA256SUMS` accanto allo zip e il README spiega l'avviso di SmartScreen e la verifica dell'hash.
   - Il resto della regola vale per intero.
