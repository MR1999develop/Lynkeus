# Threat model (prima versione)

Metodo: le Quattro Domande e STRIDE. Il processo estrattore e gli archivi sono approfonditi (fase 4, sezione 4.7); la pipe dell'istanza unica e il registro sono ancora provvisori e verranno approfonditi nella fase 7.

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
- **D**: processo bloccato o vorace. Mitigazione: timeout di 60 s, Job Object con limite di memoria e `KILL_ON_JOB_CLOSE` (Windows), `RLIMIT_AS` (Linux), uscita alla chiusura dello stdin, pool limitato a `min(core/2, 4)`.
- **E**: compromissione del figlio. Mitigazione: risorse limitate e figlio senza scritture sui dati dell'utente; isolamento più forte da valutare nell'ADR.

### 4.3 Pipe e socket dell'istanza unica (da approfondire nella fase 7)

- **S**: processo ostile che si spaccia per la prima istanza o per un client. Mitigazione: pipe con nome casuale creata con `FILE_FLAG_FIRST_PIPE_INSTANCE`; client con `SECURITY_IDENTIFICATION`; token casuale da 128 bit in un file privato, confronto in tempo costante.
- **T/E**: percorsi malevoli iniettati. Mitigazione: dimensione massima dei messaggi, solo percorsi esistenti, aggiunti all'elenco senza avviare la ricerca.
- **I/D**: altri utenti sul computer. Mitigazione: DACL esplicita solo per l'utente corrente (Windows), socket 0600 in `$XDG_RUNTIME_DIR` (Linux); attivo solo con `--add`.

### 4.4 Registro e script Nautilus

- **T/E**: voce del registro che punta a un eseguibile modificato. Mitigazione: scrittura solo in `HKCU` (nessun amministratore); percorso dell'eseguibile aggiornato all'avvio e voce rimossa quando l'opzione è spenta.
- **T**: percorsi con caratteri speciali nella riga di comando. Mitigazione: argomento `"%1"` tra virgolette, trattato come dato.

### 4.5 Cartella dati e cache

- **I**: la cache contiene testo dei documenti. Mitigazione: cartelle private (descrittore esplicito solo utente e SYSTEM su Windows; 0700 e 0600 su Linux), cache spegnibile e svuotabile, avviso nel README.
- **T**: voci di cache alterate o corrotte. Mitigazione: intestazione con versione e CRC; voce non valida ignorata e cancellata; creazione dei file che fallisce se esistono.
- **D**: crescita illimitata. Mitigazione: limite LRU di 500 MB.

### 4.6 File esportati

