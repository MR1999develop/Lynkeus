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
- **D**: processo bloccato o vorace. Mitigazione: timeout di 60 s, Job Object con limite di memoria e `KILL_ON_JOB_CLOSE` (Windows), `RLIMIT_AS` (Linux), uscita alla chiusura dello stdin, pool limitato a `min(core/2, 4)`.
- **E**: compromissione del figlio. Mitigazione: risorse limitate e figlio senza scritture sui dati dell'utente; isolamento più forte da valutare nell'ADR.

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
- **Windows: cartelle private con permessi ereditati** fino alla fase 7 (vedi 4.8, parte C).

### 4.8 Dettaglio: istanza unica, clic destro e cartelle private (fase 7)

Approfondisce 4.3, 4.4 e 4.5 (confini 3, 4 e 5). Stato del codice alla stesura: esistono `create_private_dir` e `create_new_private_file` (`crates/lynkeus-engine/src/platform.rs`); su Linux impostano 0700 e 0600, su Windows non impostano alcun descrittore (si eredita quello del padre). Pipe, socket, lock, token, voci di registro e script Nautilus non sono ancora nel codice: ciò che segue è progetto, e ogni test indicato è "Previsto" salvo dove è scritto "Implementato". Una mitigazione senza test implementato non è verificata.

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
6. Messaggio: versione, token (16 byte), numero di percorsi, percorsi come testo. Tetto alla dimensione totale (proposta 64 KiB) e al numero di percorsi (proposta 256), controllati prima di allocare; timeout di lettura (proposta 2 s); una connessione servita alla volta. Il token si confronta in tempo costante (`subtle`); a token errato la connessione si chiude senza risposta.
7. Il server accetta un percorso solo se è assoluto, senza NUL, non UNC né dispositivo (`\\?\`, `\\.\`), e se esiste (`symlink_metadata` riuscito). Lo aggiunge all'elenco dei luoghi senza avviare la ricerca e senza aprirlo. Il percorso è un dato: mai a una shell o a un programma. Una sola funzione di validazione per riga di comando e canale (un solo guardiano per risorsa).
8. Il client, consegnati i percorsi, chiama `AllowSetForegroundWindow(pid della prima istanza)`, chiede il primo piano ed esce. Se il server non risponde entro il timeout, il client apre una propria finestra: nessun blocco, nessun tentativo infinito.
9. Il server esiste solo se l'opzione del clic destro è accesa (per piano, canale "solo con `--add`"); con l'opzione spenta non c'è canale né lock di istanza.

**STRIDE**

| STRIDE | Attaccante | Minaccia | Mitigazione | Test previsto |
|---|---|---|---|---|
| S | U1, U2 | Processo che crea per primo la pipe con il nome atteso e raccoglie token e percorsi | Nome casuale a ogni avvio, noto solo dal file privato; `FILE_FLAG_FIRST_PIPE_INSTANCE`; il client verifica che il server sia dell'utente prima di mandare il token | Creazione con nome già occupato: fallisce e si cambia nome; funzione che confronta il SID del server con quello dell'utente, con SID diverso: rifiuto |
| S | U1, U2 | Processo che si spaccia per client | DACL solo utente (esclude U1); token da 128 bit nel file privato; `SO_PEERCRED` su Linux | Connessione senza token, con un byte sbagliato, con lunghezza diversa: rifiutate, nessun percorso aggiunto; confronto in tempo costante (revisione del codice: nessun `==` sul token) |
| T | U3 | Percorso con NUL, UNC, `\\?\`, `..`, inesistente, collegamento verso altrove | Validazione unica; normalizzazione lessicale con `platform::normalize`; UNC rifiutato (evita l'autenticazione verso un host scelto da altri); nessuna apertura | Partizioni: relativo, assoluto, NUL, UNC, `\\?\`, `\\.\`, inesistente, 32.000 caratteri, collegamento circolare |
| T | U2 | Messaggio troncato, con conteggi o lunghezze false | Tetti prima dell'allocazione, timeout di lettura | Ogni troncamento, lunghezza `u32::MAX`, proptest su byte casuali (nessun panic, nessuna allocazione oltre il tetto) |
| R | tutti | Negare di aver mandato un percorso | Non rilevante: effetto limitato a una voce in più nell'elenco | |
| I | U1, U2 | Lettura del token o del nome della pipe | File privato (parte C); mai in log, riga di comando, rapporti di crash | Ricerca del token nel log e nei rapporti dopo un ciclo completo |
| I | U1 | Elencare le pipe per scoprire un'istanza attiva | Non evitabile su Windows; nome casuale e DACL limitano l'uso | Rischio accettato |
| D | U1, U2 | Connessioni che non scrivono, riempimento della pipe, blocco dell'unica istanza | Una connessione alla volta con timeout; `PIPE_REJECT_REMOTE_CLIENTS`; tetto ai percorsi; client con ripiego su finestra propria | Client lento che non scrive: il server riprende dopo il timeout e serve il successivo |
| D | U3 | Selezione di migliaia di elementi: Explorer lancia un processo per elemento | Processi brevi (connetti, manda, esci); percorsi aggiunti in blocco senza ricerca; elenco con tetto | 1.000 connessioni in sequenza: memoria e tempo entro limite |
| D | U1 | Occupare il file di lock o il nome del socket | Lock nella cartella privata (inaccessibile a U1); `$XDG_RUNTIME_DIR` scrivibile solo dall'utente | Lock o socket preesistenti con altro proprietario: funzione spenta, nessun blocco dell'avvio |
| E | U1 | Impersonazione del client da parte di un server ostile | SQoS Identification nel client; il server legittimo non impersona | Test sulla funzione che costruisce i flag di apertura: `SECURITY_SQOS_PRESENT` e `SECURITY_IDENTIFICATION` entrambi presenti |
| E | U2 | Usare il canale come deputato confuso per far leggere file | Un percorso aggiunto non si legge finché l'utente non preme Cerca; il canale non porta diritti né comandi diversi da "aggiungi" | Nessuna ricerca avviata dopo un messaggio valido (test sull'engine) |

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
2. Valore di `command`: `"<percorso assoluto dell'exe>" --add "%1"`, per `Directory\Background` `"%V"`. Il percorso viene da `current_exe()`, mai da `PATH` o dalla cartella corrente, ed è tra virgolette (niente tentativi `C:\Program.exe`). Se contiene `"` o NUL, o la riga supera 260 caratteri, l'opzione non si accende e l'utente ne vede il motivo.
3. `--add` consuma il valore successivo come percorso, anche se inizia con `-`; il parser non interpreta quel valore come opzione.
4. All'avvio, con l'opzione accesa, si legge il comando e se differisce dal percorso corrente si riscrive (exe spostato). Se la chiave esiste ma il comando punta a un programma che non è questo, non si sovrascrive: si avvisa.
5. Allo spegnimento dell'opzione si rimuovono solo le tre chiavi del nome fisso (`delete_subkey_all`). Il README descrive come toglierle a mano.
6. Se la cartella dell'exe è scrivibile da gruppi diversi da utente, Administrators e SYSTEM, la voce non si registra (la voce eseguirebbe il codice altrui con l'identità dell'utente); la DACL si legge con le API di sistema.
7. Linux: script in `~/.local/share/nautilus/scripts/`, creato con `create_new` e 0700, contenuto fisso generato dal programma. Percorso dell'eseguibile quotato con apici singoli (ogni `'` interno diventa `'\''`); si rifiuta un percorso con NUL o a capo. Contenuto: `exec '<exe>' --add -- "$@"`. Mai `eval`, mai espansioni senza virgolette. La cartella degli script deve essere dell'utente e non scrivibile da altri, altrimenti non si installa; aggiornamento con temporaneo e rinomina.

**STRIDE**

| STRIDE | Attaccante | Minaccia | Mitigazione | Test previsto |
|---|---|---|---|---|
| S | U2 | Chiave esistente di un altro programma con lo stesso nome | Nome proprio; confronto del comando esistente prima di sovrascrivere | Chiave con comando estraneo: non sovrascritta, avviso |
| T | U1 | Modifica del comando in `HKCU` | `HKCU` è protetto da ACL di sistema contro altri utenti | Nessun test (garanzia del sistema); un processo dello stesso utente può modificarlo: rischio accettato |
| T/E | U3 | Nome di file che esce dalle virgolette o diventa un'opzione (`"`, `& calc`, `-x`, `--add`) | `"%1"` tra virgolette (un nome di file Windows non contiene `"`); il valore dopo `--add` è un dato; su Linux `--` e `"$@"` | Partizioni sul parser degli argomenti (inizia con `-`, spazi, `&`, `%`, `^`, `;`, Unicode, vuoto, 32.000 caratteri); su Linux script generato eseguito con file chiamati `a'b`, `$(x)`, `-r`, `a b`, con a capo: un eseguibile finto riceve esattamente il percorso |
| T/E | U1, U2 | Exe in cartella scrivibile da altri | Controllo della DACL della cartella dell'exe prima di registrare | Exe in cartella con ACE di scrittura per `Users`: opzione rifiutata con motivo |
| T | U2 | Percorso con spazi non quotato | Virgolette sempre | Lettura del valore scritto e confronto con il formato atteso, con spazi e Unicode nel percorso |
| R | tutti | Voce residua dopo la rimozione del programma | Rimozione allo spegnimento dell'opzione; istruzioni nel README; una voce verso un exe assente produce solo un errore di Explorer | Ciclo accendi-spegni: registro identico all'originale (confronto delle esportazioni) |
| I | U2 | Percorso selezionato visibile nella riga di comando | Non evitabile con la shell; i percorsi non sono segreti | Rischio accettato |
| D | U3 | Selezione di migliaia di elementi | Come parte A | Come parte A |
| E | U2 | Tentativo di scrivere in `HKLM` o di elevarsi | Nessuna scrittura in `HKLM`; manifest `asInvoker`; nessuna auto-elevazione | Controllo del manifest e dei percorsi di registro nel codice (ricerca di `HKEY_LOCAL_MACHINE` in `cargo xtask check`) |

**Rischi accettati (B)**

- Un processo dello stesso utente può cambiare la voce, come può cambiare l'eseguibile o le impostazioni.
- Su Windows 11 la voce sta in "Mostra altre opzioni" (sezione 5).
- Il percorso selezionato è visibile nella riga di comando ad altri processi dell'utente.

#### C. Cartelle private (dati, cache, log, rapporti di crash, temporanei)

**Asset**

- Testo dei documenti nella cache e nei temporanei delle voci aperte (riservatezza).
- Token e nome della pipe, impostazioni, log e rapporti di crash (nessun estratto di testo, 4.7 D).
- Integrità dei file propri: un file scritto da altri non deve passare per valido.

**Progetto**

1. Cartelle: dati (impostazioni, lock, token), cache, log, rapporti di crash e temporanea (`data_local_dir()/temp`, vedi `crates/lynkeus-app/src/platform/mod.rs`), tutte create con `create_private_dir`.
2. Windows: `CreateDirectoryW` con `SECURITY_ATTRIBUTES` e descrittore esplicito, protetto (nessuna ereditarietà dal padre), con ACE ereditabili per il SID dell'utente (da `TokenUser`, non per nome) e per SYSTEM. Mai la DACL di default (contiene il SID della logon session, effimero) né una DACL nulla. Se la cartella esiste già, non ci si fida: si controllano proprietario e DACL e, se concedono scrittura ad altri, si rifiuta (cache e log spenti per la sessione, avviso all'utente). I file creati ereditano la DACL della cartella; il file del token riceve un descrittore esplicito anche alla creazione.
3. Linux: cartelle 0700 già alla creazione, file con `O_CREAT|O_EXCL|O_NOFOLLOW` e 0600; la `umask` può solo restringere. Se la cartella esiste, controllo su descrittore aperto (`O_DIRECTORY|O_NOFOLLOW`, `fstat`): stesso uid, nessun permesso per gruppo e altri; altrimenti rifiuto, senza `chmod` silenziosi.
4. Ogni file nuovo è `create_new` (`CREATE_NEW` / `O_EXCL`): se esiste, anche come collegamento, la creazione fallisce. Si scrive sull'handle ottenuto, senza riaprire per nome. Nomi derivati da hash o generati dal programma, mai dal documento o dalla voce.
5. La cartella temporanea si svuota all'avvio (resti di un crash) e all'uscita. Un file ci resta al massimo 64 MiB, con estensione scelta dal programma da un elenco fisso, non dal nome della voce (4.7 C); i tipi eseguibili non si aprono (4.6).
6. Le cartelle stanno sotto il profilo dell'utente (`directories`), mai sotto la radice di un volume né in cartelle condivise, dove l'ereditarietà aprirebbe la scrittura ad altri. Manifest `longPathAware`.
7. Log e rapporti di crash riportano motivo e percorso, mai estratti di testo, token o nome della pipe.

