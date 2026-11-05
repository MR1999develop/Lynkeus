# Analisi di conformità

- Data: 2026-11-05.
- Commit analizzato: `9788999`.
- Metodo: quattro revisori, uno per area, hanno letto tutto il codice dell'area rispetto alle regole sempre attive (numeri tra parentesi, "regola N"), alle skill pertinenti e alle schede consultate per sigla. Le aree:
  - A: ricerca ed estrattori (`crates/lynkeus-core`, `crates/lynkeus-extract`). Skill: `security-design`, `software-design-core`.
  - B: motore, pool, cache (`crates/lynkeus-engine`). Skill: `security-design`, `durable-data-design`, `software-design-core`.
  - C: interfaccia (`crates/lynkeus-app`, parte UI). Skill: `ui-design-core`, `rust-egui-desktop-design`.
  - D: integrazione con il sistema e rilascio (`crates/lynkeus-app` parte piattaforma, `xtask`, `.github`, `deny.toml`). Skill: `windows-system-design`, `security-design`.
- Ogni problema "verificato" è stato provato con test o con programmi di prova fuori dal repository (misure in release, file malevoli, copie dell'eseguibile). I "plausibili" sono letti nel codice ma non provati.
- Gli ID sono A-01, B-01, C-01, D-01 per i verificati e A-P1, B-P1... per i plausibili. Il numero segue l'ordine di gravità del rapporto. I percorsi sono relativi al repository.
- La gravità "media/bassa" e "media-bassa" dei rapporti è contata come media; "media-alta" come media.

## Riepilogo

| Area | Alta | Media | Bassa | Verificati | Plausibili |
|---|---|---|---|---|---|
| A ricerca ed estrattori | 0 | 5 | 5 | 10 | 5 |
| B motore, pool, cache | 0 | 4 | 9 | 13 | 5 |
| C interfaccia | 3 | 8 | 7 | 18 | 6 |
| D integrazione e rilascio | 0 | 5 | 5 | 10 | 2 |
| Totale | 3 | 22 | 26 | 51 | 18 |

### Stato dopo le correzioni

Stato al commit `2552454`, salvo B-02 e A-P4, aggiornati dopo (vedi le loro righe). Le correzioni sono nei commit tra `75a334e` e `2552454`. Ogni riga delle tabelle ha una colonna "Stato" con il commit che la corregge. Le due revisioni delle correzioni (interfaccia, librerie) sono nella sezione "Seconda revisione".

| Area | Corretti | Corretti in parte | Rimandati | Aperti | Totale (verificati + plausibili) |
|---|---|---|---|---|---|
| A ricerca ed estrattori | 15 | 0 | 0 | 0 | 15 |
| B motore, pool, cache | 17 | 0 | 1 (B-12) | 0 | 18 |
| C interfaccia | 24 | 0 | 0 | 0 | 24 |
| D integrazione e rilascio | 12 | 0 | 0 | 0 | 12 |
| Totale | 68 | 0 | 1 | 0 | 69 |

Seconda revisione: 17 punti (sezione dedicata), 15 corretti, 1 da provare su Linux (R-13), 1 rischio aperto accettato (R-17).

Punti da ricordare:

- B-12 è rimandato: lo sfratto della cache è cambiato, la decodifica in streaming no.
- Nessun test Linux è stato eseguito in locale. B-04, A-07 e il test sugli orfani occupati li eseguirà la CI su GitHub.
- Il fuzzing non è stato eseguito: ci sono gli oracoli, non ci sono esiti.
- L'avvio su Windows senza Visual C++ Redistributable non è provato (D-04), ma il runtime C ora è statico.

Esito della suite al momento dell'analisi (commit `9788999`): `cargo test -p lynkeus-engine` verde a macchina scarica (con CPU satura fallisce `cancel_stops_a_busy_worker_within_one_second`, vedi B-01); `cargo test -p lynkeus-app`: 17 accettazione e 6 interfaccia verdi, 146 test della libreria verdi.

## Priorità di correzione

1. Sicurezza e crash
   - A-01: un xlsx sparso termina il processo estrattore (abort di memoria) e fa perdere le voci seguenti dello zip.
   - D-03: le DLL importate in modo statico si caricano dalla cartella dell'eseguibile (verificato: uscita 0xC000012F con una `dwmapi.dll` finta).
   - D-01: estensioni che Windows esegue per associazione (.jnlp, .theme, .pyc, .msu...) non sono nell'elenco degli eseguibili.
   - D-02: la voce estratta da un archivio scaricato non eredita il Mark-of-the-Web.
   - B-03: il padre non rivalida la dimensione delle unità ricevute dal figlio.
   - B-04: su Linux un estrattore occupato resta orfano se il padre muore.
   - D-09: il job object non limita i processi attivi né l'interfaccia.
   - A-08, D-06, B-09, D-08, D-07: default non sicuri o controlli che falliscono aperti (memoria, voce di registro, cartelle private, eccezioni senza scadenza, verifica delle mitigazioni).
2. Accessibilità e tastiera
   - C-01: Esc annulla la ricerca invece di chiudere il menu aperto.
   - C-02, C-03: la riga selezionata non si distingue (contrasto 1,07-1,13:1) e non ha ruolo né stato per i lettori di schermo; le frecce non spostano il focus.
   - C-09: focus non visibile sul pulsante primario e sulle intestazioni.
   - C-04: un errore nei Filtri chiusi o nel pannello nascosto non si vede.
   - C-13, C-15 e i plausibili C-P1..C-P4.
3. Robustezza e limiti
   - B-01: Annulla oltre 1 s sotto carico.
   - B-02: con AND ogni thread trattiene fino a 200.000 risultati (circa 1,3 GB stimati).
   - D-04: CRT dinamico, l'avvio su Windows senza redistributable non è provato.
   - D-05: posizione e dimensione della finestra non si salvano.
   - C-07, C-08, C-11: lavoro pesante sul thread dell'interfaccia (copia 0,4 s, `step_target` 25-50 ms per frame, salvataggio con `sync_all`).
   - C-06, C-10, C-05, C-12, C-14, C-16..C-18, D-10: feedback e messaggi imprecisi.
   - B-07, B-10..B-12, A-05, A-06 e i plausibili di B.
4. Correttezza della ricerca e dei formati
   - A-02: Hangul con accenti ignorati non si trova.
   - A-03: termini che dopo la normalizzazione diventano jolly.
   - A-04: RTF, `\par` dentro destinazioni saltate.
   - A-05: .gz con tar dentro.
   - A-06: EPUB senza controllo di profondità e voci.
   - A-07: nomi non UTF-8 su Linux.
5. Qualità del codice e test
   - B-05: cicli tra moduli.
   - B-06, A-09: duplicazioni.
   - B-13: test dell'Annulla instabile.
   - A-P5: fuzzing senza oracoli.
   - A-10, B-08, D-07, D-08.
   - Il tracciamento problema → test è in `docs/piano-di-test.md`.

## Area A: ricerca ed estrattori

Percorsi: `core` = `crates/lynkeus-core/src`, `extract` = `crates/lynkeus-extract/src`.

| ID | File:riga | Gravità | Problema | Rif. | Correzione proposta | Stato |
|---|---|---|---|---|---|---|
| A-01 | crates/lynkeus-extract/src/extract/sheet.rs:89 | media | `worksheet_range` di calamine riserva righe×colonne celle prima di ogni limite. Un xlsx di 1,5 KB con A1 e XFD1048576 chiede 549755813888 byte: abort (non panic, `catch_unwind` non lo ferma), muore il processo estrattore, avviso sull'intero file, le voci seguenti dello stesso zip non si leggono. Vale anche per fogli veri con poche celle molto distanti | regole 13, 16, 33; OWCS, CWE | xlsx con `worksheet_cells_reader` a flusso; ods e xls con tetto righe×colonne prima di calamine, esito `LimitExceeded` | corretto (8ae3935, 69c92c8): xlsx a flusso; ods e xls con tetto in byte (`sheet_memory.rs`) |
| A-02 | crates/lynkeus-core/src/pattern.rs:114 (unicode.rs:76) | media | Con "Ignora gli accenti" il termine va in NFD: l'Hangul diventa jamo (non Mn) e non si ricompone. "한국" su "한국 사람" non trova nulla (con accenti distinti trova) | regola 27; specifica §4 | Dopo aver tolto i Mn, tornare in NFC e aggiungere l'alternativa NFD come nella modalità distinta | corretto (8ae3935) |
| A-03 | crates/lynkeus-core/src/query.rs:125 | media/bassa | "Vuoto" e "solo jolly" si controllano prima della normalizzazione: "*\u{301}" con accenti ignorati diventa "*" e trova ogni parola fino al limite di 200.000; "\u{301}" da solo non trova nulla senza errore | regola 13; specifica §3 | In `Query::new` rifiutare i termini che dopo la tokenizzazione con le opzioni non hanno né letterale né `?` | corretto (8ae3935) |
| A-04 | crates/lynkeus-extract/src/extract/rtf.rs:327 (e :257) | media/bassa | `\par`, `\cell`, `\row`, `\`+a capo chiudono il paragrafo anche in una destinazione saltata: `La cena{\footnote\pard nota\par} di aragosta\par` dà due paragrafi, la frase non si trova, i numeri di paragrafo slittano | specifica §7 | Ignorare i segni di fine paragrafo se `group.skip` | corretto (8ae3935) |
| A-05 | crates/lynkeus-extract/src/extract/tarball.rs:51-62 | media/bassa | Un .gz che contiene un tar (riconosciuto dal contenuto) lo legge con `LimitedReader::new`: il massimo per voce vale per tutto il tar e i byte si contano due volte. Lo stesso tar dà risultati diversi come "x.gz" e "x.tar.gz" | regola 3 | Testa letta con `LimitedReader::for_container`; limite per voce solo nel ramo "documento singolo" | corretto (8ae3935) |
| A-06 | crates/lynkeus-extract/src/extract/epub.rs:92-93 | bassa | EPUB è un archivio ma non chiama `check_depth` né `add_entries`: con `max_depth` 1 uno zip interno è rifiutato, i capitoli di un EPUB nella stessa posizione si leggono | TM 4.7 C | `check_depth` e `add_entries(capitoli)` prima del ciclo | corretto (8ae3935) |
| A-07 | crates/lynkeus-extract/src/format.rs:366 | bassa | `to_str()` scarta i nomi non UTF-8: su Linux "rel\xff.docx" diventa testo e la firma PK lo fa saltare come binario, senza avviso | regola 13 | Estensione da `OsStr::as_encoded_bytes()` | corretto (8ae3935); test Linux non eseguito in locale |
| A-08 | crates/lynkeus-extract/src/worker.rs:85-101, :123 | bassa | Senza argomenti `WorkerConfig` non ha limite di memoria; su Linux il processo parte senza `RLIMIT_AS` | regola 33 (default sicuri, fallire chiusi) | Limite obbligatorio in `from_args` | corretto (8ae3935): `worker::run` non parte senza limite |
| A-09 | crates/lynkeus-extract/src/extract/archive.rs:52 e crates/lynkeus-engine/src/doc.rs:75 | bassa | `neutralize` duplicata in due crate e non copre U+2028/2029, U+200B-200D, U+FEFF | regola 3 | Una sola funzione nel core con l'elenco completo | corretto (8ae3935): `neutralize_char` nel core |
| A-10 | crates/lynkeus-extract/src/extract/mod.rs:630-632, crates/lynkeus-extract/src/extract/sheet.rs:53-55 | bassa | Commenti fuori posto o falsi ("file già in memoria" è falso per `Input::File`) | regole 1, 18 | Correggere i commenti | corretto (8ae3935) |

### Plausibili (area A)

| ID | File:riga | Osservazione | Stato |
|---|---|---|---|
| A-P1 | crates/lynkeus-extract/src/extract/archive.rs:178 | Il rapporto di compressione usa la dimensione compressa dichiarata: uno zip imbottito aggira il 200:1 (restano i limiti di 512 MiB per voce e 4 GB totali) | corretto (8ae3935) |
| A-P2 | crates/lynkeus-extract/src/extract/ooxml.rs:181 | Lo stesso `sldId` ripetuto fa decomprimere N volte la stessa diapositiva fino al budget o al timeout | corretto (8ae3935) |
| A-P3 | crates/lynkeus-extract/src/extract/ppt.rs, `read_persist_directory` | 4096 modifiche che puntano alla stessa directory moltiplicano il lavoro | corretto (8ae3935) |
| A-P4 | sevenz.rs:46, sheet.rs:58 (xls), nomi lunghi PAX/GNU in tarball.rs (tutti sotto crates/lynkeus-extract/src/extract/) | Le librerie allocano prima dei nostri limiti; ferma solo il limite di memoria del processo | corretto (8ae3935, poi i nomi del tar): xls coperto da A-01, 7z verificato. Il crate `tar` (0.4.46) legge per intero in memoria i nomi lunghi GNU (`L`, `K`) e le estensioni PAX (`x`) con `read_all`: ora il tar si legge in modo grezzo e i nomi lunghi e le estensioni PAX si leggono al più 64 KiB (`MAX_TAR_EXTENSION_BYTES`, `tarball.rs`); oltre, avviso di limite (memoria) per la voce e le seguenti si leggono. Test `oversized_tar_names_are_a_limit_and_later_entries_are_read` con `malevoli/nomi-lunghi.tar.gz` |
| A-P5 | fuzz/ | Gli obiettivi non hanno oracoli oltre il crash (FZB); nessun esito di sessione nel threat model | corretto (8ae3935, 69c92c8): oracoli in tutti i target; il fuzzing non è ancora stato eseguito, nessun esito |

### Punti di forza (area A)

- Core puro con `forbid(unsafe)`, regex lineari, tipi validi per costruzione; proptest su confini UTF-8, blocchi uguali all'intero, NFC/NFD.
- Protocollo: lunghezza prima di allocare, versione, tipi concreti; test di troncamento e proptest.
- `LimitedReader` (take+1) con budget sull'intero file radice; `catch_unwind` per voce con ripristino delle etichette.
- Parser propri (.doc, .ppt, RTF) con `get` e `checked_*` e overflow-checks in release.
- Errori → `WarningKind`/`SkipReason` in un solo punto (`Ctx::report`).

## Area B: motore, pool, cache

Tutti i percorsi sono sotto `crates/lynkeus-engine/` (`src/` e `tests/`).

| ID | File:riga | Gravità | Problema | Rif. | Correzione proposta | Stato |
|---|---|---|---|---|---|---|
| B-01 | crates/lynkeus-engine/src/pool.rs:197-211, 499-502 | media | Annulla oltre 1 s sotto carico: il lettore attende `STOP_GRACE` 500 ms, kill, altri 500 ms per raccogliere il processo, poi `Finished`. Con CPU satura (12 core): 1,08 / 1,21 / 1,19 s; `cancel_stops_a_busy_worker_within_one_second` fallisce 3/3 sotto carico, 2/2 con due `cargo test` in parallelo, passa 7/7 a macchina scarica | regole 40, 16; RIT, PNB | Chiudere stdin, restituire subito `Stopped`, passare il processo a un thread di raccolta atteso in shutdown | corretto (7aa9e68): Annulla risponde subito, il processo si raccoglie in un thread del pool |
| B-02 | crates/lynkeus-engine/src/textfile.rs:86-94, docsearch.rs:136-152 | media | Con AND ogni thread trattiene fino a `room` (200.000) risultati, non il totale: fino a 16 thread × ~400 B, circa 1,3 GB (stima) | regola 16; RIT | Contatore atomico condiviso dei risultati trattenuti, prenotato a ogni push | corretto (7aa9e68, poi rivisto dopo la misura P-03): il contatore condiviso non bastava, P-03 misurava 888 MB. La memoria andava nei risultati di `find_words` di un passo da 1 MiB (circa 73 MB per thread). Passi di 64 KiB anche per le unità dei documenti (`step.rs`) e tetto comune di 32 MiB stimati per i risultati trattenuti (`MAX_HELD_BYTES`, `batch.rs`), indipendente dal limite dei risultati: P-03 145 MB, sotto il budget di 256 MiB |
| B-03 | crates/lynkeus-engine/src/pool.rs:579-586, 66-68 | media | Il padre non rivalida la dimensione delle unità (`MAX_UNIT_TEXT_BYTES` 1 MiB vale solo nel figlio). Figlio compromesso: unità da ~16 MiB, coda 16×16 MiB per processo, milioni di `WordMatch` | regola 33 (rivalidare a ogni confine); TM 4.7 D | In `serve`, unità > 1 MiB o `inner` oltre la profondità massima = `ProtocolViolation` | corretto (7aa9e68) |
| B-04 | crates/lynkeus-engine/src/launcher.rs:123-141 (+ ciclo del worker in lynkeus-extract) | media | Su Linux un figlio occupato non muore con il padre (legge stdin solo tra due richieste, niente `PR_SET_PDEATHSIG`). Il test sugli orfani prova solo un figlio inattivo | regola 16; W10SP1; ADR 0001 | rustix `set_parent_process_death_signal(Kill)` all'avvio del figlio e controllo `getppid` | corretto (7aa9e68, 8ae3935, 69c92c8): `--parent-pid` e `watch_parent`; test Linux mai eseguito (lo farà la CI) |
| B-05 | crates/lynkeus-engine/src/worker.rs:37,42 ↔ handle.rs:17, textfile.rs:28, docsearch.rs:23 | bassa | Cicli tra moduli (`Notify`, `Stop`, `Search` in worker) | regola 8; LSC | `Stop` e `Notify` in un modulo foglia; `DocSearch` dipende da un trait | corretto (7aa9e68) |
| B-06 | crates/lynkeus-engine/src/textfile.rs:87-104, docsearch.rs:136-160 | bassa | Regola dei lotti e del limite duplicata | regola 3 | Tipo `HitBatcher` unico | corretto (7aa9e68) |
| B-07 | crates/lynkeus-engine/src/cache.rs:333-364, 377-378 | bassa | `clear()` e `scan()` fanno I/O tenendo il mutex | regola 23 | Elenco sotto lock, cancellazione fuori | corretto (7aa9e68) |
| B-08 | crates/lynkeus-engine/src/documents.rs:122 (cache.rs:288,295,411) | bassa | `let _saved = cache.put(..)` ingoia l'errore | regola 14 | Contatore degli errori con il primo errore, registrato una volta dall'app | corretto (7aa9e68, edd4e24): errori contati, registrati una volta a fine ricerca |
| B-09 | crates/lynkeus-engine/src/cache.rs:230-237, platform.rs:82-90,162-168 | bassa | Su Windows `TextCache::new` e `create_private_*` non sono privati se usati senza il `PrivateFs` dell'app | regole 17, 33 | `PrivateFs` obbligatorio nel costruttore; versione portabile solo `cfg(not(windows))` | corretto (7aa9e68, edd4e24): solo `TextCache::at` con `PrivateFs` |
| B-10 | crates/lynkeus-engine/src/documents.rs:322-324 | bassa | `UnitWindow` lascia finire l'estrazione con cache spenta o voce > 64 MiB: l'anteprima occupa un processo fino a 60 s | regola 16 | `Err(Stopped)` a finestra completa quando non si salva | corretto (7aa9e68) |
| B-11 | crates/lynkeus-engine/src/request.rs:111-116,178-181, platform.rs:106-113 | bassa | `to_lowercase` per ogni voce nel ciclo caldo | regola 31; RPB | Filtro compilato una volta per ricerca | corretto (7aa9e68, 3ea1c0f) |
| B-12 | crates/lynkeus-engine/src/cache.rs:190-210, 275-282 | bassa | A cache piena ogni `put` ordina l'indice sotto lock; `get` tiene file+frame+eventi (~3×64 MiB per lettore) | regola 31 | Sfratto a lotti fino al 90%, decodifica in streaming | rimandato: lo sfratto è cambiato (7aa9e68: indice ordinato per uso, nessun ordinamento a ogni `put`, sfratto fuori dal lucchetto), ma la decodifica in streaming no. `get` tiene insieme file compresso e frame decompressi (fino a 64 MiB). Lo streaming chiede un formato a pezzi e `CACHE_FORMAT_VERSION` 2 |
| B-13 | crates/lynkeus-engine/tests/documenti.rs:299-310 | bassa | Test di Annulla con `sleep(200 ms)` e polling, budget da orologio | regola 27; GOOS, PNB | Segnale "richiesta ricevuta" dal worker di test, budget in una costante | corretto (7aa9e68) |

### Plausibili (area B)

| ID | File:riga | Osservazione | Stato |
|---|---|---|---|
| B-P1 | crates/lynkeus-engine/src/filter.rs:54-61 | Cartelle con `RECALL_ON_DATA_ACCESS`/`RECALL_ON_OPEN` (OneDrive) visitate anche con "salta solo online": l'elenco può scatenare richiami (WFIO) | corretto (7aa9e68) |
| B-P2 | crates/lynkeus-engine/src/pool.rs:184-194 | Scrittura della richiesta su stdin senza scadenza | corretto (7aa9e68) |
| B-P3 | crates/lynkeus-engine/src/pool.rs:571 + worker.rs:426-441 | La scadenza di 60 s conta anche l'attesa della coda verso l'interfaccia: Timeout spuri | corretto (7aa9e68) |
| B-P4 | crates/lynkeus-engine/src/cache.rs:213-214 | Due istanze ignorano le voci l'una dell'altra (disco fino a N × max); `set_max_size` da thread staccati può applicarsi in ordine inverso | corretto (7aa9e68, 69c92c8): rilettura della cartella dopo almeno 64 scritture e almeno una ogni 16 voci dell'indice (`rescan_due`), e a ogni cambio di limite |
| B-P5 | crates/lynkeus-engine/src/pool.rs:162-164, handle.rs:119-121 | Thread staccati senza proprietario che li attenda (regola 16) | corretto (7aa9e68) |

### Punti di forza (area B)

- Cache: lunghezze prima di allocare, CRC, sequenza, temporaneo `create_new` e rinomina, voce corrotta cancellata; test di corruzione e proptest.
- Il pool non si fida del figlio: id, `SequenceCheck`, `Rejected`, timeout con kill, percorso assoluto, `env_clear`, stderr nullo; test con sei comportamenti scorretti.
- Code limitate (eventi 256, documenti 64, messaggi 16); annullamento a ogni voce e passo da 1 MiB; atomici Relaxed; `try_lock` sull'avanzamento; thread con nome e scope.
- Filtri puri con test a partizioni; radici duplicate e annidate, link e junction coperti; driver comune con `Notify` e scadenza.

## Area C: interfaccia

Tutti i percorsi sono sotto `crates/lynkeus-app/` (`src/`, `locales/`).

| ID | File:riga | Gravità | Problema | Rif. | Correzione proposta | Stato |
|---|---|---|---|---|---|---|
| C-01 | crates/lynkeus-app/src/ui/mod.rs:184-190 | alta | Durante una ricerca Esc è consumato da `Command::Cancel` prima che i menu siano disegnati: aprendo "Ricerca" o il menu contestuale e premendo Esc la ricerca si annulla e il menu resta aperto | regole 36, 41; GHIG, MI | Niente Esc/Invio come scorciatoie se un menu o popup è aperto | corretto (db50394) |
| C-02 | crates/lynkeus-app/src/ui/table.rs:118, theme.rs:118,139 | alta | La riga selezionata si riconosce solo dal fondo: contrasto selezione/riga alterna 1,13:1 (chiaro) e 1,07:1 (scuro); contro la riga sotto il puntatore 1,01:1; la riga del cursore non ha segni | regola 45; HCP | Segno sulla riga (barra o "›") e test di contrasto su `selection_bg`/`fill`/`fill_hover` | corretto (db50394) |
| C-03 | crates/lynkeus-app/src/ui/table.rs:58-62,110-157 | alta | Righe senza `WidgetInfo`: nessun ruolo, nessuno stato "selezionato", le frecce non spostano il focus; Narrator e Orca non dicono quale riga è selezionata | skill `ui-design-core` (nome, ruolo, valore); WDG, GHIG | Nodo focalizzabile per riga con `WidgetInfo::selected` e `request_focus` sulla riga del cursore | corretto (db50394) |
| C-04 | crates/lynkeus-app/src/ui/mod.rs:460-468, ui/search_panel.rs:293-297,328-332 | media | Un errore nei Filtri chiusi o con il pannello nascosto (F9) non si vede: Cerca non fa nulla, stato "Pronto." (es. "Solo queste" con elenco vuoto salvato; al riavvio F5 non fa niente) | regola 37; KHIG, FDP | Con `StartError::Form` mostrare il pannello, aprire i Filtri e portare il focus sul primo campo sbagliato | corretto (db50394) |
| C-05 | crates/lynkeus-app/src/i18n.rs:18-23 | media | Numeri non formattati per lingua ("Risultati (200000)" in entrambe) | regola 42; FLNT | Formatter personalizzato nello `static_loader` e test | corretto (db50394) |
| C-06 | crates/lynkeus-app/src/ui/status.rs:24-34 | media | La barra fa done/seen ma seen cresce nello stesso thread che legge: resta al 99-100% e oltre 10 s non c'è stima | regola 40; DWMM, GHIG, MI | Barra indeterminata con i conteggi finché il totale non è noto | corretto (db50394, edd4e24): frazione e tempo stimato dopo 10 s |
| C-07 | crates/lynkeus-app/src/app.rs:940-947,984-985 | media | Tabella per copia/esportazione costruita sul thread dell'interfaccia: 200.000 righe → 259 ms, con `to_tsv` 341 ms e 39 MB; Ctrl+A Ctrl+C blocca ~0,4 s | regole 23, 40 | Costruzione nel thread di esportazione, copia come lavoro con avanzamento | corretto (db50394) |
| C-08 | crates/lynkeus-app/src/results.rs:270-297 (da ui/mod.rs:184, ui/preview.rs:119) | media | `step_target` scorre tutte le righe, chiamato 8 volte per frame: 12,5 ms per chiamata su 200.000 righe, 25-50 ms per frame al passaggio del mouse | regole 31, 40; ADM | Inizio e fine del gruppo di file calcolati in `finish`/`set_sort`, controllo O(1) | corretto (db50394) |
| C-09 | crates/lynkeus-app/src/ui/search_panel.rs:379-385, ui/table.rs:91-95, theme.rs:237 | media | Focus non visibile: sul primario il bordo di focus ha lo stesso colore del fondo (1,00:1); intestazioni con `frame(false)` senza bordo; campi con bordo di 1 px | regola 45 | Bordo di focus `text_strong` da 2 px; `frame_when_inactive(false)` | corretto (db50394) |
| C-10 | crates/lynkeus-app/src/app.rs:1393-1400 | media (media-bassa) | Dopo un annullamento senza righe, o un arresto del thread per errore, compare "Nessuna parola trovata… prova un termine più corto": feedback ingannevole | MI, DOET | Stati vuoti propri per "annullata" e "interrotta" | corretto (db50394) |
| C-11 | crates/lynkeus-app/src/ui/mod.rs:141-147, main.rs:211-227 | media (media-bassa) | Salvataggio periodico con `sync_all` sul thread dell'interfaccia; con `settings.toml` più recente o illeggibile le modifiche non si salvano mai e l'utente non lo sa (solo log) | regole 23, 35; KHIG | Salvataggio in un thread, messaggio per `NewerVersion`/`Unreadable`/`Invalid` | corretto (db50394) |
| C-12 | crates/lynkeus-app/locales/it/main.ftl:215,226 | bassa | "{ $skipped } saltati, { $binary } binari" senza plurali ("1 saltati") | FLNT | Selettori one/other | corretto (db50394) |
| C-13 | crates/lynkeus-app/src/ui/preview.rs:120 | bassa | Nomi accessibili "« File"/"File »" ambigui | skill `ui-design-core` | `accessible_name` dal label del comando | corretto (db50394) |
| C-14 | crates/lynkeus-app/src/ui/mod.rs:373 | bassa | "Ripristina i filtri predefiniti" senza Annulla | regola 36 | Messaggio con "Annulla" | corretto (db50394) |
| C-15 | crates/lynkeus-app/src/ui/search_panel.rs:26-28 | bassa | Nell'ordine del Tab, Cerca viene prima di luoghi e termini (WCAG 2.4.3) | regola 45 | Ordine di focus coerente con la lettura | corretto (db50394) |
| C-16 | crates/lynkeus-app/src/form.rs:561-566 | bassa | Una riga di termine vuota blocca la ricerca | regola 37 | Ignorare le righe vuote se ce n'è una valida | corretto (db50394) |
| C-17 | crates/lynkeus-app/src/results.rs:149-155 | bassa | A fine ricerca il riordino non riporta in vista la riga del cursore; con ordinamento scelto durante la ricerca le righe nuove vanno in coda anche con "↑" | regola 35 | Riportare in vista e inserire in ordine | corretto (db50394) |
| C-18 | crates/lynkeus-app/src/i18n.rs:94-96 | bassa | ID mancante o errore di formattazione: restituisce l'ID senza log | FLNT | Registrare l'errore una volta | corretto (db50394) |

### Plausibili (area C)

| ID | Osservazione | Stato |
|---|---|---|
| C-P1 | Celle focalizzabili (`Sense::click`): il Tab passa da ogni cella; Invio su una cella la seleziona invece di aprirla | corretto (db50394) |
| C-P2 | Invio con un menu aperto apre il risultato corrente | corretto (db50394) |
| C-P3 | Nessuna regione "live" AccessKit: messaggi ed esito della ricerca non annunciati | corretto (db50394) |
| C-P4 | Mancano F6 tra i pannelli e il contrasto elevato di Windows (WDG) | corretto (db50394, edd4e24): F6 e contrasto elevato letto dal sistema |
| C-P5 | Dialoghi `rfd` sincroni: finestra non ridisegnata, thread di ricerca fermo sulla coda piena | corretto (db50394) |
| C-P6 | crates/lynkeus-app/src/ui/preview.rs:162: paragrafo di 4 KiB su una riga con scorrimento orizzontale | corretto (db50394) |

### Punti di forza (area C)

- `Application`, `ResultsView`, `format`, `export` senza egui; gesti → `Action` applicate dopo il disegno; `logic()` con tetto e ripaint su richiesta.
- Una tabella dei comandi per menu e scorciatoie, motivi delle voci disabilitate, test sull'ordine delle scorciatoie.
- `theme.rs` a token con test dei contrasti; accento blu; corrispondenze con fondo, SemiBold e sottolineatura; errori con "Errore:".
- `settings.toml` versionato, campi sconosciuti conservati, scrittura durevole; esportazione atomica con formule neutralizzate in CSV/TXT/TSV e test di proprietà.
- Fluent: ID uguali, messaggi risolti, commenti; kittest per ruolo e nome.
- Nota: `empty_state_partitions` (crates/lynkeus-app/src/app.rs) esiste ma fissa il comportamento sbagliato di C-10 (interruzione = "nessun risultato").

## Area D: integrazione con il sistema e rilascio

Percorsi sotto `crates/lynkeus-app/src/` salvo indicazione.

| ID | File:riga | Gravità | Problema | Rif. | Correzione proposta | Stato |
|---|---|---|---|---|---|---|
| D-01 | crates/lynkeus-app/src/launch.rs:34-93 | media | `EXECUTABLE_EXTENSIONS` non copre tipi che Windows esegue per associazione: .jnlp, .theme, .themepack, .deskthemepack, .msu, .ppkg, .vsix, .pyc, .pyz, .pyzw, .mht (verificato nel registro: .theme → rundll32 themecpl.dll, .jnlp → jp2launcher, .pyc/.pyz → py.exe, .msu → wusa, .ppkg → provtool). Un .theme o .jnlp è testo: la ricerca lo trova e il doppio clic applica un tema con risorse remote (fuga NTLM) o avvia Java Web Start; una voce x.pyc in uno zip diventa voce-….pyc ed è eseguita | regola 33 (default sicuri, allowlist); TM 4.7 F; CWE | Aggiungere le estensioni e un secondo filtro (`AssocIsDangerous` o allowlist dei formati letti più testo comune) con ripiego su "Mostra nella cartella" | corretto (c1208e8) |
| D-02 | crates/lynkeus-app/src/extraction.rs:227-266 (launch.rs:279) | media | La voce estratta non riceve il Mark-of-the-Web (`Zone.Identifier`) dell'archivio scaricato: un fattura.docm da uno zip scaricato, aperto con "Apri", offre "Abilita contenuto" invece del blocco delle macro | regola 33; CWE | Copiare lo stream `Zone.Identifier` dell'archivio esterno sul file estratto (o `IAttachmentExecute::Save`) | corretto (c1208e8) |
| D-03 | crates/lynkeus-app/src/main.rs:47-49, .cargo/config.toml:6 | media | `SetDefaultDllDirectories` non copre le importazioni statiche: l'exe importa 9 DLL non KnownDLLs (dwmapi, uxtheme, dwrite, dxgi, opengl32, uiautomationcore, userenv, bcryptprimitives, VCRUNTIME140) con `DependentLoadFlags` = 0. Verificato: copia dell'exe con una `dwmapi.dll` finta accanto, lanciata con `--extract-worker`, esce con 0xC000012F prima di `main`; vale per ogni processo estrattore | regole 16, 54; MSDL, W10SP2 | `-C link-arg=/DEPENDENTLOADFLAG:0x800` in `.cargo/config.toml`, verifica in `dist`, commento di `main.rs` corretto | corretto (c1208e8) |
| D-04 | .cargo/config.toml:5-6, xtask/src/dist.rs:284 | media | CRT dinamico: l'exe importa `VCRUNTIME140.dll`, lo zip non la contiene e il README non cita il Visual C++ Redistributable. Su Windows senza redistributable l'avvio fallirebbe (plausibile, non provato su VM pulita) | regole 26, 32; HPT | `-C target-feature=+crt-static` | corretto (c1208e8); avvio su Windows pulito non provato |
| D-05 | crates/lynkeus-app/src/main.rs:174-179 | media | Posizione e dimensione della finestra non si salvano: `persistence_path` riceve la cartella, ma eframe 0.37 lo tratta come file RON; `File::create(cartella)` fallisce e l'avviso va al crate `log`, che `tracing` non raccoglie | regole 39, 14; AF, EGUI | `persistence_path` = `config\window.ron`; `persist_window` false se la cartella manca | corretto (c1208e8) |
| D-06 | crates/lynkeus-app/src/context_menu.rs:331-334 | bassa | Con `ExeDirWritable` `refresh` esce prima di guardare lo stato: una voce già registrata resta attiva | regola 33 (fallire chiusi); TM 4.8 B | Togliere le voci proprie e spegnere l'opzione | corretto (c1208e8, edd4e24) |
| D-07 | xtask/src/dist.rs:218-224, .github/workflows/ | bassa | Senza `dumpbin` `verify_mitigations` passa (fallisce aperto); nessun workflow di rilascio da tag (niente SBOM, provenienza, cargo auditable); manca il job pianificato di fuzzing previsto dal piano | regole 32, 33; SSCS, RSEC, MSDL | Leggere CFG, CETCOMPAT e `DependentLoadFlags` dal PE in Rust; `release.yml` su tag e `fuzz.yml` | corretto (c1208e8): `release.yml`, `fuzz.yml`, lettura del PE in `xtask/src/pe.rs`; workflow non eseguiti in locale |
| D-08 | deny.toml:10-21 | bassa | Eccezioni motivate ma senza scadenza | regola 33; SSCS, RSEC | Data di revisione nel `reason` e controllo in xtask che fallisce dopo quella data | corretto (c1208e8) |
| D-09 | crates/lynkeus-app/src/platform/windows_ffi.rs:147-149 | bassa | Il job non ha `JOB_OBJECT_LIMIT_ACTIVE_PROCESS` né restrizioni UI: un decoder compromesso può avviare cmd.exe o leggere gli appunti | regola 33 (privilegi minimi); W10SP1 | `ActiveProcessLimit` = 1 e `JOBOBJECT_BASIC_UI_RESTRICTIONS` | corretto (c1208e8) |
| D-10 | crates/lynkeus-app/src/cli.rs:42-44, main.rs:55-60 | bassa | Un percorso senza `--add` (cartella trascinata sull'icona dell'exe, "Invia a") dà uscita 2 con messaggio su stderr, invisibile in un'app GUI | regole 35, 37 | Percorsi nudi come `--add`, o un avviso | corretto (c1208e8) |

### Plausibili (area D)

| ID | File:riga | Osservazione | Stato |
|---|---|---|---|
| D-P1 | crates/lynkeus-app/src/platform/windows_ffi.rs:1250, 1316, 1468, 1477 | `cancel_and_wait(handle, &overlapped)` crea un `&OVERLAPPED` mentre il kernel può ancora scriverci: UB formale (NOMI); passare `&raw mut`. `owned` (120), `AttributeList::set` (458), `overlapped_io` (1280) sono `fn` safe con precondizioni non verificate: il modulo è sound solo perché sono private; meglio `unsafe fn` o `AttributeList<'a>` | corretto (c1208e8) |
| D-P2 | crates/lynkeus-app/src/context_menu.rs:128 | Un `%` nel percorso dell'exe non è rifiutato: Explorer sostituisce %1/%L/%V anche nella parte dell'eseguibile (es. `C:\Strumenti\50%L\`) e la voce si rompe | corretto (c1208e8) |

### Punti di forza (area D)

- FFI in un solo modulo con SAFETY su ogni blocco e `OwnedHandle`; I/O sovrapposto sempre annullato e atteso, con test.
- Processo estrattore: `JOB_LIST` e `HANDLE_LIST`, ambiente con la sola `SystemRoot`, percorso assoluto, `IsProcessInJob` che fallisce chiuso, `KILL_ON_JOB_CLOSE`.
- Istanza unica: `FIRST_PIPE_INSTANCE`, `REJECT_REMOTE_CLIENTS`, DACL solo utente, SQoS Identification, utente del server verificato dal client, token a 128 bit in tempo costante, tetti prima di allocare.
- Cartelle private: descrittore esplicito protetto, controllo sull'handle senza seguire reparse point, `CREATE_NEW`; registro solo HKCU con voci altrui riconosciute; script Nautilus quotato.
- CI: action fissate a SHA, `contents: read`, `persist-credentials: false`, nessuna interpolazione, stesso `cargo xtask check`; `deny.toml` con `unsound` e `yanked`; percorsi rimappati e scansione dell'exe.

## Fuori ambito e limiti dell'analisi

- Linux è stato solo compilato (`cargo xtask check-linux`); nessun test è stato eseguito su Linux. B-04, A-07, la parte Nautilus e Orca sono quindi letti nel codice, non provati. Vale ancora dopo le correzioni.
- Le misure (Annulla sotto carico, copia di 200.000 righe, `step_target`) sono state prese su una sola macchina, senza distribuzione né ambiente annotato: sono ordini di grandezza, non budget. Il piano di test (livello 3) le ripete con metodo.
- Il fuzzing non è stato eseguito (serve Linux nightly); non ci sono esiti di sessioni. Dopo le correzioni i target hanno gli oracoli (A-P5) e `fuzz.yml` li esegue ogni giorno, ma nessuna sessione è ancora stata fatta.
- Non provati: avvio su Windows 10 pulito senza Visual C++ Redistributable (D-04), clic destro su Windows 11, lettori di schermo reali, OneDrive reale (B-P1).
- Le stime di memoria (B-02) erano calcoli; dopo le correzioni B-02 è misurato da P-03 (picco della memoria privata).
- Non rivisti: gli script di fixture, `docs/`, README, il contenuto di `locales/` oltre agli ID e ai plurali citati.
- Il tracciamento di ogni problema verso i test è in `docs/piano-di-test.md`.

## Seconda revisione

Revisione delle correzioni, in due parti: interfaccia (commit `09be123`) e librerie (commit `69c92c8`). Gli ID sono R-01.. in ordine di commit. Stato al commit `2552454`.

| ID | Problema trovato | Stato |
|---|---|---|
| R-01 | Clic e doppio clic sulla riga non arrivavano per risultato (le celle sentivano i clic) | corretto (09be123): il nodo della riga ha l'id del risultato; il doppio clic apre solo se il primo clic era sullo stesso risultato |
| R-02 | Le righe ordinate che entrano sopra la riga in vista la spostavano | corretto (09be123): scorrimento compensato, senza effetto elastico |
| R-03 | Il focus arrivato con Tab o F6 cambiava la selezione | corretto (09be123): sposta solo il cursore |
| R-04 | Tabella senza struttura di griglia per AccessKit | corretto (09be123): griglia a selezione multipla con numero di righe e colonne, righe figlie con indice |
| R-05 | All'uscita `save_settings` poteva attendere senza limite il file tenuto dal thread di scrittura | corretto (09be123): `try_lock`, la causa va nel log |
| R-06 | Il messaggio "impostazioni non salvate" restava dopo un salvataggio riuscito | corretto (09be123) |
| R-07 | Copia negli appunti di testi enormi sul thread dell'interfaccia (circa 1 ms per MB) | corretto (09be123): oltre 8 MiB (`MAX_CLIPBOARD_BYTES`) niente appunti, un messaggio propone Esporta con le sole righe selezionate |
| R-08 | Test dell'interfaccia che falliva a caso (frame per `Harness::run`) | corretto (09be123): limite portato a 12 |
| R-09 | xls quasi sempre "danneggiato": il file composto rifatto per calamine era di versione 4 | corretto (69c92c8): versione 3 (settori da 512 byte); fixture `registro-lungo.xls` e test che legge ogni cella |
| R-10 | Tetto in celle poco adatto a ods e xls | corretto (69c92c8): stima in byte (`sheet_memory.rs`), tetto 512 MiB; fixture malevola `testo-ripetuto.ods`; le formule degli xls si svuotano prima di misurare |
| R-11 | Documenti da rileggere (AND) non riletti dopo una prima lettura fallita | corretto (69c92c8): la seconda lettura avvisa per quelli che non ha potuto consegnare |
| R-12 | Frame fino a 16 MiB per processo | corretto (69c92c8): 1,25 MiB, `PROTOCOL_VERSION` 3, al più 16 frame in coda (circa 20 MiB) |
| R-13 | Estrattore occupato orfano su Linux (punto 7 della revisione) | codice fatto (69c92c8): `--parent-pid`, `watch_parent` confronta il padre dalla prima lettura e chiama la funzione del chiamante, che nell'app esce con `exit(3)`. Il test Linux (`busy_orphan_exits_with_its_parent`) non è mai stato eseguito: lo farà la CI su GitHub |
| R-14 | Rilettura della cartella della cache ogni 64 scritture: costo lineare nelle voci, per scrittura crescente con la cache | corretto (69c92c8): dopo almeno 64 scritture e almeno una ogni 16 voci dell'indice (50-70 ms con 20.000-30.000 voci) |
| R-15 | Oracoli mancanti per `doc_piece_table`, `ppt_records`, `rtf` | corretto (69c92c8): due letture uguali, paragrafi con testo e senza controlli, tetto ai caratteri; fuzzing non eseguito |
| R-16 | Test del pool con tempi stretti che includevano l'avvio dell'eseguibile | corretto (69c92c8): il processo si avvia prima di misurare |
| R-17 | Un xls con record sovrapposti, dove una formula sfugge allo svuotamento, è rifiutato con l'avviso "memoria" | aperto, accettato: falso positivo che non apre rischi di sicurezza |