- **T/E**: iniezione di formule nei CSV aperti con un foglio di calcolo (testo che inizia con `=`, `+`, `-`, `@`, tabulazione o CR). Mitigazione (`crates/lynkeus-app/src/export.rs`): `neutralize_formula` antepone un apostrofo a ogni cella che inizia con uno di questi caratteri, in CSV, TXT e TSV degli appunti; nei TXT e TSV la neutralizzazione avviene dopo l'appiattimento di tabulazioni e a capo. Verifica: test `formula_partitions`, `formulas_are_neutralized_in_every_format` e proptest `csv_round_trip_any_cells` (nessuna cella esportata inizia con un carattere a rischio). Limite: l'apostrofo resta visibile se il file è riletto da un programma che non è un foglio di calcolo.
- **T/E**: "Apri" su un risultato che sia un programma o uno script (doppio clic su `pulisci.bat` in una cartella cercata). Mitigazione (`crates/lynkeus-app/src/launch.rs`): `open_plan` non apre i file con estensione eseguibile (`exe`, `bat`, `cmd`, `ps1`, `vbs`, `js`, `lnk`, `url`, `sh`, `py` e simili, elenco largo; punti e spazi finali e maiuscole non contano) e propone solo "Mostra nella cartella"; per una voce d'archivio si guarda il nome della voce. I file si aprono con il programma predefinito del sistema, mai con un programma cercato per nome, e con percorso assoluto. Verifica: test `executable_name_partitions` e `plan_partitions`. Limite: l'elenco è per estensione; un tipo di file con un gestore che esegue codice (per esempio un documento con macro) si apre come il sistema stabilisce.
- **I**: l'esportazione contiene contesto dei documenti: scelta esplicita dell'utente.
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
| Annulla | stop (stdin chiuso), kill dopo 500 ms |
| Processi del pool | `min(core/2, 4)` |
| Cache su disco | 500 MB, LRU |
| Blocco di lettura dei file di testo | circa 8 MB, tagliato a fine riga |
| Memoria per processo estrattore | 1 GiB (`DEFAULT_MEMORY_LIMIT`; minimo accettato dal figlio 64 MiB). Windows: memoria impegnata per processo, Job Object. Linux: `RLIMIT_AS` imposto dal figlio |
| Dimensione massima di un messaggio del protocollo | 16 MiB (`MAX_FRAME_BYTES`) |
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
| T | Dati malformati: lettura fuori buffer, cicli, allocazioni da numeri dichiarati | Rust sicuro (`forbid(unsafe_code)` in `lynkeus-extract`); parser propri (.doc, .ppt, RTF) con lunghezze e offset confrontati con i byte presenti prima di allocare (`checked_*`, `try_from`); errore di parsing = avviso "danneggiato" | `malevoli/troncato.pdf`: avviso, nessun blocco. Fuzzing dei parser propri con cargo-fuzz (`fuzz/`: `doc_piece_table`, `ppt_records`, `rtf`, `protocol_frame`; vedi `fuzz/README.md`); i parser di .doc, .ppt e RTF sono implementati, il corpus parte da `formati/`. Gli esiti delle sessioni di fuzzing non sono registrati qui |
| T | XML con DTD ed entità annidate (billion laughs, XXE) | `quick-xml` senza espansione; `<!DOCTYPE>` rifiutato nei pacchetti (OOXML, ODF, file del pacchetto EPUB). Unica eccezione, i capitoli XHTML degli EPUB: un `<!DOCTYPE>` senza sottoinsieme interno (`<!DOCTYPE html>`, DTD pubblici) è ignorato, uno con sottoinsieme interno (quello che definisce entità) è rifiutato; si risolvono solo le entità predefinite e le entità HTML comuni da una tabella fissa; nessun XInclude né risorsa esterna | `malevoli/entita.docx`: avviso, nessuna espansione, memoria entro il limite |
| D | Decoder che non termina o consuma memoria | Timeout di 60 s con kill e nuovo processo; limite di memoria del processo; flussi con `Read::take` | Decoder finto che dorme e decoder finto che alloca (test del pool); `malevoli/bomba.zip`. Implementato: `misbehaving_workers_are_replaced_and_reported` (lento = `Timeout`), `malicious_files_give_warnings_without_blocking` (`bomba.zip`), `exceeding_the_memory_limit_kills_and_is_recognized` (Job Object, Windows) |
| D | Panic o abort del decoder abbattono l'app | Il decoder gira nel figlio: la sua morte dà "danneggiato" per quel file, il pool ne avvia un altro, la ricerca continua | Test: il figlio esce a metà di un file, gli altri file del lotto sono elaborati. Implementato: `misbehaving_workers_are_replaced_and_reported` ("muore": `Corrupt`, `note.docx` letto con un processo nuovo) |
| I | Il decoder legge altro o apre la rete | Il figlio riceve il percorso e nulla di più; nessuna dipendenza che usi la rete (`cargo deny` e revisione); ambiente svuotato | Controllo delle dipendenze in CI; test sulle variabili d'ambiente del figlio |
| I | Testo protetto da password | Avviso "protetto da password", nessun tentativo di indovinare | `formati/protetto-password.docx`: avviso, nessun testo |
| E | Esecuzione di codice (macro, OLE, JavaScript nei PDF, collegamenti esterni) | Si estrae solo testo; macro, oggetti incorporati e collegamenti ignorati; nessun avvio di programmi | Controllo in CI: nessun uso di `std::process::Command` nei moduli dei decoder (il pool che avvia il figlio ne è escluso). Previsto: il controllo non c'è ancora; il solo `Command` è in `launcher.rs` dell'engine |
| E | Compromissione del figlio per un bug | Risorse limitate, nessuna scrittura sui dati dell'utente, ambiente e handle minimi; il resto è rischio accettato (sezione 5) | Sezione D e ADR 0001 |