**STRIDE**

| STRIDE | Attaccante | Minaccia | Mitigazione | Test previsto |
|---|---|---|---|---|
| S | U1 | Cartella pre-creata da altri con lo stesso nome, per far scrivere lì i dati | Cartelle nel profilo; se esiste, controllo di proprietario e permessi; rifiuto | Cartella preesistente con ACE per `Users` o 0777: rifiutata, cache e log spenti |
| T | U1, U2 | Voce di cache o impostazioni alterata | Intestazione con versione e CRC, chiave `blake3`; cartella inaccessibile a U1 (U2 con gli stessi diritti: rischio accettato) | Implementato (4.7 E): `corrupt_entries_are_ignored_and_removed` |
| T | U2, U3 | File preesistente, collegamento simbolico o hard link al posto del nome atteso | `create_new` non segue né sovrascrive; nomi da hash | `create_new_private_file` usa `create_new`; da scrivere il test con un collegamento preesistente (Linux: symlink; Windows: symlink o junction se i privilegi lo permettono) |
| T | U3 | Nome di voce d'archivio usato come nome di file | Nome generato dal programma | `malevoli/percorsi.zip` (4.7 C) |
| R | tutti | Nessuna traccia di chi ha scritto | Non rilevante: file locali di un solo utente; il log non è un registro di audit | |
| I | U1 | Lettura di cache, temporanei, impostazioni, log | Descrittore esplicito utente+SYSTEM (Windows), 0700 e 0600 (Linux) | Windows: dopo la creazione `icacls` o `Get-ACL` mostrano solo proprietario e SYSTEM, ereditarietà disattivata, nessun `Users`, `Everyone`, `Authenticated Users`. Linux: `stat` = 0700 e 0600, con `umask` 0 e 077. Non implementato su Windows |
| I | U2 | Lettura da un processo a integrità Low o con token ristretto | La DACL nomina solo l'utente; se un processo Low con lo stesso SID legge ancora, la risposta è un'etichetta di integrità Medium (`NO_READ_UP`), da adottare solo se la prova la mostra utile | Prova con un processo avviato a integrità Low; esito registrato qui |
| I | U1 | Contenuto dei documenti nei log o nei rapporti di crash | Solo motivo e percorso | File con testo riconoscibile, poi ricerca nel log e nei rapporti (4.7 D) |
| D | U2, U3 | Crescita illimitata di log, rapporti, temporanei | LRU per la cache (500 MB); rotazione dei log; numero massimo di rapporti (proposta 10); temporanei puliti all'avvio | Cache implementato (4.7 E); log, rapporti e temporanei previsti |
| D | U1 | Disco pieno o nomi occupati nella cartella | Cartella inaccessibile ad altri; errore di scrittura = cache spenta, la ricerca continua | Scrittura che fallisce: ricerca completata |
| E | U1, U3 | Un file della cartella viene eseguito o caricato | Il programma non esegue né carica nulla dalle cartelle dati; i tipi eseguibili non si aprono | Implementato: `executable_name_partitions`, `plan_partitions` |

