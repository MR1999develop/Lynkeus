# Threat model (prima versione)

Metodo: le Quattro Domande e STRIDE. Il processo estrattore e gli archivi sono approfonditi (fase 4, sezione 4.7); la pipe dell'istanza unica, il registro e le cartelle private sono approfonditi nella fase 7 (sezione 4.8).

## 1. Le Quattro Domande

1. **Cosa stiamo costruendo?** Un'app desktop che legge file e archivi scelti dall'utente, ne estrae il testo (parte in un processo figlio isolato), cerca e mostra i risultati; conserva una cache del testo, impostazioni e, a richiesta, integra una voce nel clic destro.
2. **Cosa può andare storto?** Vedi sezione 4.
3. **Cosa facciamo al riguardo?** Mitigazioni elencate per ogni confine nella sezione 4.
4. **Abbiamo fatto un buon lavoro?** Si verifica con i test di robustezza (file malevoli, fuzzing dei parser propri), la verifica delle ACL e dei processi figli, la ricerca di dati personali nell'eseguibile; l'esito si registra qui a ogni fase.

## 2. Asset

- **Documenti dell'utente e il loro testo nella cache:** riservatezza.
- **Integrità del sistema dell'utente:** il programma non deve eseguire codice o scrivere fuori dalle sue cartelle a causa di un file letto.
- **Disponibilità dell'app:** un file ostile non deve bloccarla né esaurire memoria, disco o CPU.

## 3. Trust boundary

1. File e archivi letti (input non fidato).
2. Processo estrattore (figlio che decodifica i formati complessi).
3. Pipe (Windows) o socket (Linux) dell'istanza unica.
4. Registro di Windows e script Nautilus (voce del clic destro).
5. Cartella dati e cache (altri utenti o processi dello stesso computer).
6. File esportati (CSV/TXT letti poi da altri programmi).

## 4. STRIDE per confine

### 4.1 File e archivi letti (sintesi; dettaglio in 4.7)

- **S**: un file finge un'estensione. Mitigazione: formato verificato dal contenuto; incongruenze negli avvisi.
- **T**: nomi di voci con `..` o assoluti. Mitigazione: i nomi sono solo etichette, mai usati per scrivere su disco.
- **R**: non rilevante.
- **I**: nessuna uscita in rete; i risultati restano locali.
- **D**: zip bomb, archivi annidati, file enormi, regex costose. Mitigazione: limiti su profondità, voci, byte decompressi (`Read::take`) e rapporto 200:1; regex lineare del crate `regex`; lettura a blocchi; timeout.
- **E**: bug nei decoder. Mitigazione: decoder in un processo isolato (4.2), `forbid(unsafe_code)` nei crate di dominio, DTD XML rifiutati.

### 4.2 Processo estrattore (sintesi; dettaglio in 4.7)

- **S**: avvio di un eseguibile diverso. Mitigazione: percorso assoluto di `current_exe()`.
- **T**: messaggi alterati sul canale stdin/stdout. Mitigazione: protocollo `postcard` con lunghezza in testa e dimensione massima.
- **I**: ambiente e handle ereditati. Mitigazione: `env_clear` salvo poche variabili di sistema, nessun handle oltre stdin e stdout.
- **D**: processo bloccato o vorace. Mitigazione: timeout di 60 s, Job Object con limite di memoria e `KILL_ON_JOB_CLOSE` (Windows), `RLIMIT_AS` (Linux), uscita alla chiusura dello stdin e, su Linux, alla morte del padre (`--parent-pid`, vedi D), pool limitato a `min(core/2, 4)`.
- **E**: compromissione del figlio. Mitigazione: risorse limitate e figlio senza scritture sui dati dell'utente; su Windows il job ha `ActiveProcessLimit` 1 (il figlio non avvia altri programmi) e restrizioni dell'interfaccia (appunti, atomi globali, handle di finestre altrui, impostazioni di sistema e dello schermo, desktop, uscita da Windows: `WORKER_UI_RESTRICTIONS`, `windows_ffi.rs`); isolamento più forte da valutare nell'ADR.
- **E**: DLL piazzata accanto all'eseguibile e caricata prima di `main`. Mitigazione: `/DEPENDENTLOADFLAG:0x800` (le DLL importate si cercano solo in System32) e runtime C statico (`+crt-static`), in `.cargo/config.toml`; `cargo xtask dist` legge dal PE le opzioni e prova l'eseguibile con DLL finte accanto (`xtask/src/pe.rs`). `SetDefaultDllDirectories` in `main` copre solo i caricamenti successivi.

### 4.3 Pipe e socket dell'istanza unica (sintesi; dettaglio in 4.8)

- **S**: processo ostile che si spaccia per la prima istanza o per un client. Mitigazione: pipe con nome casuale creata con `FILE_FLAG_FIRST_PIPE_INSTANCE`; client con `SECURITY_IDENTIFICATION`; token casuale da 128 bit in un file privato, confronto in tempo costante.
- **T/E**: percorsi malevoli iniettati. Mitigazione: dimensione massima dei messaggi, solo percorsi esistenti, aggiunti all'elenco senza avviare la ricerca.
- **I/D**: altri utenti sul computer. Mitigazione: DACL esplicita solo per l'utente corrente (Windows), socket 0600 in `$XDG_RUNTIME_DIR` (Linux); attivo solo con `--add`.

### 4.4 Registro e script Nautilus (sintesi; dettaglio in 4.8)

- **T/E**: voce del registro che punta a un eseguibile modificato. Mitigazione: scrittura solo in `HKCU` (nessun amministratore); percorso dell'eseguibile aggiornato all'avvio e voce rimossa quando l'opzione è spenta.
- **T**: percorsi con caratteri speciali nella riga di comando. Mitigazione: argomento `"%1"` tra virgolette, trattato come dato.

### 4.5 Cartella dati e cache (sintesi; dettaglio in 4.8, parte C)

- **I**: la cache contiene testo dei documenti. Mitigazione: cartelle private (descrittore esplicito solo utente e SYSTEM su Windows; 0700 e 0600 su Linux), cache spegnibile e svuotabile, avviso nel README.
- **T**: voci di cache alterate o corrotte. Mitigazione: intestazione con versione e CRC; voce non valida ignorata e cancellata; creazione dei file che fallisce se esistono.
- **D**: crescita illimitata. Mitigazione: limite LRU di 500 MB.

### 4.6 File esportati

- **T/E**: iniezione di formule nei CSV aperti con un foglio di calcolo (testo che inizia con `=`, `+`, `-`, `@`, tabulazione o CR). Mitigazione (`crates/lynkeus-app/src/export.rs`): `neutralize_formula` antepone un apostrofo a ogni cella che inizia con uno di questi caratteri, in CSV, TXT e TSV degli appunti; nei TXT e TSV la neutralizzazione avviene dopo l'appiattimento di tabulazioni e a capo. Verifica: test `formula_partitions`, `formulas_are_neutralized_in_every_format` e proptest `csv_round_trip_any_cells` (nessuna cella esportata inizia con un carattere a rischio). Limite: l'apostrofo resta visibile se il file è riletto da un programma che non è un foglio di calcolo.
- **T/E**: "Apri" su un risultato che sia un programma o uno script (doppio clic su `pulisci.bat` in una cartella cercata). Mitigazione (`crates/lynkeus-app/src/launch.rs`): `open_plan` non apre i file con estensione eseguibile (`exe`, `bat`, `cmd`, `ps1`, `vbs`, `js`, `lnk`, `url`, `sh`, `py` e simili, elenco largo, compresi i tipi che Windows esegue per associazione: `jnlp`, `theme`, `themepack`, `msu`, `ppkg`, `vsix`, `pyc`, `pyz`, `mht`; punti e spazi finali e maiuscole non contano) e propone solo "Mostra nella cartella"; per una voce d'archivio si guarda il nome della voce. I file si aprono con il programma predefinito del sistema, mai con un programma cercato per nome, e con percorso assoluto. Verifica: test `executable_name_partitions` e `plan_partitions`. Un secondo filtro, `would_execute`, rifiuta anche i tipi che il sistema stesso considera pericolosi (`AssocIsDangerous` su Windows; su Linux non c'è un secondo filtro), perché l'elenco non conosce i tipi registrati dai programmi installati. Limite: un tipo di file con un gestore che esegue codice (per esempio un documento con macro) si apre come il sistema stabilisce.
- **I**: l'esportazione contiene contesto dei documenti: scelta esplicita dell'utente.
- **D**: copia negli appunti di un testo enorme (Ctrl+C con molte righe). Mitigazione: la tabella si costruisce in un thread; oltre 8 MiB (`MAX_CLIPBOARD_BYTES`, `app.rs`) niente appunti e un messaggio propone Esporta con le sole righe selezionate.
- **T**: scrittura del file esportato. Mitigazione: scrittura atomica e durevole (`write_export`: temporaneo nuovo, `sync_all`, rinomina); la conferma della sovrascrittura la chiede il dialogo di salvataggio del sistema.

### 4.7 Dettaglio: file, archivi e processo estrattore (fase 4)

Approfondisce 4.1 e 4.2 (confini 1 e 2). La scelta di isolare i decoder è nell'ADR `docs/adr/0001-estrazione-in-processo-isolato.md`. Le fasi 4A (crate `lynkeus-extract`) e 4B (pool, cache e Job Object in `lynkeus-engine` e `lynkeus-app`) sono implementate: nelle tabelle, "Implementato" elenca i test che ci sono (nome della funzione di test); "Previsto" ciò che manca ancora. Una mitigazione senza test implementato non è verificata. Le fixture sono in `fixtures/malevoli/` e `fixtures/formati/` (descrizione in `fixtures/README.md`).

**Asset**

