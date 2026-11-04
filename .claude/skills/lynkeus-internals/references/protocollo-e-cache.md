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

`settings.toml` proprio, versionato, con i campi sconosciuti conservati. Scrittura durevole: file temporaneo, `sync_all`, rinomina nello stesso volume. eframe ricorda solo la posizione della finestra.
