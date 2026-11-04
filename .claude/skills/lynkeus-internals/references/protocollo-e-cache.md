# Protocollo del processo estrattore, cache e impostazioni

Fonti: `crates/lynkeus-extract` (`protocol.rs`, `event.rs`, `worker.rs`, `format.rs`) e `crates/lynkeus-engine` (`launcher.rs`, `pool.rs`, `cache.rs`, `documents.rs`, `docsearch.rs`). Se cambiano, vale il codice.

## Frame

```
[u32 LE lunghezza][u16 LE versione][corpo postcard]
```

- La lunghezza conta versione + corpo: da 2 a `MAX_FRAME_BYTES` (16 MiB). `PROTOCOL_VERSION` = 1.
- Il lettore (`FrameReader`) controlla la lunghezza prima di allocare, poi la versione, poi decodifica il corpo in un tipo concreto e rifiuta i byte in più. Fine del flusso prima del primo byte di un frame = fine normale (`Ok(None)`); a metà frame = `ProtocolError::Truncated`. Ogni altra violazione chiude il canale.
- Lo scrittore (`encode_frame`, `FrameWriter`) non scrive un messaggio oltre il massimo (`FrameTooLarge`).
- Il percorso di `ExtractRequest` viaggia come `OsString` (anche non UTF-8).
- Si cambia un tipo del protocollo (messaggi, `ExtractEvent`, `WarningKind`, `SkipReason`, `Format`) solo aggiungendo varianti in fondo; ogni altro cambio alza `PROTOCOL_VERSION`, che fa parte anche della chiave della cache.

## Messaggi e sequenza

- `Request::Extract(ExtractRequest { id: u64, path, options: ExtractOptions })`; l'`id` lo sceglie il padre.
- `Response`: `Event { id, event }`, `Done { id }`, `Rejected { reason }` (`RejectReason`: `VersionMismatch { found }`, `FrameTooLarge`, `Malformed`; dopo un `Rejected` il figlio esce).
- Nessun messaggio dal figlio senza una richiesta. Per ogni richiesta: zero o più `Event` con lo stesso `id`, poi `Done`. I problemi del file sono eventi, non errori del protocollo.
- `ExtractEvent`: `DocumentStart { inner, format }`, `Unit(TextUnit)`, `DocumentEnd`, `Warning { inner, kind }`, `Skipped { inner, reason }`. `inner` = nomi delle voci d'archivio dall'esterno all'interno (vuoto per il file stesso); sono etichette con caratteri di controllo e bidi sostituiti da U+FFFD.
- Sequenza (`SequenceCheck`, usata dal pool e dalla cache): ogni documento è una coppia `DocumentStart` ... `DocumentEnd` mai annidata, con solo `Unit` in mezzo; un archivio non è un documento (lo è ogni sua voce); `Warning` e `Skipped` stanno fuori dalle coppie e portano il proprio `inner`; un avviso può seguire un documento già chiuso (PDF con pagina danneggiata); un documento che fallisce prima del testo dà solo l'avviso. Errori: `NestedDocument`, `OutsideDocument`, `NoticeInsideDocument`, `Unclosed` (a `finish`).
- Più righe in un'unità: per `Location::Line` e `PageLine` le righe seguenti hanno numeri successivi (`line_location`); per le altre posizioni la posizione è la stessa.
- `WarningKind` (estrattore): `NotFound`, `AccessDenied`, `InUse`, `Io`, `Encrypted`, `Corrupt`, `Timeout` (lo decide solo il padre), `LimitExceeded(LimitKind)`, `ContentMismatch`, `NoText`. `SkipReason`: `Binary`, `TooLarge`, `SpecialEntry`. L'engine aggiunge `ExtractorUnavailable` ai propri avvisi.

## Processo estrattore (worker)

