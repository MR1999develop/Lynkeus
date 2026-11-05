# 0001. Estrazione del testo in processi isolati

Stato: accettata

## Contesto

Per cercare nei documenti il programma deve leggere PDF, Office, ODF, RTF, EML, EPUB e archivi. Sono formati complessi e non fidati: un file preparato ad arte nella cartella cercata può causare letture fuori limite, cicli infiniti, consumo di memoria (bombe di decompressione, entità XML) o, nel caso peggiore, esecuzione di codice. Il threat model (`docs/threat-model.md`, sezione 4.7) elenca le minacce.

Vincoli:

- l'app deve restare reattiva e non morire per un solo file ostile; la ricerca continua con gli altri file;
- il programma gira con i diritti dell'utente, senza amministratore e senza installazione di servizi;
- gli stessi crate devono funzionare su Windows 10/11 e Ubuntu;
- nei crate di dominio vale `forbid(unsafe_code)`: le chiamate di sistema dirette stanno in un solo modulo dell'app (`crates/lynkeus-app/src/platform/windows_ffi.rs`, `windows-sys`), dietro un'API safe; i crate di dominio restano senza `unsafe`;
- migliaia di file per ricerca: il costo per file deve essere piccolo.

## Opzioni

### 1. Nello stesso processo, con `catch_unwind` e limiti

I decoder girano in un thread del pool `rayon`; i panic si intercettano con `catch_unwind`; i limiti si impongono con i contatori nel codice.

Lati negativi:

- `catch_unwind` non intercetta abort, esaurimento di memoria, overflow dello stack né i cicli infiniti;
- un thread non si può terminare dall'esterno: il timeout si ottiene solo con controlli cooperativi che un decoder di terze parti non ha;
- nessun limite di memoria per decoder: una sola bomba manda in errore l'intera app;
- un bug di sicurezza in un decoder compromette il processo che ha anche l'interfaccia, la cache e le impostazioni.

### 2. Un processo figlio per file

Per ogni file si avvia un processo, che legge e termina.

Lati negativi:

- l'avvio di un processo su Windows costa decine di millisecondi (più con antivirus): con 100.000 file la scansione diventa molto più lenta;
- il costo è pagato anche per file piccoli e innocui;
- molte creazioni e distruzioni di Job Object e handle.

Vantaggio: isolamento massimo tra file. Con il pool lo si ottiene in parte riavviando il figlio dopo un errore.

### 3. Pool di processi figli (scelta)

Pochi processi longevi, avviati al primo documento, ciascuno con più file in sequenza.

Lati negativi:

- serve un protocollo, una macchina a stati e la gestione di morte e riavvio dei figli: più codice e più superficie da testare;
- stato condiviso tra file dello stesso figlio: un bug di corruzione della memoria con un file può influenzare i successivi (mitigato riavviando il figlio dopo ogni errore e, se necessario, dopo N file);
- il figlio ha gli stessi diritti dell'utente: non protegge dalla lettura dei file dell'utente se il decoder è compromesso;
- su Windows il limite di memoria è sulla memoria commessa del job (non sul working set) e il processo che lo supera viene terminato senza avvisi, quindi la causa va dedotta dall'uscita.

### 4. Sandbox WASM

I decoder compilati in WebAssembly e eseguiti in un runtime (per esempio `wasmtime`).

Lati negativi:

- i crate scelti (`pdf_oxide`, `calamine`, `mail-parser`, `sevenz-rust2`) dovrebbero compilare per `wasm32` e funzionare con l'ambiente ridotto: non verificato e probabilmente falso per alcune dipendenze;
- il runtime è una dipendenza grande, con la sua superficie e i suoi aggiornamenti, e fa crescere eseguibile e tempi di compilazione;
- prestazioni inferiori sul testo estratto da file grandi; limiti di memoria e di tempo vanno comunque imposti ("fuel");
- il runtime stesso gira nel processo principale: un bug del runtime lo compromette.

Resta una via possibile in futuro, per la parte che poi si vorrà isolare ancora di più.

## Decisione

Pool di processi figli (opzione 3). Lo stesso eseguibile, avviato con l'opzione nascosta `--extract-worker`, dal percorso assoluto di `current_exe()`.

- **Pool:** `min(core/2, 4)` processi, avviati al primo documento. Un figlio morto, scaduto o con un messaggio non valido si sostituisce; il file che lo ha causato riceve l'avviso adatto (danneggiato, tempo scaduto, limite superato).
- **Protocollo:** messaggi `postcard` in tipi concreti, con lunghezza in testa e dimensione massima, su stdin e stdout. Stdout è riservato al protocollo; il log va su stderr. Il padre non accetta messaggi fuori sequenza.
- **Limiti di risorse:**
  - Windows: un Job Object **per processo**, creato con `windows-sys` (il crate `win32job` non è usato: non offre il limite di memoria per processo né la porta di completamento). Limiti del job: `KILL_ON_JOB_CLOSE` (i figli non sopravvivono all'app), `PROCESS_MEMORY` (memoria impegnata per processo, 1 GiB) `DIE_ON_UNHANDLED_EXCEPTION` (nessuna finestra di errore: un'eccezione non gestita termina il processo) e `ACTIVE_PROCESS` con limite 1 (il figlio non avvia altri processi), più restrizioni dell'interfaccia (appunti, atomi globali, handle di finestre altrui, impostazioni di sistema e dello schermo, desktop, uscita da Windows). Il processo si avvia con `CreateProcessW` e `PROC_THREAD_ATTRIBUTE_JOB_LIST`, quindi nasce già nel job, senza finestra in cui sfugga al limite; `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` fa ereditare solo stdin e stdout (`std::process::Command` farebbe ereditare tutti gli handle ereditabili del padre). Dopo l'avvio `IsProcessInJob` deve confermare, altrimenti il processo è terminato e l'avvio fallisce. Una **porta di completamento** associata al job riceve `JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT`: il codice d'uscita da solo non distingue il limite superato da un aborto qualsiasi, e così l'avviso è "limite superato (memoria)";
  - Linux: il figlio imposta da sé `RLIMIT_AS` all'avvio, prima di leggere qualunque file, controlla l'esito (se non riesce esce con errore) e esce quando lo stdin si chiude. Il padre passa il proprio pid con `--parent-pid` (`--extract-worker --memory-limit-bytes <n> --parent-pid <pid>`); il figlio ne sorveglia l'esistenza dall'avvio (`watch_parent`, `/proc`) e, se il padre non c'è più, esce subito con `exit(3)` anche se è occupato in un decoder (`PR_SET_PDEATHSIG` non si usa: il motivo è in `crates/lynkeus-extract/src/platform.rs`). Limite noto: qui la memoria superata non si distingue da un aborto.
