# Protocollo del processo estrattore, cache e impostazioni

Stato: solo ciò che è deciso. Il dettaglio dei messaggi si scriverà nella fase 4.

## Processo estrattore

- Stesso eseguibile con l'opzione nascosta `--extract-worker`, avviato dal percorso assoluto di `current_exe()`, con `env_clear` (salvo poche variabili di sistema) e nessun handle ereditato oltre stdin e stdout.
- Messaggi `postcard` con lunghezza in testa e dimensione massima, su stdin e stdout.
- Windows: Job Object (`win32job`) con limite di memoria e `KILL_ON_JOB_CLOSE`. Linux: `RLIMIT_AS` (`rlimit`) impostato dal figlio, che esce quando il suo stdin si chiude.
- Stop: chiusura dello stdin, poi terminazione dopo 500 ms; oltre il timeout il processo è terminato subito.
- Estrattori: `zip` + `quick-xml` (DTD rifiutati), `calamine`, `pdf_oxide` (ripiego `pdf-extract`, ADR in fase 4), `cfb` con parser propri per .doc e .ppt, tokenizer RTF proprio, `mail-parser`, `sevenz-rust2`, `tar`, `flate2`.

## Cache

- Posizione: `%LOCALAPPDATA%\Lynkeus Searcher\cache` su Windows, `~/.cache/lynkeus-searcher` su Linux (crate `directories`).
- Chiave: `blake3` di percorso, dimensione, data di modifica, versione degli estrattori e limiti.
- Contenuto: gli stessi messaggi del protocollo, compressi con `lz4_flex`, con intestazione di versione e CRC.
- Scrittura: file temporaneo nella stessa cartella, poi rinomina; nessun flush (derivato rigenerabile). Voce corrotta: ignorata e cancellata.
- Pulizia: LRU a 500 MB; spegnibile e svuotabile dalle Impostazioni.

## Cartelle private

Dati, cache, log, rapporti di crash e temporanea delle voci degli archivi. Windows: descrittore esplicito (solo utente e SYSTEM). Linux: 0700 per le cartelle, 0600 per i file. Creazione dei file che fallisce se esistono già.

## Impostazioni

`settings.toml` nella cartella di configurazione (`crates/lynkeus-app/src/settings.rs`). eframe ricorda solo la posizione della finestra.

Formato, versione 1 (`format_version = 1`), chiavi stabili e valori come testo leggibile:

- radice: `language` ("system", "it", "en"), `theme` ("system", "light", "dark");
- `[search]`: `case` ("insensitive", "sensitive"), `accents` ("distinct", "ignored"), `wildcards` (bool), `combine` ("any", "all"), `roots`, `terms`, `recent_roots`, `recent_terms`;
- `[filters]`: `extension_mode` ("all", "include", "exclude"), `extensions`, `skip_dirs`, `include_hidden`, `include_online_only`, `limit_file_size`, `max_file_size_mib`;
- `[layout]`: `search_panel_width`, `search_panel_visible`, `column_word`, `column_term`, `column_file`, `column_position`, `sort_column` ("none", "word", "term", "file", "position", "context"), `sort_descending`.

Lettura tollerante: non fallisce mai. Ogni campo si legge da solo; un valore mancante o del tipo sbagliato prende il predefinito senza perdere gli altri; i numeri fuori intervallo si riportano ai limiti (larghezza colonne 40-2000, pannello 260-560); le estensioni non valide e i percorsi vuoti si scartano. `skip_dirs` assente = predefinite, presente ma vuoto = nessuna.

Campi sconosciuti (di versioni future o scritti a mano) conservati: il documento letto si tiene in memoria e a ogni salvataggio le tabelle si fondono chiave per chiave con quelle scritte dal programma.

Casi del file:

- assente: predefinite;
- non TOML: spostato in `settings.toml.invalid` (il file nuovo si scrive al primo salvataggio); se lo spostamento fallisce il file non si riscrive;
- `format_version` maggiore di 1: letto per quanto possibile, mai riscritto (`SettingsError::NewerVersion`);
- non leggibile (permessi, UTF-8 non valido, oltre 1 MiB): predefinite, mai riscritto (`SettingsError::NotLoaded`).

Scrittura durevole (`write_durably`): file temporaneo nuovo (`create_new`) nella stessa cartella, `sync_all`, rinomina sopra il vecchio, flush della cartella. I percorsi non UTF-8 non si salvano.