- Stesso eseguibile, opzione nascosta `--extract-worker` seguita da `--memory-limit-bytes <n>` (`WorkerConfig`). Il valore deve essere numerico e almeno 64 MiB (`MIN_MEMORY_LIMIT`); altrimenti il figlio non parte. Limite proposto: 1 GiB (`DEFAULT_MEMORY_LIMIT`, usato da `WORKER_MEMORY_LIMIT` nell'app).
- Ciclo (`worker::run`): imposta il limite di memoria prima di leggere qualunque file (Linux: `RLIMIT_AS` dal figlio; Windows: lo impone il Job Object del padre) e se non ci riesce esce con errore; per ogni richiesta manda gli eventi man mano (flush a ogni `DocumentEnd` e alla fine); esce con successo quando stdin finisce tra due richieste, con errore su frame non valido (dopo `Rejected`) o se stdout non si scrive più.
- Stdout riservato al protocollo, nessun `print!` nel crate; stderr nullo. Il figlio legge solo i file richiesti, non scrive su disco, non avvia programmi, non usa la rete.
- Avvio (`WorkerLauncher`, trait nell'engine): percorso assoluto (un percorso relativo è rifiutato), cartella corrente = cartella dell'eseguibile. `CommandLauncher` (Linux e test): `std::process::Command`, `env_clear` salvo `KEPT_ENV_VARS` (`SystemRoot` su Windows, nessuna su Linux). Windows: `JobLauncher` dell'app (vedi sotto).
- Windows (`crates/lynkeus-app/src/platform/windows_ffi.rs`, unico modulo con `unsafe`): un Job Object per processo con `KILL_ON_JOB_CLOSE`, `PROCESS_MEMORY` e `DIE_ON_UNHANDLED_EXCEPTION`; porta di completamento associata al job per riconoscere `JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT`; `CreateProcessW` con `PROC_THREAD_ATTRIBUTE_JOB_LIST` (il processo nasce già nel job) e `HANDLE_LIST` (ereditati solo stdin e stdout), ambiente con la sola `SystemRoot`, senza console. Dopo l'avvio `IsProcessInJob` deve dare vero, altrimenti il processo è terminato e l'avvio fallisce. `win32job` non è usato. `WorkerExit::MemoryExceeded` = messaggio del job oppure picco entro 1/16 dal limite.
- `WorkerExit`: `Success`, `MemoryExceeded`, `Failed { code }`. Su Linux una memoria superata non si distingue da un aborto (`Failed`).

## Pool (`ExtractorPool`)

- Dimensione `min(core/2, 4)` (`default_pool_size`, `MAX_POOL_SIZE` 4), processi avviati al primo documento, un file alla volta per processo. Un thread per processo legge lo stdout (coda di 16 messaggi) e chi chiama aspetta con scadenza.
- Timeout per file (predefinito 60 s, `Limits::extract_timeout`): processo terminato subito e sostituito, `WorkerFailure::Timeout`.
- Annulla, o destinatario che chiede di fermarsi: stdin chiuso; dopo `STOP_GRACE` (500 ms) il processo è terminato. `shutdown` chiude insieme i processi liberi con un'unica attesa.
- Violazioni (`WorkerFailure::ProtocolViolation`, processo terminato): frame non valido (anche byte estranei su stdout), `id` diverso, `Rejected`, sequenza sbagliata, stdout chiuso con uscita pulita a metà file. Morte a metà file: `Died(WorkerExit)`. Nessun processo avviabile o pool chiuso: `Unavailable`. Avvisi: `Timeout`; `Died(MemoryExceeded)` = `LimitExceeded(Memory)`; le altre morti e le violazioni = `Corrupt`; `Unavailable` = `ExtractorUnavailable`.
- Un processo libero che ha messaggi in sospeso o è morto non si riusa. Se l'invio della richiesta fallisce si riprova una volta con un altro processo (`SEND_ATTEMPTS` 2).
- Con `Failed` gli eventi già consegnati restano validi (documenti completi di un archivio), l'ultimo documento aperto no.

## Cache

- Cartella: `%LOCALAPPDATA%\Lynkeus Searcher\cache` su Windows, `~/.cache/lynkeus-searcher` su Linux (crate `directories`). Voci: `<chiave in esadecimale>.lkc`; temporanei `.tmp`.
- Chiave (`CacheKey`): `blake3` di versione del formato della cache, `PROTOCOL_VERSION`, versione del programma (`CARGO_PKG_VERSION`, che fissa quella degli estrattori), percorso, dimensione, data di modifica (secondi e nanosecondi), opzioni d'estrazione (limiti, codificati come nel protocollo). Senza data di modifica il file non va in cache.
- Intestazione (44 byte, `CACHE_FORMAT_VERSION` 1): magia `LKC\0` (4), formato u16 LE, protocollo u16 LE, chiave completa (32), byte originali u32 LE (al più 64 MiB), byte compressi u32 LE, CRC32 del contenuto compresso u32 LE. Contenuto: `lz4_flex` a blocchi dei frame del protocollo, uno per `ExtractEvent`.
- Lettura, in quest'ordine: dimensione, magia, versioni, chiave, lunghezze (prima di allocare), CRC, decompressione esatta, frame, `SequenceCheck`. Qualsiasi fallimento = voce corrotta: ignorata e cancellata.
- Scrittura: temporaneo creato nuovo (`create_new_private_file`, mai sopra un file esistente) nella stessa cartella, poi rinomina; nessun `fsync`. La cartella si crea privata al primo salvataggio.
- LRU: 500 MB predefiniti (`DEFAULT_CACHE_SIZE`); l'ordine è la data di modifica della voce, aggiornata a ogni lettura (sopravvive al riavvio). Voce massima: 64 MiB di frame decompressi (`MAX_ENTRY_BYTES`); una voce più grande della cache o del massimo non si salva (non è un errore). Temporanei più vecchi di un'ora cancellati alla scansione. `set_max_size` sfratta subito; `clear` cancella voci e temporanei.
- Uso (`DocumentSource`): prima la cache, poi il pool; si salva solo se l'estrazione è `Completed` e senza avvisi passeggeri (file in uso, accesso negato, errori di lettura). Il pool non conosce la cache.
- File di testo e voci: i file con estensione fuori dal registro si leggono nel processo principale (non nel pool); le voci d'archivio senza formato si leggono come testo nell'estrattore.

## Registro dei formati e come aggiungerne uno

`crates/lynkeus-extract/src/format.rs`. `REGISTRY` lega estensioni (minuscole, senza punto), firma attesa (`Signature`: `Zip`, `Pdf`, `Cfb`, `Rtf`, `SevenZ`, `Gzip`, `Tar`, `Html`, `Unknown`; `None` = qualsiasi contenuto, come Text ed Eml), tipo (`Document` o `Archive`) e funzione d'estrazione. `Format::for_path`, `for_name` (separatori `/` e `\`, conta l'ultima estensione), `for_extension`; `None` = nessun formato (mai provato "per vedere"). `Signature::sniff` guarda i primi 1024 byte. `verify` decide: firma attesa = si legge; OOXML con firma CFB = candidato cifrato; altrimenti la tabella `HARMLESS`; altrimenti `ContentMismatch`.

Per aggiungere un formato:

1. variante in fondo a `Format` (le esistenti non si spostano; se cambia il significato di una, si alza `PROTOCOL_VERSION`);
2. se serve, variante di `Signature` e riconoscimento in `Signature::sniff`;
3. riga in `REGISTRY` con la funzione d'estrazione in un modulo nuovo di `extract/`;
4. righe in `HARMLESS` per i casi innocui (formato dell'estensione, firma trovata, formato da usare). Ora: `(Doc, Rtf, Rtf)`, `(Xls, Html, Html)`, `(Xls, Unknown, Text)`. `Html` non ha estensioni: si raggiunge solo da `HARMLESS`;
5. test del registro (una sola riga per formato, estensioni uniche e minuscole) e fixture in `fixtures/formati/`.

Registrati (indice `postcard` in ordine): Text (voci d'archivio), Docx, Xlsx, Pptx, Odt, Ods, Odp, Pdf, Zip, Doc (`doc`, `dot`), Xls (`xls`, `xlt`), Ppt (`ppt`, `pot`, `pps`), Rtf, Eml (nessuna firma), Epub (archivio, firma Zip), SevenZ, Tar, Gz (`gz`, `tgz`; archivio), Html (solo caso innocuo). Archivi (`Kind::Archive`): Zip, Epub, SevenZ, Tar, Gz; gli altri sono documenti. Un `.gz` il cui nome interno termina in `.tar` (o `.tgz`), o il cui contenuto ha la firma tar, è un solo archivio con le voci del tar sotto il file. Gli allegati EML sono documenti a sé, dopo la fine del messaggio. Moduli in `extract/`: `word97`, `ppt`, `cfbfile` (file composto, condiviso), `sheet` (xlsx, xls, ods), `rtf`, `eml`, `html`, `epub`, `sevenz`, `tarball`.

## Cartelle private

Dati, cache, log, rapporti di crash e temporanea delle voci degli archivi (`%LOCALAPPDATA%\Lynkeus Searcher\data\temp` su Windows, `~/.local/share/lynkeus-searcher/temp` su Linux; svuotata all'avvio e all'uscita). Obiettivo Windows: descrittore esplicito (solo utente e SYSTEM). Linux: 0700 per le cartelle, 0600 per i file. Creazione dei file che fallisce se esistono già. Stato del codice: su Linux i permessi ci sono; su Windows `create_private_dir` eredita i permessi della cartella dell'utente e il descrittore esplicito è rimandato alla fase 7.

Voci d'archivio da aprire (`OpenedEntries`, app): solo le voci di testo, ricomposte dalle unità in un `.txt` con nome generato (al più 64 MiB). Per le voci in formato documento servono i byte originali, che il protocollo non porta: messaggio di non disponibilità.

## Impostazioni

`settings.toml` nella cartella di configurazione (`crates/lynkeus-app/src/settings.rs`). eframe ricorda solo la posizione della finestra.

Formato, versione 1 (`format_version = 1`), chiavi stabili e valori come testo leggibile:

- radice: `language` ("system", "it", "en"), `theme` ("system", "light", "dark");
- `[search]`: `case` ("insensitive", "sensitive"), `accents` ("distinct", "ignored"), `wildcards` (bool), `combine` ("any", "all"), `roots`, `terms`, `recent_roots`, `recent_terms`;
- `[filters]`: `extension_mode` ("all", "include", "exclude"), `extensions`, `skip_dirs`, `include_hidden`, `include_online_only`, `limit_file_size`, `max_file_size_mib`;
- `[layout]`: `search_panel_width`, `search_panel_visible`, `column_word`, `column_term`, `column_file`, `column_position`, `sort_column` ("none", "word", "term", "file", "position", "context"), `sort_descending`, `preview_visible` (bool, predefinito vero), `preview_height` (altezza dell'anteprima, 80-900, predefinita 220);
- `[export]`: `format` ("csv", "txt"; altro = "csv"), `scope` ("visible", "selected"; altro = "visible"), `last_dir` (ultima cartella di salvataggio; assente se non c'è, scartata se vuota o troppo lunga; un percorso non UTF-8 non si salva);
- `[cache]`: `enabled` (bool, predefinito vero), `max_size_mib` (intero, predefinito 500, riportato a 16-65.536; il menu Strumenti > Cache propone 100, 250, 500, 1000 e 2000 MiB).

Lettura tollerante: non fallisce mai. Ogni campo si legge da solo; un valore mancante o del tipo sbagliato prende il predefinito senza perdere gli altri; i numeri fuori intervallo si riportano ai limiti (larghezza colonne 40-2000, pannello 260-560, anteprima 80-900, cache 16-65.536 MiB); le estensioni non valide e i percorsi vuoti si scartano. `skip_dirs` assente = predefinite, presente ma vuoto = nessuna.

Campi sconosciuti (di versioni future o scritti a mano) conservati: il documento letto si tiene in memoria e a ogni salvataggio le tabelle si fondono chiave per chiave con quelle scritte dal programma.

Casi del file:

- assente: predefinite;
- non TOML: spostato in `settings.toml.invalid` (il file nuovo si scrive al primo salvataggio); se lo spostamento fallisce il file non si riscrive;
- `format_version` maggiore di 1: letto per quanto possibile, mai riscritto (`SettingsError::NewerVersion`);
- non leggibile (permessi, UTF-8 non valido, oltre 1 MiB): predefinite, mai riscritto (`SettingsError::NotLoaded`).

Scrittura durevole (`write_durably`): file temporaneo nuovo (`create_new`) nella stessa cartella, `sync_all`, rinomina sopra il vecchio, flush della cartella. I percorsi non UTF-8 non si salvano.