- **Tempo:** 60 s per file; oltre, il figlio è terminato subito e sostituito.
- **Annulla:** prima una richiesta di stop chiudendo lo stdin; dopo 500 ms si forza la chiusura (kill).
- **Ambiente:** `env_clear` salvo poche variabili di sistema (`SystemRoot` su Windows, nessuna su Linux); cartella corrente = cartella dell'eseguibile; stderr nullo; nessun handle ereditato oltre a stdin e stdout (su Windows con lista esplicita degli handle, su Linux affidato a `Command` e al close-on-exec dei descrittori; nessun test lo verifica ancora).
- **Valori fissati:** memoria 1 GiB per processo, frame del protocollo al massimo 1,25 MiB (`MAX_FRAME_BYTES`; era 16 MiB fino al protocollo v2, ora `PROTOCOL_VERSION` 3), stop e kill dopo 500 ms, timeout 60 s.
- **Cosa non fa il figlio:** non scrive sui dati dell'utente, non apre la rete, non avvia programmi, non esegue macro né segue collegamenti; produce solo testo con la sua posizione.
- **Cosa fa il padre:** non decodifica mai formati complessi; legge solo il protocollo e la cache.
- **Cosa non si decide qui:** non si usa AppContainer, token ristretto, Landlock né seccomp. Il figlio ha i diritti dell'utente; il rischio è accettato nel threat model (sezione 5) perché i decoder sono in Rust sicuro. Se si aggiunge un decoder in codice C o `unsafe`, questa decisione va riesaminata (nuovo ADR che sostituisce questo).

Perché non le altre: l'opzione 1 non dà né limiti di memoria né timeout affidabili; l'opzione 2 costa troppo per molti file piccoli; l'opzione 4 richiede dipendenze che non sappiamo compilare in WASM e aggiunge un runtime grande.

## Conseguenze

Più facile:

- un file ostile fa morire un figlio, non l'app; la ricerca prosegue e il file riceve un avviso preciso;
- limiti di memoria e di tempo imposti dal sistema operativo, non dalla buona volontà del decoder;
- nessun figlio orfano se l'app termina (Job Object su Windows; su Linux stdin chiuso e sorveglianza del padre con `--parent-pid`).

Più difficile:

- protocollo, pool, gestione degli errori e dei riavvii da scrivere e testare, compresi i casi di morte del figlio a metà file;
- debug più complesso (due processi, log su stderr);
- il limite di memoria per processo va scelto: troppo basso rifiuta documenti grandi legittimi, troppo alto non protegge. Fissato a 1 GiB (un PDF da 200 MiB ne occupa circa il doppio, perché la libreria copia i byte) e scritto nel threat model;
- un solo modulo con `unsafe` nell'app (`windows_ffi.rs`, con `// SAFETY:` per ogni blocco);
- la cache usa gli stessi tipi di messaggio del protocollo, quindi cambiare il protocollo invalida la cache (versione nell'intestazione).

## Come si verifica il rispetto

- **Test del pool** (`cargo test`, con un decoder finto selezionabile solo nei test): figlio che non risponde (timeout e riavvio), che muore a metà file (avviso "danneggiato", lotto completato), che alloca oltre il limite (terminato dal Job Object o da `RLIMIT_AS`; `exceeding_the_memory_limit_kills_and_is_recognized` per Windows), che scrive su stdout cose estranee al protocollo, che manda una lunghezza falsa o un messaggio fuori sequenza.
- **Test dell'annulla:** stop e kill entro 500 ms, nessun figlio rimasto.
- **Nessun orfano:** test che termina l'app con figli vivi e controlla che i processi scompaiano (Windows e Linux).
- **Ambiente e handle:** test che il figlio vede solo le variabili ammesse e gli handle stdin, stdout, stderr.
- **Fixture di `fixtures/malevoli/`:** `bomba.zip`, `annidato-profondo.zip`, `entita.docx`, `troncato.pdf`, `immagine-finta.pdf`, `percorsi.zip`, `riga-lunga.txt` danno l'avviso previsto senza bloccare né far crescere la memoria dell'app oltre il limite del test.
- **Separazione dei compiti:** controllo in CI che `lynkeus-app` e `lynkeus-engine` non dipendano direttamente dai crate dei decoder (verifica sul grafo di `cargo metadata`) e che nei moduli dei decoder (non in quello del pool, che avvia il figlio) non compaia `std::process::Command`. Pool e avvio dei processi stanno in `lynkeus-engine` (`pool.rs`, `launcher.rs`), i decoder in `lynkeus-extract`; il controllo non è ancora in CI.
- **Rilascio:** controllo manuale che l'eseguibile non contenga percorsi locali e che un file ostile non lasci file fuori dalla cartella di lavoro.