#### C. Archivi e allegati annidati (zip, tar, gz, 7z, docx, odt, EPUB, EML)

| STRIDE | Minaccia | Mitigazione | Test previsto |
|---|---|---|---|
| T | Zip slip: nomi con `..`, assoluti, unità, UNC, separatori dell'altro sistema | I nomi sono solo etichette, mai usati per scrivere. Se per aprire una voce serve un file, nome generato dal programma nella cartella temporanea privata | `malevoli/percorsi.zip`: nessun file creato fuori dalla cartella di lavoro (elenco dei file prima e dopo) |
| T | Link simbolici e voci speciali (tar, 7z) | Solo voci regolari; le altre ignorate e contate; nessun link seguito | Implementato (test che costruiscono l'archivio, nessun file in `malevoli/`): `symlink_entry_is_skipped` (zip), `tar_skips_links_and_counts_entries`, `special_entries` (7z) |
| D | Zip bomb: rapporto, byte per voce e per archivio | `Read::take` sul flusso; 200:1, 512 MB per voce, 4 GB per archivio, controllati durante la lettura | `malevoli/bomba.zip` (circa 1000:1): avviso "limite superato", memoria limitata |
| D | Troppe voci | Conteggio sull'elenco delle voci prima di decomprimere: oltre 100.000, avviso | `malevoli/molte-voci.zip` (da generare con `--vuote 100001`, non committato) |
| D | Annidamento profondo o circolare | Profondità massima 5 con contatore passato a ogni livello; vale anche per allegati EML e archivi dentro docx | `malevoli/annidato-profondo.zip` (7 livelli): avviso, `fondo.txt` non raggiunto. `formati/annidato.zip` (3 livelli): letto |
| D | Livelli ciascuno sotto il limite ma sopra in totale | Byte e voci contati sull'intero archivio radice, non per livello | Implementato: `total_size_counts_all_levels` (`tests/archives.rs`); `archives_nest_inside_zip_with_depth_limit` (`tests/formats.rs`); allegati EML: `eml_attachments_follow_the_archive_limits`; gz: `gz_bomb_hits_ratio_limit` |
| I | Nomi con caratteri di controllo o bidi che ingannano chi legge i risultati | Etichette mostrate con i caratteri di controllo sostituiti | Test unitario sull'etichetta |
| S | Voce con estensione che mente dentro l'archivio | Stessa verifica del passo A su ogni voce | Implementato: `lying_extension_inside_archive` (`tests/archives.rs`, usa `malevoli/immagine-finta.pdf` dentro uno zip) |

#### D. Protocollo verso il processo principale (stdin e stdout del figlio)

| STRIDE | Minaccia | Mitigazione | Test previsto |
|---|---|---|---|
| S | Avvio di un eseguibile diverso | Percorso assoluto di `current_exe()`, mai `PATH` né cartella corrente | Previsto e implementato: `relative_executables_are_refused` e `the_command_has_a_minimal_environment` (`launcher.rs`), `relative_paths_are_refused` (`windows_ffi.rs`) |
| T | Messaggio alterato, troncato o con lunghezza falsa | Lunghezza in testa confrontata con il massimo prima di allocare; `postcard` in tipi concreti; violazione = figlio terminato e sostituito | Implementato: `every_truncation_is_detected`, `oversized_length_is_rejected_before_reading`, `short_length_wrong_version_and_trailing_bytes`, `oversized_message_is_not_written`, `random_bytes_never_panic` (proptest), `events_round_trip` (`protocol.rs`); lato figlio `malformed_frame_is_rejected_then_exit` e `truncated_request_is_rejected` (`worker.rs`). Previsto: fuzzing del decodificatore |
| T | Il figlio scrive su stdout cose estranee al protocollo | Stdout riservato al protocollo; log solo su stderr, letto con tetto | Decoder finto che stampa su stdout: figlio sostituito. Implementato: "sporco" in `misbehaving_workers_are_replaced_and_reported` |
| R | Non rilevante | | |
| I | Il figlio riceve più del necessario | `env_clear` salvo poche variabili di sistema; nessun handle ereditato oltre a stdin, stdout, stderr (lista esplicita su Windows, chiusura dei descrittori su Linux) | Implementato: ambiente (`the_child_sees_only_system_root`, Windows; `the_command_has_a_minimal_environment`). Previsto: test sugli handle ereditati (la `HANDLE_LIST` è impostata ma nessun test conta gli handle del figlio) |
| I | Contenuto dei documenti nei log o nei rapporti di crash | Gli errori portano motivo e percorso, mai estratti di testo | Previsto: test sul log dopo un file con testo riconoscibile (il figlio ha stderr nullo e non scrive log) |
| D | Figlio muto, che inonda stdout o ignora l'annulla | Timeout 60 s con kill; lettura con tetto; annulla = stdin chiuso e kill dopo 500 ms; `KILL_ON_JOB_CLOSE` (Windows) e uscita alla chiusura dello stdin (Linux) evitano orfani se l'app muore | Implementato: figlio muto con timeout ("lento"), annulla entro 1 s (`cancel_stops_a_busy_worker_within_one_second`), chiusura del job che termina il figlio (`closing_the_job_kills_the_child`), `terminate_kills_and_is_idempotent`. Previsto: figlio che scrive senza fine; app terminata con figli vivi su Linux |
| E | Un figlio compromesso chiede al padre cose non previste | Il padre accetta solo i messaggi previsti nello stato corrente (richiesta, risposta, fine); nessun comando dal figlio al padre; i risultati sono solo testo da cercare | Implementato: "fuori-sequenza" e "altro-id" in `misbehaving_workers_are_replaced_and_reported`; `sequence_partitions` (`event.rs`) |

#### E. Cache su disco

| STRIDE | Minaccia | Mitigazione | Test previsto |
|---|---|---|---|
| I | Altri utenti o processi leggono il testo dei documenti | Descrittore esplicito (solo utente e SYSTEM) su Windows, 0700 e 0600 su Linux; cache spegnibile e svuotabile; avviso nel README | Previsto: verifica di ACL e permessi dopo la creazione (fase 7: su Windows la cartella eredita ancora i permessi dell'utente, descrittore esplicito non ancora impostato) |
| T | Voce alterata, troncata o di versione diversa | Intestazione con versione e CRC; voce non valida ignorata e cancellata; stessi limiti di lunghezza del protocollo | Implementato: `corrupt_entries_are_ignored_and_removed` (CRC, troncata, versione futura, lunghezza enorme, altra chiave, spazzatura), `entries_with_a_bad_event_sequence_are_corrupt`, `entries_round_trip` (byte casuali) e `second_search_reads_from_the_cache_and_corrupt_entries_are_dropped` |
| T | Voce valida per la chiave ma con testo diverso dal documento | Chiave `blake3` di percorso, dimensione, data di modifica, versione degli estrattori e limiti. Chi scrive nella cache ha già i diritti dell'utente: rischio accettato | Implementato: `key_partitions` (percorso, dimensione, data, limiti); la versione del programma e del protocollo sono nell'hash ma non hanno un test proprio |
| T | Scrittura a metà, nomi pericolosi, file preesistenti | Scrittura su temporaneo e rinomina; creazione che fallisce se il file esiste; nomi dei file derivati dall'hash, mai dal nome del documento o della voce | Implementato: nome esadecimale in `key_partitions`. Previsto: interruzione a metà scrittura |
| D | Crescita illimitata | LRU a 500 MB; una voce che da sola non entra non si salva | Implementato: `least_recently_used_entries_are_evicted`, `an_entry_larger_than_the_cache_is_not_saved` |
| D | Dimensione decompressa `lz4_flex` dichiarata falsa | Dimensione dichiarata nell'intestazione confrontata con il massimo prima di decomprimere | Implementato: lunghezza dichiarata `u32::MAX` in `corrupt_entries_are_ignored_and_removed` |

#### Limiti noti di 4.7

- **Linux: memoria superata non distinta da un aborto.** `RLIMIT_AS` fa fallire le allocazioni e il figlio abortisce: il padre vede `WorkerExit::Failed` e dà l'avviso "danneggiato" invece di "limite superato (memoria)". Su Windows il Job Object (messaggio sulla porta di completamento, o picco entro 1/16 dal limite) lo distingue. Il limite è sulla memoria impegnata/virtuale, non sul working set.
- **Voci d'archivio non di testo non ancora apribili.** Il protocollo porta solo testo: una voce PDF o Office dentro un archivio si trova e si mostra in anteprima, ma "Apri" dà il messaggio di non disponibilità (servono i byte originali della voce). Le voci di testo si ricompongono in un file della cartella privata (al più 64 MiB).
- **Decoder di terzi con `unsafe`.** `lynkeus-extract` ha `forbid(unsafe_code)`, ma due dipendenze della fase 5 contengono `unsafe` nel proprio codice: `mail-parser` (EML, e HTML ridotto a testo) e, per il 7z, `lzma-rust2` (dipendenza di `sevenz-rust2`, che di suo non ha blocchi `unsafe`). Il processo estrattore resta l'unica difesa (sezione 5: rischio da rivalutare). Il conteggio è in `docs/dipendenze.md`.
- **Controllo "nessun `Command` nei decoder" e test degli handle ereditati ancora da scrivere.**
- **Windows: cartelle private con permessi ereditati** fino alla fase 7.

## 5. Rischi accettati

- **Eseguibile non firmato:** progetto personale senza certificato di firma del codice. Compensazione: `SHA256SUMS` e istruzioni nel README; SmartScreen mostrerà un avviso.
- **Voce del clic destro in "Mostra altre opzioni" su Windows 11:** scelta concordata.
- **Processo estrattore con gli stessi diritti dell'utente:** accettato (ADR 0001). Un bug di esecuzione di codice in un decoder permette di leggere i file dell'utente e scrivere dove l'utente scrive; i limiti di risorse non lo impediscono. Compensazione: decoder in Rust sicuro, ambiente e handle minimi, nessuna rete richiesta. Da rivalutare ora che la fase 5 ha aggiunto decoder di terzi con `unsafe` (`mail-parser`, `lzma-rust2` tramite `sevenz-rust2`; vedi 4.7, limiti noti): finora la scelta è di mantenere l'isolamento attuale. Isolamento più forte da valutare se si aggiunge un decoder in codice C o `unsafe` non coperto (AppContainer o token ristretto su Windows, Landlock su Linux).
- **Cache leggibile da chi ha già i diritti dell'utente:** non si difende dallo stesso utente o da malware con i suoi diritti.
- **File che cambia durante la lettura o aperto da un altro programma:** testo incoerente o avviso "file in uso"; nessun blocco del file.
- **Tempo di rilevamento:** un decoder in ciclo occupa un processo fino a 60 s prima del kill.
- **PDF scansionati e .rar non letti:** limite funzionale, non di sicurezza.