- Riservatezza dei documenti e della cache: un decoder compromesso legge con i diritti dell'utente.
- Integrità dei file dell'utente e del sistema: nessuna scrittura causata da un file letto.
- Disponibilità: memoria, CPU, disco e tempo dell'app e del computer.
- Correttezza: un file ostile non deve far apparire testo che il documento non contiene, né nasconderlo senza avviso.

**Attaccante**

Chi riesce a mettere un file preparato ad arte nella cartella cercata (allegato scaricato, archivio ricevuto, cartella condivisa). Controlla ogni byte del file, il nome, l'estensione e i nomi delle voci negli archivi. Non è nel modello chi già esegue codice con i diritti dell'utente (potrebbe leggere la cache direttamente) né chi modifica l'eseguibile.

**Flusso**

file su disco, poi (A) scelta del formato, (B) decoder nel processo estrattore, (C) archivi e allegati annidati, (D) protocollo verso il processo principale, (E) cache su disco, poi ricerca nel processo principale. Il processo principale non decodifica formati complessi: legge solo i messaggi del protocollo e la cache, formati propri, stretti e versionati.

**Limiti (valori del piano, modificabili in "Avanzate")**

| Limite | Valore |
|---|---|
| Profondità degli archivi annidati | 5 |
| Voci per archivio | 100.000 |
| Byte decompressi per voce / per archivio | 512 MB / 4 GB |
| Rapporto di compressione | 200:1 |
| Tempo per file | 60 s |
| Annulla | stop (stdin chiuso), la ricerca risponde subito; il thread del pool termina il processo dopo 500 ms (`STOP_GRACE`) |
| Processi del pool | `min(core/2, 4)` |
| Cache su disco | 500 MB, LRU |
| Blocco di lettura dei file di testo | circa 8 MB, tagliato a fine riga |
| Memoria per processo estrattore | 1 GiB (`DEFAULT_MEMORY_LIMIT`; minimo accettato dal figlio 64 MiB). Windows: memoria impegnata per processo, Job Object. Linux: `RLIMIT_AS` imposto dal figlio |
| Dimensione massima di un messaggio del protocollo | 1,25 MiB (`MAX_FRAME_BYTES`; prima 16 MiB, protocollo v2). `PROTOCOL_VERSION` 3. Il lettore tiene al più 16 messaggi (circa 20 MiB) |
| Memoria stimata di un foglio ods o xls | 512 MiB in byte (`MAX_WORKBOOK_BYTES`, `extract/sheet_memory.rs`), al posto di un tetto in celle |
| Unità di testo | 1 MiB (`MAX_UNIT_TEXT_BYTES`), rivalidata dal padre |
| Voce di cache | 64 MiB di frame decompressi (`MAX_ENTRY_BYTES`) |

I byte si contano sul flusso decompresso (`Read::take`), mai sulle dimensioni dichiarate nelle intestazioni.

#### A. Scelta del formato

| STRIDE | Minaccia | Mitigazione | Test previsto |
|---|---|---|---|
| S | Estensione che mente: un decoder sbagliato legge dati scelti dall'attaccante | Formato scelto dall'estensione e verificato dal contenuto; se non coincidono, avviso "contenuto diverso dall'estensione", salvo i casi innocui del piano (.doc che è RTF, .xls che è HTML o CSV) | `malevoli/immagine-finta.pdf`: avviso, decoder PDF non eseguito. `formati/in-realta-rtf.doc`: letto come RTF (`HARMLESS` ha ora tre righe: `(Doc, Rtf)`, `(Xls, Html)`, `(Xls, testo)`). Implementato: `name_partitions`, `sniff_partitions`, `verify_partitions` (`format.rs`) e `malicious_files_give_warnings_without_blocking` (`ContentMismatch` per `immagine-finta.pdf`) |
| T | Binario spacciato per testo | NUL nei primi 8 KB e firme note controllati prima della lettura come testo | Fixture di `demo/binario/`: saltati e contati |
| D | File o riga enormi | Filtro sulla dimensione prima dell'apertura; lettura a blocchi di circa 8 MB; numero di riga con `memchr` | `malevoli/riga-lunga.txt` (riga di 2.000.000 di caratteri): memoria limitata, `aragosta` a riga 2 |
| E | Estensione che porta a un decoder più pericoloso del necessario | Mappa estensione-decoder fissa (allowlist); estensioni sconosciute mai provate "per vedere" | Test unitario sulla mappa: estensione non in elenco, doppia estensione, maiuscole, nome con NUL |

#### B. Decoder (PDF, Office, ODF, RTF, EML, EPUB)

| STRIDE | Minaccia | Mitigazione | Test previsto |
|---|---|---|---|
| T | Dati malformati: lettura fuori buffer, cicli, allocazioni da numeri dichiarati | Rust sicuro (`forbid(unsafe_code)` in `lynkeus-extract`); parser propri (.doc, .ppt, RTF) con lunghezze e offset confrontati con i byte presenti prima di allocare (`checked_*`, `try_from`); errore di parsing = avviso "danneggiato" | `malevoli/troncato.pdf`: avviso, nessun blocco. Fuzzing con cargo-fuzz (`fuzz/`; vedi `fuzz/README.md`). Ogni target ha un oracolo oltre al crash: `doc_piece_table`, `ppt_records` e `rtf` due letture uguali, paragrafi con testo e senza caratteri di controllo, tetto ai caratteri (`ppt_records` anche numeri di diapositiva crescenti e il tempo); `protocol_frame` andata e ritorno; `find_words` gli invarianti di `crates/lynkeus-core/tests/proprieta.rs`; `xml_reader` chiusure corrispondenti alle aperture e due letture uguali; `text_sniff` blocchi piccoli uguali ai predefiniti e UTF-8 riletto identico. Il corpus parte da `formati/`. `fuzz.yml` esegue ogni target per 5 minuti ogni giorno e conserva gli input che falliscono come artefatto (`fuzz-artifacts`). **Esiti delle sessioni:** si registrano nel registro delle esecuzioni di `docs/piano-di-test.md` (data, commit, target, durata, esito); ogni input che fallisce si riduce e diventa un test normale. Finora nessuna sessione è stata eseguita |
| T | XML con DTD ed entità annidate (billion laughs, XXE) | `quick-xml` senza espansione; `<!DOCTYPE>` rifiutato nei pacchetti (OOXML, ODF, file del pacchetto EPUB). Unica eccezione, i capitoli XHTML degli EPUB: un `<!DOCTYPE>` senza sottoinsieme interno (`<!DOCTYPE html>`, DTD pubblici) è ignorato, uno con sottoinsieme interno (quello che definisce entità) è rifiutato; si risolvono solo le entità predefinite e le entità HTML comuni da una tabella fissa; nessun XInclude né risorsa esterna | `malevoli/entita.docx`: avviso, nessuna espansione, memoria entro il limite |
| D | Decoder che non termina o consuma memoria | Timeout di 60 s con kill e nuovo processo; limite di memoria del processo; flussi con `Read::take` | Decoder finto che dorme e decoder finto che alloca (test del pool); `malevoli/bomba.zip`. Implementato: `misbehaving_workers_are_replaced_and_reported` (lento = `Timeout`), `malicious_files_give_warnings_without_blocking` (`bomba.zip`), `exceeding_the_memory_limit_kills_and_is_recognized` (Job Object, Windows) |
| D | Panic o abort del decoder abbattono l'app | Il decoder gira nel figlio: la sua morte dà "danneggiato" per quel file, il pool ne avvia un altro, la ricerca continua | Test: il figlio esce a metà di un file, gli altri file del lotto sono elaborati. Implementato: `misbehaving_workers_are_replaced_and_reported` ("muore": `Corrupt`, `note.docx` letto con un processo nuovo) |
| I | Il decoder legge altro o apre la rete | Il figlio riceve il percorso e nulla di più; nessuna dipendenza che usi la rete (`cargo deny` e revisione); ambiente svuotato | Controllo delle dipendenze in CI; test sulle variabili d'ambiente del figlio |
| I | Testo protetto da password | Avviso "protetto da password", nessun tentativo di indovinare | `formati/protetto-password.docx`: avviso, nessun testo |
| E | Esecuzione di codice (macro, OLE, JavaScript nei PDF, collegamenti esterni) | Si estrae solo testo; macro, oggetti incorporati e collegamenti ignorati; nessun avvio di programmi | Controllo `nospawn` di `cargo xtask check` (`xtask/src/nospawn.rs`): i sorgenti di `crates/lynkeus-extract/src` non nominano `std::process`, `process::`, `Command` (parola intera), `libc::exec`/`fork`/`vfork`/`system`, `posix_spawn`, `CreateProcess`, `ShellExecute`, `WinExec`; le righe di commento non contano; l'elenco dei file esclusi (`ALLOWED_FILES`) è vuoto, e un controllo che non trova sorgenti fallisce. L'avvio dei processi sta in `launcher.rs` dell'engine e nell'app, fuori dalla cartella controllata. Implementato: `forbidden_uses_are_found`, `similar_names_and_comments_are_not_findings`, `line_numbers_start_at_one`. Limite: è un controllo sul testo, non sul grafo delle chiamate (una dipendenza che avvia processi non si vede: la coprono `cargo deny` e la revisione) |
| E | Compromissione del figlio per un bug | Risorse limitate, nessuna scrittura sui dati dell'utente, ambiente e handle minimi; il resto è rischio accettato (sezione 5) | Sezione D e ADR 0001 |

#### C. Archivi e allegati annidati (zip, tar, gz, 7z, docx, odt, EPUB, EML)