**Rischi accettati (C)**

- Chi esegue codice con i diritti dell'utente legge cache, token e temporanei (sezione 5).
- Backup del profilo, copie shadow, indicizzatori e cartelle sincronizzate possono copiare la cache: l'avviso nel README lo dice.
- Dopo un crash una voce di cache può essere incompleta: la legge il CRC, che la scarta.
- Su file system senza ACL né permessi (FAT, exFAT) il descrittore non si applica: il programma lo rileva, avvisa e spegne la cache.

#### Incongruenze tra codice e documenti (4.8)

- Windows: `create_private_dir` e `create_new_private_file` non impostano alcun descrittore (il commento nel codice rimanda alla fase 7). La parte C non è verificata su Windows.
- Linux: `create_private_dir` non controlla proprietario e permessi di una cartella già esistente, e `recursive(true)` applica 0700 anche alle cartelle intermedie create (ammesso, ma da sapere).
- Pipe, socket, lock, token, registro e script Nautilus non sono nel codice: le parti A e B sono solo progetto.
- Il piano dice "istanza unica solo con `--add`"; qui si precisa che il canale esiste solo se l'opzione del clic destro è accesa (parte A, punto 9). Da confermare.

## 5. Rischi accettati

- **Eseguibile non firmato:** progetto personale senza certificato di firma del codice. Compensazione: `SHA256SUMS` e istruzioni nel README; SmartScreen mostrerà un avviso.
- **Voce del clic destro in "Mostra altre opzioni" su Windows 11:** scelta concordata.
- **Processo estrattore con gli stessi diritti dell'utente:** accettato (ADR 0001). Un bug di esecuzione di codice in un decoder permette di leggere i file dell'utente e scrivere dove l'utente scrive; i limiti di risorse non lo impediscono. Compensazione: decoder in Rust sicuro, ambiente e handle minimi, nessuna rete richiesta. Da rivalutare ora che la fase 5 ha aggiunto decoder di terzi con `unsafe` (`mail-parser`, `lzma-rust2` tramite `sevenz-rust2`; vedi 4.7, limiti noti): finora la scelta è di mantenere l'isolamento attuale. Isolamento più forte da valutare se si aggiunge un decoder in codice C o `unsafe` non coperto (AppContainer o token ristretto su Windows, Landlock su Linux).
- **Cache leggibile da chi ha già i diritti dell'utente:** non si difende dallo stesso utente o da malware con i suoi diritti.
- **File che cambia durante la lettura o aperto da un altro programma:** testo incoerente o avviso "file in uso"; nessun blocco del file.
- **Tempo di rilevamento:** un decoder in ciclo occupa un processo fino a 60 s prima del kill.
- **PDF scansionati e .rar non letti:** limite funzionale, non di sicurezza.