| STRIDE | Minaccia | Mitigazione | Test previsto |
|---|---|---|---|
| T | Zip slip: nomi con `..`, assoluti, unità, UNC, separatori dell'altro sistema | I nomi sono solo etichette, mai usati per scrivere. Se per aprire una voce serve un file, nome generato dal programma nella cartella temporanea privata | `malevoli/percorsi.zip`: nessun file creato fuori dalla cartella di lavoro (elenco dei file prima e dopo) |
| T | Link simbolici e voci speciali (tar, 7z) | Solo voci regolari; le altre ignorate e contate; nessun link seguito | Implementato (test che costruiscono l'archivio, nessun file in `malevoli/`): `symlink_entry_is_skipped` (zip), `tar_skips_links_and_counts_entries`, `special_entries` (7z) |
| D | Zip bomb: rapporto, byte per voce e per archivio | `Read::take` sul flusso; 200:1, 512 MB per voce, 4 GB per archivio, controllati durante la lettura | `malevoli/bomba.zip` (circa 1000:1): avviso "limite superato", memoria limitata |
| D | Troppe voci | Conteggio sull'elenco delle voci prima di decomprimere: oltre 100.000, avviso | Implementato: `many_entries_zip_hits_the_default_limit` (`tests/archives.rs`) costruisce al volo uno zip con 100.001 voci vuote (circa 9 MB, troppo per il repository): l'unico evento è l'avviso `LimitExceeded(ArchiveEntries)`, nessuna voce letta; `too_many_entries` prova lo stesso con un limite basso. `malevoli/molte-voci.zip` resta generabile con `--vuote 100001`, non committato |
| D | Annidamento profondo o circolare | Profondità massima 5 con contatore passato a ogni livello; vale anche per allegati EML e archivi dentro docx | `malevoli/annidato-profondo.zip` (7 livelli): avviso, `fondo.txt` non raggiunto. `formati/annidato.zip` (3 livelli): letto |
| D | Livelli ciascuno sotto il limite ma sopra in totale | Byte e voci contati sull'intero archivio radice, non per livello | Implementato: `total_size_counts_all_levels` (`tests/archives.rs`); `archives_nest_inside_zip_with_depth_limit` (`tests/formats.rs`); allegati EML: `eml_attachments_follow_the_archive_limits`; gz: `gz_bomb_hits_ratio_limit` |
| I | Nomi con caratteri di controllo o bidi che ingannano chi legge i risultati | Etichette mostrate con i caratteri di controllo sostituiti | Implementato: `control_characters_in_names_are_neutralized` (`tests/archives.rs`, bidi U+202E sostituito da U+FFFD) |
| S | Voce con estensione che mente dentro l'archivio | Stessa verifica del passo A su ogni voce | Implementato: `lying_extension_inside_archive` (`tests/archives.rs`, usa `malevoli/immagine-finta.pdf` dentro uno zip) |

#### D. Protocollo verso il processo principale (stdin e stdout del figlio)

| STRIDE | Minaccia | Mitigazione | Test previsto |
|---|---|---|---|
| S | Avvio di un eseguibile diverso | Percorso assoluto di `current_exe()`, mai `PATH` né cartella corrente | Previsto e implementato: `relative_executables_are_refused` e `the_command_has_a_minimal_environment` (`launcher.rs`), `relative_paths_are_refused` (`windows_ffi.rs`) |
| T | Messaggio alterato, troncato o con lunghezza falsa | Lunghezza in testa confrontata con il massimo prima di allocare; `postcard` in tipi concreti; violazione = figlio terminato e sostituito | Implementato: `every_truncation_is_detected`, `oversized_length_is_rejected_before_reading`, `short_length_wrong_version_and_trailing_bytes`, `oversized_message_is_not_written`, `random_bytes_never_panic` (proptest), `events_round_trip` (`protocol.rs`); lato figlio `malformed_frame_is_rejected_then_exit` e `truncated_request_is_rejected` (`worker.rs`). Fuzzing del decodificatore: `protocol_frame` (andata e ritorno), nessuna sessione ancora eseguita |
| T | Il figlio scrive su stdout cose estranee al protocollo | Stdout riservato al protocollo; log solo su stderr, letto con tetto | Decoder finto che stampa su stdout: figlio sostituito. Implementato: "sporco" in `misbehaving_workers_are_replaced_and_reported` |
| R | Non rilevante | | |
| I | Il figlio riceve più del necessario | `env_clear` salvo poche variabili di sistema; nessun handle ereditato oltre a stdin, stdout, stderr (lista esplicita su Windows, chiusura dei descrittori su Linux) | Implementato: ambiente (`the_child_sees_only_system_root`, Windows; `the_command_has_a_minimal_environment`). Handle ereditati: `worker_inherits_only_the_standard_descriptors` (`crates/lynkeus-engine/tests/processi.rs`, solo Linux): il padre tiene aperti tre file e un processo estrattore fratello, il figlio elenca i propri descrittori e vede `0 1 2`. Non eseguito in locale (su questa macchina Linux è solo compilato): lo esegue la CI. Su Windows nessun test conta gli handle del figlio: il lanciatore dell'app imposta `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (`spawn_in_job`) ma la lista non è verificata; il lanciatore portabile su Windows è coperto solo in modo indiretto da `closing_stdin_stops_a_worker_while_a_sibling_lives` (il fratello non ha ereditato il capo di scrittura dello stdin). Il confine degli handle su Windows resta quindi non verificato |
| I | Contenuto dei documenti nei log o nei rapporti di crash | Gli errori portano motivo e percorso, mai estratti di testo; il messaggio di un panic nel rapporto di crash perde i testi tra virgolette e le parole che sembrano percorsi (`redact`) | Implementato: `redaction_partitions` e `source_files_lose_local_folders` (`diagnostics.rs`). Previsto: test del log completo dopo un file con testo riconoscibile (il figlio ha stderr nullo e non scrive log) |
| D | Figlio muto, che inonda stdout o ignora l'annulla | Timeout 60 s con kill; lettura con tetto; annulla = stdin chiuso, risposta subito e kill dopo 500 ms nel thread del pool; `KILL_ON_JOB_CLOSE` (Windows) e, su Linux, la sorveglianza del padre evitano orfani se l'app muore. Linux (B-04): il padre passa il proprio pid con `--parent-pid` (`PARENT_PID_FLAG`, `WorkerConfig::parent_pid`); `worker::run` avvia `watch_parent`, che legge il padre in `/proc/self/stat` subito e poi a intervalli fissi (`PARENT_CHECK_INTERVAL`), e appena è diverso dal pid ricevuto chiama la funzione passata a `run`, che in `main.rs` termina il processo con `exit(3)`, anche se è occupato in un decoder. `PR_SET_PDEATHSIG` è escluso, con il motivo scritto in `crates/lynkeus-extract/src/platform.rs`: chiamare `prctl` chiederebbe in quel crate un modulo di gestione dei processi che il controllo `nospawn` non ammette, e il figlio dovrebbe comunque confrontare il padre con il pid ricevuto per il caso di padre morto prima della chiamata | Implementato: figlio muto con timeout ("lento"), annulla entro 1 s (`cancel_stops_a_busy_worker_within_one_second`), chiusura del job che termina il figlio (`closing_the_job_kills_the_child`), `terminate_kills_and_is_idempotent`. Orfani: `an_orphaned_worker_exits_when_its_parent_dies` (`processi.rs`, lanciatore portabile, su entrambi i sistemi): un padre intermedio avvia il figlio, che tiene un lock su un file; il padre è ucciso di colpo e il figlio esce entro 5 s (il lock si libera). `busy_orphan_exits_with_its_parent` (`crates/lynkeus-engine/tests/processi.rs` e `crates/lynkeus-extract/tests/processo.rs`, solo Linux) prova lo stesso con il figlio occupato: **mai eseguito in locale**, lo esegue la CI. Su Windows l'app ha in più il Job Object (`closing_the_job_kills_the_child`). Previsto: figlio che scrive senza fine |
| D | Figlio compromesso che manda unità di testo enormi o voci annidate oltre la profondità chiesta (il limite di 1 MiB vale nel figlio) | Il padre rivalida (B-03): un'unità oltre `MAX_UNIT_TEXT_BYTES` (1 MiB) o una voce oltre `max_depth` è `ProtocolViolation`, processo terminato (`event_fits`, `pool.rs`); frame al più 1,25 MiB controllato dal lettore prima di allocare, al più 16 messaggi in coda | Implementato: `unit_over_1_mib_is_a_protocol_violation` e `frame_over_the_maximum_is_a_protocol_violation` (`crates/lynkeus-engine/tests/protocollo.rs`) |
| D | Foglio ods o xls piccolo che in memoria diventa enorme (per esempio testo ripetuto, `malevoli/testo-ripetuto.ods`, 737 byte) | `sheet_memory.rs` stima i byte che calamine userebbe (rettangoli, valori, testo clonato per ogni ripetizione, stringhe condivise) prima di passargli il file; oltre 512 MiB avviso «memoria»; le formule degli xls si svuotano prima di misurare. Gli xlsx si leggono a flusso (`worksheet_cells_reader`). Falso positivo noto e accettato: un xls con record sovrapposti in cui una formula sfugge allo svuotamento si rifiuta con l'avviso «memoria» | `sparso.xlsx`, `testo-ripetuto.ods` (anche nel test d'accettazione dell'app) e `registro-lungo.xls` (letto per intero) |
| E | Un figlio compromesso chiede al padre cose non previste | Il padre accetta solo i messaggi previsti nello stato corrente (richiesta, risposta, fine); nessun comando dal figlio al padre; i risultati sono solo testo da cercare | Implementato: "fuori-sequenza" e "altro-id" in `misbehaving_workers_are_replaced_and_reported`; `sequence_partitions` (`event.rs`) |

#### E. Cache su disco

| STRIDE | Minaccia | Mitigazione | Test previsto |
|---|---|---|---|
| I | Altri utenti o processi leggono il testo dei documenti | Descrittore esplicito (solo utente e SYSTEM) su Windows, 0700 e 0600 su Linux; cache spegnibile e svuotabile; avviso nel README | Implementato (fase 7, dettaglio in 4.8 C): Windows `private_dirs_have_an_explicit_protected_dacl`, `private_files_have_an_explicit_dacl_and_are_new_only`, `an_existing_dir_open_to_others_is_refused`; Linux `unix_permissions_and_refusals` (0700 e 0600, cartella 0777 rifiutata senza `chmod`). Previsto: prova con `umask` 0 e 077 |
| T | Voce alterata, troncata o di versione diversa | Intestazione con versione e CRC; voce non valida ignorata e cancellata; stessi limiti di lunghezza del protocollo | Implementato: `corrupt_entries_are_ignored_and_removed` (CRC, troncata, versione futura, lunghezza enorme, altra chiave, spazzatura), `entries_with_a_bad_event_sequence_are_corrupt`, `entries_round_trip` (byte casuali) e `second_search_reads_from_the_cache_and_corrupt_entries_are_dropped` |
| T | Voce valida per la chiave ma con testo diverso dal documento | Chiave `blake3` di percorso, dimensione, data di modifica, versione degli estrattori e limiti. Chi scrive nella cache ha già i diritti dell'utente: rischio accettato | Implementato: `key_partitions` (percorso, dimensione, data, limiti); la versione del programma e del protocollo sono nell'hash ma non hanno un test proprio |
| T | Scrittura a metà, nomi pericolosi, file preesistenti | Scrittura su temporaneo e rinomina; creazione che fallisce se il file esiste; nomi dei file derivati dall'hash, mai dal nome del documento o della voce | Implementato: nome esadecimale in `key_partitions`. Cache interrotta: `interrupted_cache_writes_are_ignored_and_the_document_is_read_again` (`crates/lynkeus-engine/tests/documenti.rs`): una voce troncata a metà si scarta e il documento si rilegge (nessun avviso, risultati uguali, nessun colpo di cache); un temporaneo `.tmp` di oltre un'ora è cancellato alla scansione, uno recente resta (può essere di un'altra istanza che sta scrivendo); la voce riscritta è valida e la ricerca successiva, senza processi, la legge dalla cache |
| D | Crescita illimitata | LRU a 500 MB; una voce che da sola non entra non si salva | Implementato: `least_recently_used_entries_are_evicted`, `an_entry_larger_than_the_cache_is_not_saved` |
| D | Dimensione decompressa `lz4_flex` dichiarata falsa | Dimensione dichiarata nell'intestazione confrontata con il massimo prima di decomprimere | Implementato: lunghezza dichiarata `u32::MAX` in `corrupt_entries_are_ignored_and_removed` |

#### F. Apertura delle voci d'archivio (`ExtractEntry`, dal protocollo v2)

Con `ExtractEntry` i byte originali di una voce d'archivio escono dal processo isolato e arrivano nel processo principale, che li scrive in un file e lo fa aprire dal sistema. È il solo punto in cui contenuto non fidato lasciato dall'estrattore diventa un file eseguibile da un altro programma (`protocol.rs`, `extract_entry`; `crates/lynkeus-app/src/extraction.rs`, `launch.rs`). Il padre tratta i byte come opachi: non li interpreta.

| STRIDE | Minaccia | Mitigazione | Test previsto |
|---|---|---|---|
| T/E | La voce è un programma o uno script che il sistema eseguirebbe all'apertura | `open_plan` guarda il nome della voce e rifiuta le estensioni eseguibili (stesso elenco dei file sul disco); `run` ricontrolla il file estratto e non lo apre se ha un'estensione eseguibile | Implementato: `plan_partitions`, `an_extracted_executable_is_never_opened` (`launch.rs`) |
| T/E | Estensione ingannevole, con caratteri strani, troppo lunga o con punti e spazi finali (`a.bat. `) | L'estensione del file sul disco è `opened_extension`: quella della voce in minuscolo solo se fatta di lettere e cifre ASCII (al più 16) e non eseguibile; altrimenti `txt` per le voci senza formato noto, `bin` per le altre | Implementato: `opened_extension_partitions` |
| T | Nome della voce con percorso (`..\..`), nomi riservati o caratteri di controllo usato come nome del file | Il nome della voce è solo un'etichetta: il file si chiama `voce-<16 cifre esadecimali>.<estensione>`, generato dal programma; creazione con `create_new_private_file`, che fallisce se il file esiste (ritentativi limitati a 16) | Implementato: `entry_files_have_generated_names_and_are_new` |
| I | Altri utenti o processi leggono la voce estratta | Cartella creata privata (`create_private_dir`) e file con permessi privati alla creazione (0700/0600 su Linux; su Windows vale il limite della parte C di 4.8) | Implementato: gli stessi test della parte C di 4.8 (le cartelle sono create da `create_private_dir`), compresa la DACL esplicita su Windows. Il file estratto eredita la DACL della cartella (Windows) o nasce 0600 (Linux) |
| D | Voce enorme (anche da bomba di decompressione) che riempie il disco o la memoria | Limite per voce `max_entry_size` (512 MiB predefiniti, modificabile in "Avanzate") inviato nella richiesta e contato dall'estrattore sul flusso decompresso; stessi limiti degli archivi della ricerca (profondità, voci, rapporto); superato il limite l'esito è "limite superato" e il file si cancella | Implementato: `entry_size_limit_is_counted_on_the_stream` (`archives.rs`), `entry_partitions` (`voci.rs`) e `entry_partitions_through_the_pool` (`documenti.rs`) |
| D | Frame enorme o inondazione di dati dal figlio | I byte viaggiano in pezzi di al più 1 MiB (`ENTRY_CHUNK_BYTES`), sotto `MAX_FRAME_BYTES` (1,25 MiB, con 256 KiB di margine per posizione, etichette e percorso); il lettore controlla la lunghezza prima di allocare; timeout di 60 s e kill come per le altre richieste | Implementato: `largest_entry_chunk_fits_in_a_frame` (`protocol.rs`) |
| T | Estrazione interrotta o fallita a metà (figlio ucciso, disco pieno, esito diverso da "estratta") | Se l'esito non è "estratta" i byte già mandati non valgono e il file a metà viene cancellato; il primo errore di scrittura ferma l'estrazione | Previsto: test sul file a metà dopo un esito di errore (oggi coperto solo dalla struttura del codice) |
| D | Avanzi nella cartella temporanea: dati dell'utente che restano su disco | Ogni istanza scrive in una propria sottocartella `s-<32 cifre esadecimali>` con `session.lock` bloccato finché vive; all'avvio (in un thread) `clean_stale_sessions` cancella solo le sottocartelle il cui lock non è tenuto da nessuno (istanze finite, anche in modo anomalo) e i file sciolti di versioni precedenti, mai quelle di un'altra istanza in corso; all'uscita l'istanza cancella la propria; un file ancora aperto da un altro programma resta e si riprova alla volta dopo | Implementato: `startup_cleaning_spares_the_files_of_a_running_instance` e `session_names_are_recognized` (`extraction.rs`: sessione in corso intatta, sessione finita e file sciolto tolti, cartella estranea intatta). Limite: tra l'apertura e l'uscita i file restano su disco |
| E | Un figlio compromesso manda `EntryData` senza richiesta o con un altro `id` | Il padre accetta solo i messaggi previsti nello stato corrente (come in D); figlio sostituito alla prima violazione | Implementato: `entry_data_out_of_sequence_is_rejected` (`crates/lynkeus-engine/tests/protocollo.rs`: processo sostituito, il file seguente si legge) |
| E | La voce estratta da un archivio scaricato perde il Mark-of-the-Web: un documento con macro si apre senza il blocco (D-02) | Windows: `copy_mark_of_the_web` (`platform/zone.rs`) copia sulla voce estratta la zona (`ZoneId`) dello stream `Zone.Identifier` dell'archivio; lo stream si legge per al più 64 KiB e se ne riconosce solo `ZoneId`; sulla voce si scrive un testo generato dal programma, senza gli URL d'origine; stream illeggibile o senza zona valida = Internet (fallire chiusi); se la zona non si scrive il file estratto si cancella e non si apre. Linux: nessuna zona | Implementato: `extracted_entry_keeps_mark_of_the_web` (`extraction.rs`: zona copiata, URL d'origine non copiato) e i test di `zone.rs` |
| E | Il programma associato dal sistema al tipo di file ha vulnerabilità o esegue macro | Fuori dal controllo del programma; si apre solo ciò che l'utente ha chiesto, con un'estensione ammessa, e mai con un programma cercato per nome | Rischio accettato (sezione 5) |

#### Limiti noti di 4.7

- **Linux: memoria superata non distinta da un aborto.** `RLIMIT_AS` fa fallire le allocazioni e il figlio abortisce: il padre vede `WorkerExit::Failed` e dà l'avviso "danneggiato" invece di "limite superato (memoria)". Su Windows il Job Object (messaggio sulla porta di completamento, o picco entro 1/16 dal limite) lo distingue. Il limite è sulla memoria impegnata/virtuale, non sul working set.
- **Voci d'archivio apribili (aggiornato).** Dal protocollo v2 "Apri" estrae i byte originali di qualsiasi voce (vedi F); il limite precedente, che valeva solo per le voci di testo (64 MiB), non c'è più. Resta il messaggio di non disponibilità quando il processo estrattore non è collegato.
- **Decoder di terzi con `unsafe`.** `lynkeus-extract` ha `forbid(unsafe_code)`, ma due dipendenze della fase 5 contengono `unsafe` nel proprio codice: `mail-parser` (EML, e HTML ridotto a testo) e, per il 7z, `lzma-rust2` (dipendenza di `sevenz-rust2`, che di suo non ha blocchi `unsafe`). Il processo estrattore resta l'unica difesa (sezione 5: rischio da rivalutare). Il conteggio è in `docs/dipendenze.md`.
- **Handle ereditati non contati su Windows.** Il test `worker_inherits_only_the_standard_descriptors` esiste solo per Linux; la `HANDLE_LIST` del lanciatore dell'app (Windows) non ha un test che conti gli handle del figlio.
- **Il controllo `nospawn` è sul testo** dei sorgenti di `lynkeus-extract/src`, non sulle dipendenze (vedi la riga E della parte B).
- **Test di Linux eseguiti solo dalla CI.** In locale Linux è solo compilato (`cargo xtask check-linux`): i test con `#[cfg(target_os = "linux")]` o con permessi Unix non sono stati visti passare su questa macchina.

### 4.8 Dettaglio: istanza unica, clic destro e cartelle private (fase 7)

Approfondisce 4.3, 4.4 e 4.5 (confini 3, 4 e 5). La fase 7 è nel codice: istanza unica (`crates/lynkeus-app/src/instance.rs`, trasporto in `platform/mod.rs` e `platform/windows_ffi.rs`), clic destro (`context_menu.rs`, registro e script Nautilus in `platform/mod.rs`), cartelle private (`private_dirs.rs`; `PrivateFs` in `crates/lynkeus-engine/src/platform.rs`; descrittore esplicito su Windows in `windows_ffi.rs`), log e rapporti di crash (`diagnostics.rs`), riga di comando (`cli.rs`), manifest (`windows/lynkeus-searcher.manifest`, incorporato da `build.rs`). Nelle tabelle "Implementato" elenca i test che ci sono (nome della funzione di test); "Previsto" ciò che manca e "Differenza" ciò che il codice fa in modo diverso dal progetto scritto. Una mitigazione senza test implementato non è verificata. Le sezioni "Progetto" descrivono il progetto; dove il codice si discosta c'è una nota "Nel codice".

**Cosa è implementato (riassunto)**

- Il server dell'istanza unica nasce solo negli avvii con `--add` (`main.rs`: `launched_with_add`) e solo se la cartella dati è privata; senza `--add` non c'è né canale né lock. Ogni errore (lock, canale, consegna) porta ad `Alone`: l'avvio apre la propria finestra.
- Una cartella dell'app non privata (altro proprietario, permessi o DACL aperti) non si usa per la sessione: la funzione che ne dipende si spegne (configurazione: impostazioni solo in memoria; dati: istanza unica, log, rapporti e voci d'archivio; cache; ciascuna per conto suo) e una barra messaggi avvisa l'utente (`Notice::PrivateDirsRefused`, con il numero di cartelle). Nessuna cartella è "riparata" in silenzio.
- La voce del clic destro non si sovrascrive né si cancella se è di un altro programma (`MenuState::Foreign`); non si registra se l'eseguibile o la sua cartella hanno un proprietario non fidato o sono scrivibili da altri (`MenuError::ExeDirWritable`), se il percorso ha `"`, NUL, a capo o non è UTF-8, o se la riga supera 260 caratteri. L'utente vede il motivo in una barra messaggi (`Notice::ContextMenuFailed`).
- Il tetto di 256 luoghi di ricerca (`MAX_ROOTS`) vale per l'elenco dell'utente, per i percorsi dell'istanza unica e per le impostazioni lette; oltre, il luogo non entra e un avviso (`Notice::RootsLimit`) lo dice.
- Log in file con rotazione giornaliera (al più 7 file) e rapporti di crash (al più 10), in cartelle private.

**Attaccanti (comuni alle tre parti)**

- **U1, altro utente locale** dello stesso computer.
- **U2, processo dello stesso utente a privilegi diversi**: processo a integrità Low o con token ristretto che non dovrebbe poter comandare l'app. Un processo con integrità pari o superiore e lo stesso utente è fuori modello (potrebbe già leggere la cache e sostituire l'eseguibile; sezione 5).
- **U3, file nella cartella cercata**: nomi di file e cartelle (spazi, virgolette, `-` iniziale, `&`, `%`, a capo, percorsi lunghi, collegamenti) che l'utente seleziona in Explorer o Nautilus e arrivano come argomento di `--add`.

#### A. Istanza unica (pipe, socket, lock, token)

**Asset**

- Controllo dell'app in esecuzione: chi scrive sul canale decide quali luoghi compaiono in "dove cercare", quindi cosa verrà letto alla prossima ricerca.
- Identità dell'utente: un server finto non deve ottenere un token di impersonazione del client.
- Segretezza del token e del nome della pipe.
- Disponibilità: né la prima istanza né i client devono bloccarsi.

**Progetto**

1. Il lock (`fs4`) è un file della cartella privata (parte C). Chi lo ottiene è la prima istanza, gli altri sono client. Il lock decide solo chi è il server; non protegge la segretezza.
2. La prima istanza genera a ogni avvio un nome di pipe casuale (GUID) e un token da 128 bit dal generatore del sistema operativo. Nome e token stanno in un file della cartella privata, creato con `create_new` e riscritto a ogni avvio della prima istanza, cancellato allo spegnimento. Mai in riga di comando, log, registro o rapporti di crash.
3. Windows, server: `CreateNamedPipeW` con `FILE_FLAG_FIRST_PIPE_INSTANCE` (se il nome esiste già, errore: si sceglie un altro nome, non si usa mai l'oggetto esistente), `PIPE_REJECT_REMOTE_CLIENTS`, una sola istanza, DACL esplicita e protetta con una sola ACE per il SID dell'utente (letto dal token, `TokenUser`; non il SID della logon session). Il server non impersona mai il client: non chiama `ImpersonateNamedPipeClient`.
4. Windows, client: apre la pipe con `CreateFile` e `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION` (senza il primo flag il secondo è ignorato; in Rust `security_qos_flags`). Prima di mandare il token verifica che il server sia un processo dell'utente corrente (`GetNamedPipeServerProcessId` e il SID del suo token).
5. Linux: socket Unix in `$XDG_RUNTIME_DIR` con 0600 già alla creazione (`umask` 0177 attorno al `bind`); se la variabile manca o la cartella non è dell'utente con 0700, l'istanza unica si spegne e ogni avvio resta indipendente (fallire chiusi). Il server legge `SO_PEERCRED` e accetta solo lo stesso uid.
6. Messaggio: versione, token (16 byte), numero di percorsi, percorsi come testo. Tetto alla dimensione totale (64 KiB) e al numero di percorsi (256), controllati prima di allocare; timeout di lettura (2 s); una connessione servita alla volta. Il token si confronta in tempo costante (`subtle`); a token errato la connessione si chiude senza risposta.
7. Il server accetta un percorso solo se è assoluto, senza NUL, non UNC né dispositivo (`\\?\`, `\\.\`), e se esiste (`symlink_metadata` riuscito). Lo aggiunge all'elenco dei luoghi senza avviare la ricerca e senza aprirlo. Il percorso è un dato: mai a una shell o a un programma. Una sola funzione di validazione per riga di comando e canale (un solo guardiano per risorsa).
8. Il client, verificato il server e prima di mandare i percorsi, chiama `AllowSetForegroundWindow(pid della prima istanza)` (se fallisce, solo un log di debug); poi consegna, aspetta la conferma (1 byte) ed esce. La prima istanza porta la finestra in primo piano quando arrivano percorsi. Se il server non risponde entro il timeout, il client apre una propria finestra: nessun blocco, nessun tentativo infinito. Il client aspetta il file dell'istanza al più 2 s (ritenta ogni 50 ms): Explorer avvia un processo per elemento quasi insieme e il primo può non averlo ancora scritto.
9. **Deciso:** il server parte a ogni avvio con `--add`, indipendentemente dall'opzione del clic destro, perché la stessa voce può essere creata da altri strumenti o dall'utente; senza `--add` non c'è né canale né lock. Un `--add` a mano è un avvio dell'utente: non cambia il modello.
10. Formato (versione 1, `PROTOCOL_VERSION`): messaggio = magia `LKSM` (4), versione u16 LE, token (16), lunghezza del resto u32 LE; poi numero di percorsi u16 LE e, per ognuno, lunghezza u16 LE e testo UTF-8. I percorsi non UTF-8 o oltre 65.535 byte non si mandano. Il file dell'istanza: magia `LKSI`, versione, token, lunghezza e testo del nome del canale (al più 1 KiB). Un client manda un messaggio per gruppo di al più 256 percorsi e 64 KiB. Una coda di 64 messaggi in attesa del thread dell'interfaccia; oltre si scartano (log di avviso). Dettagli del protocollo nella skill `lynkeus-internals`.

**STRIDE**

| STRIDE | Attaccante | Minaccia | Mitigazione | Test previsto |
|---|---|---|---|---|
| S | U1, U2 | Processo che crea per primo la pipe con il nome atteso e raccoglie token e percorsi | Nome casuale a ogni avvio, noto solo dal file privato; `FILE_FLAG_FIRST_PIPE_INSTANCE`; il client verifica che il server sia dell'utente prima di mandare il token | Implementato: `a_pipe_name_already_in_use_is_refused` (seconda istanza con lo stesso nome: errore; `bind_new_endpoint` ritenta con un nome nuovo, al più 3 volte). Il client rifiuta un server di un altro utente (`connect_instance`: `process_user(pid) != current_user()`), ma il caso con SID diverso non ha un test (solo `the_current_process_belongs_to_the_current_user`): Previsto. Il client accetta solo nomi `\\.\pipe\lynkeus-searcher-…` senza ulteriori `\`. Linux: il client non verifica l'uid del server; protegge `$XDG_RUNTIME_DIR` (dell'utente, senza permessi per altri) |
| S | U1, U2 | Processo che si spaccia per client | DACL solo utente (esclude U1); token da 128 bit nel file privato; `SO_PEERCRED` su Linux | Implementato: `a_message_without_token_is_refused`, `a_wrong_token_is_refused_without_reply`, `a_second_launch_delivers_to_the_first` (sul canale vero: token con un byte sbagliato, nessuna risposta e nessun percorso aggiunto), `random_bytes_are_refused` (proptest); il token si confronta con `subtle::ConstantTimeEq` (`Token::matches`), la sola comparazione sul token (`Token` non ha `PartialEq`, quindi non si può confrontare in altro modo); `tokens_match_only_themselves`. DACL della pipe: `private_descriptor(PrivateObject::Pipe)` (solo utente); Linux: `SO_PEERCRED` con scarto delle connessioni di un altro uid, senza test (richiede un secondo utente): Previsto |
| T | U3 | Percorso con NUL, UNC, `\\?\`, `..`, inesistente, collegamento verso altrove | Validazione unica; normalizzazione lessicale con `platform::normalize`; UNC rifiutato (evita l'autenticazione verso un host scelto da altri); nessuna apertura | Implementato: `path_shape_partitions` (vuoto, relativo, NUL, oltre 32.767 caratteri, UNC, `\\?\`, `\\.\`, `\\?\UNC\`), `accepted_paths_exist_and_are_normalized` (`..` e `.` risolti sul testo, inesistente, nomi con `-`, spazi, `&`, `%`, `^`, `;`, Unicode), `values_are_data` e `several_adds_and_end_of_options` (`cli.rs`: il valore dopo `--add` è un dato). Previsto: collegamento circolare |
| T | U2 | Messaggio troncato, con conteggi o lunghezze false | Tetti prima dell'allocazione, timeout di lettura | Implementato: `oversized_messages_are_refused_before_allocating`, `too_many_paths_are_refused`, `malformed_payloads_are_refused`, `many_paths_are_split_into_messages_within_limits`, `random_bytes_are_refused` e `tampered_messages_never_add_paths` (proptest: nessun panic, mai più percorsi di quelli mandati). Il timeout di lettura (2 s) è nel trasporto, senza un test dedicato: Previsto |
| R | tutti | Negare di aver mandato un percorso | Non rilevante: effetto limitato a una voce in più nell'elenco | |
| I | U1, U2 | Lettura del token o del nome della pipe | File privato (parte C); mai in log, riga di comando, rapporti di crash | Implementato: `instance_file_round_trip_and_garbage`, `instance_file_is_written_new_and_private` (file creato con `PrivateFs`, riscritto a ogni avvio); `Token` non stampa il valore (`Debug` fisso). Previsto: la ricerca del token nel log e nei rapporti dopo un ciclo completo |
| I | U1 | Elencare le pipe per scoprire un'istanza attiva | Non evitabile su Windows; nome casuale e DACL limitano l'uso | Rischio accettato |
| D | U1, U2 | Connessioni che non scrivono, riempimento della pipe, blocco dell'unica istanza | Una connessione alla volta con timeout; `PIPE_REJECT_REMOTE_CLIENTS`; tetto ai percorsi; client con ripiego su finestra propria. Su Windows l'I/O sovrapposto non visto finire (scadenza, anche a zero millisecondi con `ERROR_IO_INCOMPLETE`, o errore dell'attesa) si annulla e se ne attende la chiusura (`cancel_and_wait`) prima di liberare `OVERLAPPED`, evento e buffer. Un client che si connette e chiude subito (`ERROR_NO_DATA`) non ferma il server; un errore nell'attesa si registra e si riprova dopo una pausa, e solo 20 errori di fila (`MAX_ACCEPT_ERRORS`) fermano il server, registrandolo. Su Linux l'`accept` non è bloccante, con `poll` a tempo: lo stop si vede sempre | Implementato: `pipe_round_trip_with_timeouts` (Windows, `windows_ffi.rs`: letture e scritture a tempo limitato), `an_expired_deadline_cancels_the_pending_read` (scadenza: lettura annullata e conclusa prima di liberare la struttura), `a_client_that_closes_at_once_does_not_stop_the_server` (il client successivo è accettato). Previsto: client lento che non scrive, con il server che serve il successivo; i 20 errori di fila non hanno un test |
| D | U3 | Selezione di migliaia di elementi: Explorer lancia un processo per elemento | Processi brevi (connetti, manda, esci); percorsi aggiunti in blocco senza ricerca; elenco con tetto (256 percorsi e 64 KiB per messaggio, coda di 64 messaggi); l'elenco dei luoghi in sé ha un tetto di 256 voci (`MAX_ROOTS`): oltre, i percorsi non entrano e l'utente vede un avviso | Implementato: `many_paths_are_split_into_messages_within_limits`, `roots_stop_at_the_limit` (`form.rs`: l'ultimo entra, il successivo no, un doppione resta un doppione; anche le impostazioni lette si fermano a 256) e `a_full_list_of_places_refuses_more_with_a_notice` (`app.rs`: avviso `RootsLimit`). Previsto: 1.000 connessioni in sequenza, memoria e tempo entro limite |
| D | U1 | Occupare il file di lock o il nome del socket | Lock nella cartella privata (inaccessibile a U1); `$XDG_RUNTIME_DIR` scrivibile solo dall'utente, altrimenti istanza unica spenta; ogni errore dà `Startup::Alone` | `a_second_launch_delivers_to_the_first` (senza `$XDG_RUNTIME_DIR` su Linux il test verifica che l'istanza unica sia spenta). Previsto: lock o socket preesistenti con altro proprietario |
| E | U1 | Impersonazione del client da parte di un server ostile | SQoS Identification nel client; il server legittimo non impersona | Implementato: `the_pipe_client_asks_for_identification_only` (`PIPE_CLIENT_SQOS` contiene `SECURITY_SQOS_PRESENT` e `SECURITY_IDENTIFICATION`, livello 1) |
| E | U2 | Usare il canale come deputato confuso per far leggere file | Un percorso aggiunto non si legge finché l'utente non preme Cerca; il canale non porta diritti né comandi diversi da "aggiungi" | Implementato: `external_roots_are_added_without_searching` (`app.rs`: i percorsi esterni si aggiungono senza avviare la ricerca) |

**Rischi accettati (A)**

- Un processo dello stesso utente con integrità Medium o superiore legge il file del token e aggiunge luoghi all'elenco: effetto che avrebbe già modificando `settings.toml`.
- L'esistenza di un'istanza in esecuzione non è segreta.
- Senza `$XDG_RUNTIME_DIR` (sessioni senza systemd) l'istanza unica è spenta e si aprono più finestre.

#### B. Clic destro: registro di Windows e script Nautilus

**Asset**

- Integrità di ciò che Explorer o Nautilus eseguono a nome dell'utente: la voce deve portare al programma giusto.
- Integrità del registro dell'utente: nessuna voce residua, nessuna voce di altri programmi toccata.
- Il percorso selezionato (dato non fidato, U3).

**Progetto**

1. Scrittura solo in `HKCU\Software\Classes` (nessun amministratore; manifest `asInvoker`), con `winreg`. Chiavi: `Directory\shell\<nome>`, `Directory\Background\shell\<nome>`, `*\shell\<nome>`, ciascuna con sottochiave `command`; `<nome>` fisso e proprio del programma. Mai scrittura tramite `HKEY_CLASSES_ROOT` (fonde utente e macchina).
2. Valore di `command`: `"<percorso assoluto dell'exe>" --add "%1"`, per `Directory\Background` `"%V\."`: `"%V"` sulla radice di un'unità darebbe `"C:\"`, dove `\"` è una virgoletta letterale per `CommandLineToArgvW` e l'argomento mangerebbe il resto della riga; con `\.` l'argomento resta `C:\\.`, che il guardiano dei percorsi normalizza in `C:\`. Le voci di sfondo scritte con `"%V"` da versioni precedenti sono riconosciute come proprie e «da aggiornare». Il percorso viene da `current_exe()`, mai da `PATH` o dalla cartella corrente, ed è tra virgolette (niente tentativi `C:\Program.exe`). Se contiene `"` o NUL, o la riga supera 260 caratteri, l'opzione non si accende e l'utente ne vede il motivo.
3. `--add` consuma il valore successivo come percorso, anche se inizia con `-`; il parser non interpreta quel valore come opzione.
4. All'avvio, con l'opzione accesa, si legge il comando e se differisce dal percorso corrente si riscrive (exe spostato). Se la chiave esiste ma il comando punta a un programma che non è questo, non si sovrascrive: si avvisa.
5. Allo spegnimento dell'opzione si rimuovono solo le tre chiavi del nome fisso (`delete_subkey_all`), e solo se lo stato è «nostra» o «da aggiornare»: una voce altrui resta. Il README descrive come toglierle a mano (anche per chi cancella il programma senza spegnere l'opzione).
6. Si controllano l'eseguibile e la sua cartella (`exe_writable_by_others`; Windows: proprietario e DACL). La voce non si registra (`MenuError::ExeDirWritable`; eseguirebbe il codice altrui con l'identità dell'utente) se uno dei due ha un proprietario diverso dall'utente, SYSTEM, Administrators o TrustedInstaller (il proprietario può sempre cambiare la DACL), se la DACL è nulla, o se una ACE non solo ereditabile concede scrittura a un SID fuori da questo elenco. OWNER RIGHTS e CREATOR OWNER non sono fidati: sono segnaposto che valgono per chi possiede o crea l'oggetto, anche un altro utente. Le ACE `inherit_only` non contano per l'oggetto stesso. Su Linux: cartella di un altro utente che non sia root, o con scrittura per gruppo o altri.
7. Linux: script in `~/.local/share/nautilus/scripts/<testo della voce>` (il nome del file è il testo, «Cerca con Lynkeus» o «Search with Lynkeus»), creato con `create_new` su un temporaneo (0700, senza seguire collegamenti), `sync_all` e rinomina; contenuto fisso generato dal programma, con una riga marcatore (`SCRIPT_MARKER`) che lo fa riconoscere come proprio. Percorso dell'eseguibile quotato con apici singoli (ogni `'` interno diventa `'\''`); si rifiuta un percorso con NUL o a capo. Contenuto: `exec '<exe>' --add -- "$@"`; senza argomenti (clic sullo sfondo di una cartella) lo script passa la cartella corrente, `$PWD`, tra virgolette dopo `--`. Mai `eval`, mai espansioni senza virgolette. La cartella degli script (creata 0700 se manca) deve essere dell'utente e non scrivibile da altri, altrimenti non si installa. Lo script con il nome dell'altra lingua, se è nostro, si toglie.
8. Il testo della voce segue la lingua scelta (`context-menu-label`). La voce si riallinea all'avvio (`refresh_context_menu`): cambiando lingua a programma aperto, la voce resta nella lingua precedente fino al riavvio. Una voce nostra con un testo diverso conta come «da aggiornare».

**STRIDE**

| STRIDE | Attaccante | Minaccia | Mitigazione | Test previsto |
|---|---|---|---|---|
| S | U2 | Chiave esistente di un altro programma con lo stesso nome | Nome proprio; confronto del comando esistente prima di sovrascrivere | Implementato: `a_foreign_entry_is_left_alone` (registro), `use_cases_respect_foreign_entries_and_writable_folders` e `windows_states`, `script_states` (`context_menu.rs`; `enable` dà `MenuError::Foreign`, `disable` non tocca) e `the_context_menu_setting_follows_the_outcome` (`app.rs`: l'impostazione segue l'esito). Limite: una voce è «nostra» se il comando ha la forma `"<…>\<nome dell'exe>" --add "%1"` con lo stesso nome di file (senza distinguere maiuscole), non per il percorso: un altro programma con lo stesso nome di file e gli stessi argomenti sarebbe riscritto |
| T | U1 | Modifica del comando in `HKCU` | `HKCU` è protetto da ACL di sistema contro altri utenti | Nessun test (garanzia del sistema); un processo dello stesso utente può modificarlo: rischio accettato |
| T/E | U3 | Nome di file che esce dalle virgolette o diventa un'opzione (`"`, `& calc`, `-x`, `--add`), o radice di un'unità che rompe l'ultima virgoletta (`"C:\"`) | `"%1"` tra virgolette (un nome di file Windows non contiene `"`); `"%V\."` per lo sfondo; il valore dopo `--add` è un dato; su Linux `--` e `"$@"` | Implementato: `values_are_data`, `several_adds_and_end_of_options`, `usage_errors` (`cli.rs`: inizia con `-`, spazi, `&`, `%`, `^`, `;`, Unicode, vuoto, 32.000 caratteri); `the_background_command_survives_the_real_command_line_split` (`context_menu.rs`: il comando dello sfondo passa dalla vera divisione della riga di comando, con un figlio di prova, `child_prints_its_arguments`, che stampa gli argomenti), `legacy_background_entries_are_ours_and_outdated`, `the_script_passes_the_current_folder_without_arguments` (script senza argomenti: `$PWD`), `sh_quoting` (`context_menu.rs`); `the_script_passes_paths_verbatim` (solo Linux, `platform/mod.rs`: script generato eseguito con `a'b`, `$(x)`, `-r`, `a b`, a capo e `--add`, un eseguibile finto riceve esattamente `--add`, `--` e i percorsi; permessi 0700 dello script). Non eseguito in locale (Linux solo compilato) |
| T/E | U1, U2 | Exe o cartella dell'exe con proprietario estraneo o scrivibili da altri | Controllo di proprietario e DACL dell'eseguibile e della sua cartella prima di registrare e a ogni riallineamento; OWNER RIGHTS e CREATOR OWNER non fidati | Implementato: `program_rules_on_owners_and_placeholders` (proprietario fidato o no, TrustedInstaller, segnaposto non fidati), `an_exe_writable_by_users_is_detected` (file exe con ACE di scrittura per `Users`), `privacy_rules_on_dacls` (`writable_by_others` con `Users`/`Everyone`: partizioni su DACL costruite a mano, DACL nulla, solo ereditarietà), `an_existing_dir_open_to_others_is_refused` (`dir_writable_by_others` vero con una ACE `Users` creata con `icacls`), `use_cases_respect_foreign_entries_and_writable_folders` (`refresh` dà `ExeDirWritable`, nessuna scrittura). Previsto: prova su una cartella vera con proprietario estraneo (richiede un secondo utente) |
| T | U2 | Percorso con spazi non quotato | Virgolette sempre | Implementato: `commands_are_quoted`, `entry_partitions` (limite di 260 caratteri al confine, `"`, NUL, a capo, relativo) e `registry_entries_are_written_read_and_removed` (valore riletto dal registro con spazi e Unicode nel percorso, `"%1"` e `"%V"`) |
| R | tutti | Voce residua dopo la rimozione del programma | Rimozione allo spegnimento dell'opzione; istruzioni nel README; una voce verso un exe assente produce solo un errore di Explorer | Implementato: `registry_entries_are_written_read_and_removed` (accendi, sposta l'exe → «da aggiornare» e riscritta, spegni → nessuna voce; `own_commands_are_recognized_even_when_moved`). Differenza: la rimozione toglie le tre chiavi `LynkeusSearcher` ma lascia le chiavi padre vuote (`shell` sotto `*`, `Directory` e `Directory\Background`); il registro non torna identico all'originale, resta qualche chiave vuota. Il test controlla che le chiavi `shell` siano vuote |
| I | U2 | Percorso selezionato visibile nella riga di comando | Non evitabile con la shell; i percorsi non sono segreti | Rischio accettato |
| D | U3 | Selezione di migliaia di elementi | Come parte A | Come parte A |
| E | U2 | Tentativo di scrivere in `HKLM` o di elevarsi | Nessuna scrittura in `HKLM`; manifest `asInvoker`; nessuna auto-elevazione | Il manifest (`windows/lynkeus-searcher.manifest`, incorporato da `build.rs`) dichiara `asInvoker` e `uiAccess="false"`; il registro si apre solo con `RegKey::predef(HKEY_CURRENT_USER)` (`ContextMenu::classes`). Previsto: il controllo automatico non c'è. Nessun passo di `cargo xtask check` cerca `HKEY_LOCAL_MACHINE` o legge il manifest; `nospawn` guarda solo `lynkeus-extract/src` |

**Rischi accettati (B)**

- Un processo dello stesso utente può cambiare la voce, come può cambiare l'eseguibile o le impostazioni.
- Su Windows 11 la voce sta in "Mostra altre opzioni" (sezione 5).
- Il percorso selezionato è visibile nella riga di comando ad altri processi dell'utente.
- Cancellando il programma senza spegnere l'opzione restano le tre chiavi (o lo script): Explorer mostra un errore se la si usa. Il README spiega come toglierle a mano.
- La lingua della voce segue quella dell'app solo al riavvio.

#### C. Cartelle private (dati, cache, log, rapporti di crash, temporanei)

**Asset**

- Testo dei documenti nella cache e nei temporanei delle voci aperte (riservatezza).
- Token e nome della pipe, impostazioni, log e rapporti di crash (nessun estratto di testo, 4.7 D).
- Integrità dei file propri: un file scritto da altri non deve passare per valido.

**Progetto**

1. Cartelle (`AppDirs`, `platform/mod.rs`): configurazione (`settings.toml`), dati (lock e file dell'istanza unica), cache, log (`data/logs`), rapporti di crash (`data/crash-reports`) e temporanea delle voci aperte (`data/temp`). Windows: `%APPDATA%\Lynkeus Searcher\config`, `%LOCALAPPDATA%\Lynkeus Searcher\{data,cache}`. Linux: `~/.config/lynkeus-searcher`, `~/.local/share/lynkeus-searcher` (dati, con le stesse sottocartelle) e `~/.cache/lynkeus-searcher`. Tutte create all'avvio da `private_dirs::prepare` con `PrivateFs::create_dir`; le sottocartelle di una cartella dati rifiutata non si creano né si usano.
2. Windows: `CreateDirectoryW` con `SECURITY_ATTRIBUTES` e descrittore esplicito, protetto (nessuna ereditarietà dal padre), con ACE ereditabili per il SID dell'utente (da `TokenUser`, non per nome) e per SYSTEM. Mai la DACL di default (contiene il SID della logon session, effimero) né una DACL nulla. Se la cartella esiste già, non ci si fida: si controllano proprietario e DACL e, se concedono scrittura ad altri, si rifiuta (cache e log spenti per la sessione, avviso all'utente). I file creati ereditano la DACL della cartella; il file del token riceve un descrittore esplicito anche alla creazione.
3. Linux: cartelle 0700 già alla creazione, file con `O_CREAT|O_EXCL|O_NOFOLLOW` e 0600; la `umask` può solo restringere. Se la cartella esiste, controllo su descrittore aperto (`O_DIRECTORY|O_NOFOLLOW`, `fstat`): stesso uid, nessun permesso per gruppo e altri; altrimenti rifiuto, senza `chmod` silenziosi.
4. Ogni file nuovo è `create_new` (`CREATE_NEW` / `O_EXCL`): se esiste, anche come collegamento, la creazione fallisce. Si scrive sull'handle ottenuto, senza riaprire per nome. Nomi derivati da hash o generati dal programma, mai dal documento o dalla voce.
5. La cartella temporanea contiene una sottocartella per istanza, `s-<32 cifre esadecimali>`, con un file `session.lock` bloccato finché l'istanza vive (il sistema lo rilascia anche se il processo muore). All'avvio si cancellano solo le sessioni senza lock tenuto (resti di istanze finite o di un crash) e i file sciolti di versioni precedenti; all'uscita l'istanza cancella la propria. I file di un'altra istanza in corso non si toccano. Un file è grande al più il limite per voce degli archivi (512 MiB predefiniti, 4.7 F), con estensione scelta dal programma e non dal nome della voce; i tipi eseguibili non si aprono (4.6).
6. Le cartelle stanno sotto il profilo dell'utente (`directories`), mai sotto la radice di un volume né in cartelle condivise, dove l'ereditarietà aprirebbe la scrittura ad altri. Manifest `longPathAware`.
7. Log e rapporti di crash riportano motivo e percorso, mai estratti di testo, token o nome della pipe. Log (`diagnostics::init_logging`): `tracing` in `logs/lynkeus-searcher.<data>.log`, un file al giorno, al più 7; livello `warn` predefinito, cambiabile dal menu Strumenti (impostazione `[diagnostics] log_level`); senza cartella dei log (rifiutata o non creata) il log va sullo standard error. Rapporti (`install_crash_reporter`): un panic scrive `crash-<secondi>-<pid>-<n>.txt` creato nuovo e privato, con versione, sistema, thread, posizione (senza le cartelle locali) e messaggio ripulito (testi tra virgolette mascherati; dalla prima parola che sembra un percorso o un nome di file fino a fine riga tutto diventa `<percorso>`, perché un percorso può avere spazi); al più 10, i più vecchi cancellati prima di scrivere; poi il gestore precedente prosegue.

**STRIDE**

| STRIDE | Attaccante | Minaccia | Mitigazione | Test previsto |
|---|---|---|---|---|
| S | U1 | Cartella pre-creata da altri con lo stesso nome, per far scrivere lì i dati | Cartelle nel profilo; se esiste, controllo di proprietario e permessi; rifiuto | Implementato: Windows `an_existing_dir_open_to_others_is_refused` (ACE per `Users` aggiunta con `icacls`: `NotPrivate`, DACL non modificata); Linux `unix_permissions_and_refusals` (0777 rifiutata, permessi invariati; collegamento al posto della cartella rifiutato); regole sulle DACL: `privacy_rules_on_dacls`; effetto sull'app: `a_refused_cache_turns_off_only_the_cache`, `a_refused_data_dir_turns_off_what_is_inside`, `other_errors_turn_off_without_a_refusal` (`private_dirs.rs`) |
| T | U1, U2 | Voce di cache o impostazioni alterata | Intestazione con versione e CRC, chiave `blake3`; cartella inaccessibile a U1 (U2 con gli stessi diritti: rischio accettato) | Implementato (4.7 E): `corrupt_entries_are_ignored_and_removed` |
| T | U2, U3 | File preesistente, collegamento simbolico o hard link al posto del nome atteso | `create_new` non segue né sovrascrive; nomi da hash | Implementato: `private_files_are_new_only` (engine) e `private_files_have_an_explicit_dacl_and_are_new_only` (Windows: seconda creazione = `AlreadyExists`); Linux `unix_permissions_and_refusals` (collegamento simbolico al posto del file: creazione fallita, nulla creato altrove). Previsto: collegamento o junction preesistente su Windows |
| T | U3 | Nome di voce d'archivio usato come nome di file | Nome generato dal programma | `malevoli/percorsi.zip` (4.7 C) |
| R | tutti | Nessuna traccia di chi ha scritto | Non rilevante: file locali di un solo utente; il log non è un registro di audit | |
| I | U1 | Lettura di cache, temporanei, impostazioni, log | Descrittore esplicito utente+SYSTEM (Windows), 0700 e 0600 (Linux) | Implementato su Windows: `private_dirs_have_an_explicit_protected_dacl` (DACL letta con le API di sistema dopo la creazione: protetta, proprietario l'utente, solo utente e SYSTEM con accesso completo ereditabile da file e sottocartelle, anche per la cartella intermedia creata) e `private_files_have_an_explicit_dacl_and_are_new_only`. Linux: `unix_permissions_and_refusals` (0700 e 0600, non eseguito in locale). Previsto: prove con `umask` 0 e 077 e un controllo con `icacls` sull'app vera |
| I | U2 | Lettura da un processo a integrità Low o con token ristretto | La DACL nomina solo l'utente; se un processo Low con lo stesso SID legge ancora, la risposta è un'etichetta di integrità Medium (`NO_READ_UP`), da adottare solo se la prova la mostra utile | Prova con un processo avviato a integrità Low; esito registrato qui |
| I | U1 | Contenuto dei documenti nei log o nei rapporti di crash | Solo motivo e percorso | Implementato: `redaction_partitions`, `paths_with_spaces_and_bare_file_names_are_masked` e `source_files_lose_local_folders` (`diagnostics.rs`: percorsi con spazi, nomi di file nudi, testi tra virgolette, parole che sembrano percorsi, lunghezza, cartelle locali del sorgente). Previsto: file con testo riconoscibile, poi ricerca nel log e nei rapporti (4.7 D) |
| D | U2, U3 | Crescita illimitata di log, rapporti, temporanei | LRU per la cache (500 MB); rotazione dei log; numero massimo di rapporti (10); temporanei in sottocartelle per istanza, puliti all'avvio solo se senza lock | Cache implementato (4.7 E). Rapporti: `at_most_ten_reports_are_kept` (`diagnostics.rs`). Log: rotazione giornaliera con al più 7 file delegata a `tracing-appender` (nessun test proprio). Temporanei: `clean_stale_sessions` (4.7 F). Differenza: il file di log lo crea `tracing-appender`, non `PrivateFs::create_new_file`; su Linux ha i permessi secondo la `umask` (di solito 0644), ma la cartella 0700 lo rende inaccessibile ad altri; su Windows eredita la DACL della cartella |
| D | U1 | Disco pieno o nomi occupati nella cartella | Cartella inaccessibile ad altri; errore di scrittura = cache spenta, la ricerca continua | Previsto: scrittura che fallisce, ricerca completata (vale per la cache: 4.7 E; il rapporto di crash non scritto si registra e basta) |
| E | U1, U3 | Un file della cartella viene eseguito o caricato | Il programma non esegue né carica nulla dalle cartelle dati; i tipi eseguibili non si aprono | Implementato: `executable_name_partitions`, `plan_partitions` |

**Rischi accettati (C)**

- Chi esegue codice con i diritti dell'utente legge cache, token e temporanei (sezione 5).
- Backup del profilo, copie shadow, indicizzatori e cartelle sincronizzate possono copiare la cache: l'avviso nel README lo dice.
- Dopo un crash una voce di cache può essere incompleta: la legge il CRC, che la scarta.
- Su file system senza ACL né permessi (FAT, exFAT) il descrittore non si applica: il programma lo rileva, avvisa e spegne la cache.

#### Limiti noti e differenze dal progetto (4.8)

Pipe, socket, lock, token, registro, script Nautilus e descrittori espliciti di Windows sono nel codice (vedi i test nelle tabelle). Restano:

- **Linux solo compilato in locale:** socket, `SO_PEERCRED`, permessi 0700/0600 e `the_script_passes_paths_verbatim` li esegue solo la CI.
- **Permessi dei log su Linux** secondo la `umask` (il file lo crea `tracing-appender`); li protegge la cartella 0700.
- **Lingua della voce del clic destro** aggiornata solo al riavvio.
- **Registro:** allo spegnimento restano le chiavi padre vuote.
- **Voce riconosciuta come propria** dal nome del file eseguibile e dagli argomenti, non dal percorso.
- **Tracce non ancora provate da test:** client lento, 1.000 connessioni, server di un altro utente (SID diverso), collegamenti preesistenti su Windows, `umask` 0 e 077, ricerca del token e di testo dei documenti nei log, controllo automatico di `HKEY_LOCAL_MACHINE` e del manifest.
- Linux: `recursive(true)` applica 0700 anche alle cartelle intermedie create (ammesso).
- Il commento del test `an_existing_dir_open_to_others_is_refused` dice «la voce estranea è stata tolta» ma l'asserzione verifica che sia rimasta: il messaggio è fuorviante, il controllo è giusto.
- FAT/exFAT (parte C, rischi accettati): il rilevamento descritto non è nel codice né provato da test.

## 5. Rischi accettati

- **Eseguibile non firmato:** progetto personale senza certificato di firma del codice. Compensazione: `SHA256SUMS` e istruzioni nel README; SmartScreen mostrerà un avviso.
- **Voce del clic destro in "Mostra altre opzioni" su Windows 11:** scelta concordata.
- **Processo estrattore con gli stessi diritti dell'utente:** accettato (ADR 0001). Un bug di esecuzione di codice in un decoder permette di leggere i file dell'utente e scrivere dove l'utente scrive; i limiti di risorse non lo impediscono. Compensazione: decoder in Rust sicuro, ambiente e handle minimi, nessuna rete richiesta. Da rivalutare ora che la fase 5 ha aggiunto decoder di terzi con `unsafe` (`mail-parser`, `lzma-rust2` tramite `sevenz-rust2`; vedi 4.7, limiti noti): finora la scelta è di mantenere l'isolamento attuale. Isolamento più forte da valutare se si aggiunge un decoder in codice C o `unsafe` non coperto (AppContainer o token ristretto su Windows, Landlock su Linux).
- **Cache leggibile da chi ha già i diritti dell'utente:** non si difende dallo stesso utente o da malware con i suoi diritti.
- **File che cambia durante la lettura o aperto da un altro programma:** testo incoerente o avviso "file in uso"; nessun blocco del file.
- **Tempo di rilevamento:** un decoder in ciclo occupa un processo fino a 60 s prima del kill.
- **PDF scansionati e .rar non letti:** limite funzionale, non di sicurezza.
