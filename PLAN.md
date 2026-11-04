# Piano: Lynkeus Searcher

## Contesto

Serve un'app desktop che cerchi una porzione di testo in file singoli o in cartelle (con sottocartelle) e restituisca **le parole che la contengono**, con file, posizione e contesto: cercando "ar" → "aragosta" in X, "carta" in Y, "chitarra" in Z. La cartella del progetto è vuota; Rust 1.99.1, git 2.56 e LibreOffice (utile per generare i file di prova) sono installati; WSL no.

Requisiti decisi nell'intervista:
- **Prodotto:** Rust + egui 0.37, Windows 10/11 e Ubuntu, eseguibile portatile, interfaccia italiano/inglese, GPLv3 (`GPL-3.0-only`, copyright `MR`).
- **Partenza:** elenco misto di cartelle (ricorsive) e file, anche trascinati nella finestra.
- **Termini:** una riga per termine (pulsante +), cercato così com'è, spazi compresi; interruttore OR/AND (AND = file che contengono tutti i termini).
- **Regole di confronto:** jolly `*` e `?`; maiuscole ignorate (casella per distinguerle); accenti distinti (casella per ignorarli).
- **Parola restituita:** il testo tra due spazi, senza punteggiatura ai bordi.
- **Formati:** testo e codice, docx/xlsx/pptx/odt/ods/odp, PDF, doc/xls/ppt, RTF, EML, EPUB, zip/7z/tar/gz anche annidati.
- **Filtri:** estensioni, cartelle da saltare, nascosti e di sistema, dimensione massima.
- **Lettura:** diretta a ogni ricerca, con cache del testo estratto da documenti e archivi.
- **Risultati:** tabella Parola | File | Posizione | Contesto evidenziato; apri, mostra nella cartella, anteprima interna, esporta CSV/TXT, copia.
- **Integrazione:** voce "Cerca con Lynkeus" nel clic destro, attivabile dalle impostazioni.
- **Repository e Linux:** repository git locale (GitHub deciso più avanti); Linux verificato dalla CI di GitHub quando il repository verrà pubblicato.

## Architettura

Workspace Cargo a strati, dipendenze in una sola direzione (`app → engine → extract → core`):

| Crate | Ruolo | I/O |
|---|---|---|
| `crates/lynkeus-core` | Query, compilazione dei termini, ricerca in un testo, estrazione della parola, OR/AND, tipi `Location`, `TextUnit`, `Hit` | nessuno |
| `crates/lynkeus-extract` | Decodifica di testo e codifiche, estrattori dei formati, archivi con limiti, processo estrattore (`worker`) e suo protocollo | legge file |
| `crates/lynkeus-engine` | Scansione delle radici e filtri, pianificazione parallela, pool di processi estrattori, cache su disco, flusso dei risultati, annulla e avanzamento | sì |
| `crates/lynkeus-app` | Binario `lynkeus-searcher`: UI egui, `ViewState`, impostazioni, Fluent, `theme.rs`, `platform/` (clic destro, istanza unica, font, apertura file) | sì |
| `xtask` | `cargo xtask check` (fmt, clippy `-Dwarnings`, test, `cargo deny`) e `cargo xtask dist` (rilascio) | — |

Regole trasversali:
- `#![forbid(unsafe_code)]` in core, extract ed engine.
- Nell'app `#![deny(unsafe_code)]` con un solo modulo `platform/windows_ffi.rs` ammesso, con API safe e `// SAFETY:` su ogni blocco. Contiene:
  - `SetDefaultDllDirectories`;
  - `AllowSetForegroundWindow`;
  - la lettura del contrasto elevato;
  - la creazione di cartelle e file privati con descrittore esplicito.
- Errori con `thiserror` nelle librerie e `anyhow` nell'app; nessun panic sui dati letti dai file.
- Tipi validi per costruzione: `Term`, `MaxFileSize`, `ExtensionFilter` e simili.
- Composition root in `main`: `Application::build(Settings)`, usata anche dai test d'accettazione.
- Ogni sorgente ha l'intestazione SPDX e `Copyright (C) 2026 MR`.

### Regole globali e file di progetto

- **`CLAUDE.md` del progetto, versionato**, con solo ciò che è specifico del progetto:
  - lingue: interfaccia italiano/inglese, codice e identificatori in inglese, commenti, documenti e commit in italiano;
  - comandi di controllo: `cargo xtask check` e `cargo xtask dist`;
  - documenti di stato e decisioni;
  - skill da applicare sempre: `software-design-core`, `rust-egui-desktop-design`, `ui-design-core`;
  - skill per argomento:
    - `security-design` per estrattori, archivi, IPC, registro e rilascio;
    - `windows-system-design` per job, pipe, registro, file e cartelle private;
    - `durable-data-design` per cache, impostazioni ed esportazione;
  - eccezioni alle regole, ognuna con il motivo: per ora una sola, la firma degli eseguibili (permanente, vedi sotto).
- **`CLAUDE.local.md`** ignorato da git, per ciò che vale solo su questa macchina (percorso di LibreOffice).
- **Skill del progetto `.claude/skills/lynkeus-internals/`**, con `SKILL.md` breve e dettagli in `references/`: semantica della ricerca, protocollo del processo estrattore, formato della cache e delle impostazioni, limiti.
- **Documenti per le persone in `docs/`:**
  - `specifica.md`;
  - `threat-model.md`: le Quattro Domande, STRIDE, asset, trust boundary (file letti, processo estrattore, pipe, registro) e rischi accettati;
  - `adr/` per le decisioni difficili da cambiare: isolamento in processo, libreria PDF, formato delle impostazioni;
  - `dipendenze.md` con la validazione registrata di ogni dipendenza diretta (motivo, licenza, manutenzione, `unsafe`).
- **`SECURITY.md`** prima della pubblicazione su GitHub.

### Ricerca (lynkeus-core, puro e testabile)

- **Compilazione:** ogni termine diventa una regex del crate `regex` (lineare, sicuro con input dell'utente; la regex non arriva mai grezza dall'utente):
  - letterali escapati, con il termine prima portato in NFC;
  - `*` → `\S*` (greedy), `?` → `\S`;
  - spazi → `[^\S\r\n]+`, così nessuna corrispondenza attraversa una riga;
  - maiuscole ignorate con `(?i)`;
  - accenti ignorati: ogni lettera diventa la classe delle sue varianti accentate più `\p{Mn}*`, con una tabella costruita una volta dalla decomposizione Unicode (`LazyLock`);
  - accenti distinti: ogni lettera accentata accetta anche la forma decomposta (NFD), quindi il testo non va normalizzato e le posizioni restano quelle originali.
- **Validazione:** un termine vuoto o fatto solo di `*`/`?` è un errore mostrato sotto il campo. Con i jolly spenti `*` e `?` sono letterali.
- **Parola:** a ogni corrispondenza si estende la parola fino agli spazi (proprietà Unicode White_Space) a sinistra e a destra, poi si tolgono i segni di punteggiatura ai bordi, mai dentro la corrispondenza.
- **Righe dei risultati:**
  - più corrispondenze nella stessa parola, anche di termini diversi, danno una sola riga con tutti gli evidenziati;
  - parole lunghe oltre 200 caratteri vengono troncate con "…";
  - il contesto è di circa 120 caratteri intorno alla parola.
- **AND:** `FileAccumulator` trattiene i risultati di un file finché il file è finito e li scarta se manca un termine. Un file dentro un archivio conta come file a sé.
- **Posizione:** `enum Location`, con un valore per formato:
  - `Line` per i file di testo;
  - `PageLine` per i PDF;
  - `Cell { sheet, cell }` per i fogli di calcolo;
  - `Slide` per le presentazioni;
  - `Paragraph` per Word, ODT, RTF ed EPUB;
  - `Mail(part)` per le e-mail.

### Scansione e testo (engine + extract)

- **Scansione:**
  - `ignore::WalkBuilder` parallelo con i filtri standard (gitignore) spenti e i link simbolici e le junction non seguiti;
  - `filter_entry` per le cartelle da saltare (default `.git`, `node_modules`, `target`, `$RECYCLE.BIN`, `System Volume Information`), per i nascosti e di sistema (attributi Windows o punto iniziale) e per estensioni e dimensione;
  - i **file solo online** (attributo `RECALL_ON_DATA_ACCESS`, es. OneDrive) vengono saltati per default, perché leggerli li scaricherebbe; una casella li include.
- **File di testo, nel processo principale:**
  - un file binario (NUL nei primi 8 KB senza schema UTF-16, oppure firma nota) viene saltato e contato;
  - codifica: BOM UTF-8/16, poi UTF-8 valido, poi `chardetng` con ripiego Windows-1252;
  - lettura a blocchi di circa 8 MB tagliati a fine riga, regex sul blocco intero e numero di riga calcolato con `memchr`, così la memoria resta limitata anche con file grandi.
- **Formato:** si sceglie dall'estensione e si verifica dal contenuto. Se non coincidono si legge con l'estrattore del contenuto solo nei casi comuni e innocui (un .doc che in realtà è RTF, un .xls che è HTML o CSV); negli altri casi il file va negli avvisi.
- **Parallelismo:** pool `rayon`, `cancel: AtomicBool` controllato tra un file e l'altro e ogni N righe, avanzamento con atomici `Relaxed`, risultati in lotti per file su un canale `crossbeam` limitato.
- **Limite dei risultati:** 200.000 per default, modificabile. Al raggiungimento la ricerca si ferma e un banner invita a restringerla.

### Documenti in un processo isolato

- **Processo estrattore:** PDF, Office, RTF, EML, EPUB e archivi sono decoder di formati complessi e non fidati, quindi girano in processi figli: lo stesso eseguibile con l'opzione nascosta `--extract-worker`.
  - Il pool ha `min(core/2, 4)` processi, avviati al primo documento.
  - Protocollo: messaggi `postcard` con lunghezza in testa e dimensione massima, su stdin e stdout.
  - Limiti:
    - timeout di 60 s per file: oltre, il processo viene terminato e ne parte un altro;
    - su Windows il processo nasce dentro un Job Object con limite di memoria per processo e `KILL_ON_JOB_CLOSE`, creato con `CreateProcessW` (`PROC_THREAD_ATTRIBUTE_JOB_LIST` e `HANDLE_LIST`: eredita solo stdin e stdout) in `platform/windows_ffi.rs` dell'app e passato all'engine tramite un trait (`win32job` non ha il limite per processo, vedi experiments/jobobject/ESITI.md);
    - su Linux il processo imposta da sé `RLIMIT_AS` (`rlimit`) ed esce quando il suo stdin si chiude.
  - Annulla manda prima una richiesta di stop, chiudendo lo stdin, e forza la chiusura dopo 500 ms. Un processo bloccato oltre il timeout viene terminato subito.
  - Il processo si avvia dal percorso assoluto di `current_exe()`, con l'ambiente svuotato (`env_clear`) tranne le poche variabili di sistema necessarie, e senza handle ereditati oltre a stdin e stdout.
- **Estrattori:**
  - `zip` + `quick-xml` (DTD rifiutati) per docx, pptx, odt, odp ed EPUB;
  - `calamine` per xlsx, xls e ods;
  - `pdf_oxide` per i PDF con `extract_text_lines` pagina per pagina, senza ripiego (ADR 0002);
  - `cfb` più un parser proprio per .doc (piece table) e .ppt (atomi di testo per diapositiva);
  - un tokenizer RTF proprio (gruppi, `\'hh` con code page, `\uN`);
  - `mail-parser` per gli EML, con gli allegati estratti a loro volta;
  - `sevenz-rust2`, `tar` e `flate2` per gli archivi.
- **Limiti sugli archivi** (anche docx e odt), contati sui byte decompressi con `Read::take` e modificabili in "Avanzate":
  - profondità 5;
  - 100.000 voci;
  - 512 MB per voce e 4 GB per archivio;
  - rapporto di compressione massimo 200:1.
  - I nomi delle voci si trattano solo come etichette e non si scrivono mai su disco con quel nome.
- **Avvisi**, ognuno con il suo motivo: protetto da password, danneggiato, tempo scaduto, limite superato, contenuto diverso dall'estensione, PDF senza testo (probabile scansione), accesso negato o file in uso.

### Cache

- **Posizione:** `%LOCALAPPDATA%\Lynkeus Searcher\cache` su Windows, `~/.cache/lynkeus-searcher` su Linux (crate `directories`).
- **Chiave:** `blake3` di percorso, dimensione, data di modifica, versione degli estrattori e limiti.
- **Contenuto:** gli stessi messaggi del protocollo del processo estrattore, compressi con `lz4_flex`, preceduti da un'intestazione con versione e CRC.
- **Scrittura:** su un file temporaneo nella stessa cartella, poi rinominato. Una voce corrotta viene ignorata e cancellata. La cache è un derivato rigenerabile, quindi niente flush; impostazioni ed esportazioni invece si scrivono con temporaneo, `sync_all` e rinomina nello stesso volume.
- **Cartelle private:** cartella dati, cache, log, rapporti di crash e cartella temporanea per aprire le voci degli archivi. Su Windows si creano con un descrittore esplicito (solo l'utente e SYSTEM), su Linux con permessi 0700 per le cartelle e 0600 per i file. I file si creano in modo che la creazione fallisca se esistono già.
- **Pulizia:** limite LRU di 500 MB; in Impostazioni si può spegnere la cache e svuotarla, e il README avverte che la cache contiene testo dei documenti.

### Interfaccia (lynkeus-app)

- **Finestra:**
  - `Panel::top` con il menu (File, Modifica, Ricerca, Visualizza, Strumenti, Aiuto);
  - `Panel::left` con il modulo di ricerca: dove cercare, termini, opzioni e "Filtri" in un `CollapsingHeader`;
  - `CentralPanel` con le schede Risultati e Avvisi, e l'anteprima ridimensionabile sotto la tabella;
  - `Panel::bottom` con lo stato.
- **Modulo di ricerca:**
  - il pulsante primario Cerca diventa Annulla durante la ricerca;
  - le opzioni sono le caselle "Distingui maiuscole e minuscole" (spenta), "Ignora gli accenti" (spenta) e "Caratteri jolly * e ?" (accesa);
  - la scelta "qualsiasi termine / tutti i termini nello stesso file" è attiva solo con almeno 2 termini, altrimenti un tooltip ne spiega il motivo.
- **Tabella dei risultati:**
  - `egui_extras::TableBuilder` virtualizzata, ordinabile per colonna, con selezione multipla (Ctrl/Shift);
  - la colonna Termine compare solo con più termini;
  - il percorso è relativo alla radice, quello completo nel tooltip;
  - a fine ricerca l'ordine diventa File → Posizione, salvo un ordinamento scelto dall'utente.
- **Stato:**
  - si vedono file letti, saltati, binari, corrispondenze, avvisi, tempo trascorso e file in lettura;
  - una barra d'avanzamento compare dopo 1 s;
  - il job gira con il pattern di `egui-pattern.md`: thread con nome, `try_recv` in `App::logic`, al massimo N risultati inseriti per frame, contatore di generazione.
- **Azioni sui risultati:**
  - doppio clic o Invio apre il file (`opener`); una voce dentro un archivio viene prima estratta in una cartella temporanea dell'app, cancellata all'uscita;
  - "Mostra nella cartella" usa `opener::reveal`;
  - l'anteprima mostra l'unità di testo con ±20 unità vicine, le corrispondenze evidenziate e i comandi precedente/successiva;
  - l'esportazione produce CSV (UTF-8 con BOM, `;` in italiano e `,` in inglese) o TXT tabulato, scritto su un temporaneo poi rinominato;
  - Ctrl+C copia le righe selezionate come TSV.
- **Aspetto e accessibilità:**
  - `theme.rs` con i token e il test dei contrasti; tema Sistema/Chiaro/Scuro;
  - Segoe UI caricato da `C:\Windows\Fonts` su Windows, Noto Sans incluso su Linux, font di sistema di ripiego per CJK;
  - AccessKit e nomi accessibili sui bottoni con sola icona;
  - layout provato a 1024×600.
- **Persistenza:**
  - file `settings.toml` proprio, versionato, con i campi sconosciuti conservati e scrittura durevole: radici e termini recenti, opzioni, filtri, larghezza dei pannelli, ordinamento, lingua, tema, impostazioni della cache;
  - eframe ricorda solo la posizione della finestra.
- **Lingue:**
  - ogni stringa visibile passa da Fluent fin dalla fase 0 (`fluent-templates`, `locales/it` e `locales/en`), con ID stabili, commenti per le variabili, plurali CLDR e numeri formattati per lingua;
  - lingua iniziale da `sys-locale`, scelta nelle Impostazioni;
  - un test verifica che le due lingue abbiano gli stessi ID.

### Integrazione con il sistema (`platform/`)

- **Windows:**
  - voci in `HKCU\Software\Classes` (`Directory\shell`, `Directory\Background\shell`, `*\shell`) con `winreg`, senza diritti di amministratore;
  - comando `"<exe>" --add "%1"`, aggiornato all'avvio se l'eseguibile è stato spostato e rimosso quando l'utente spegne l'opzione;
  - su Windows 11 la voce compare in "Mostra altre opzioni", come concordato.
- **Linux:** uno script Nautilus in `~/.local/share/nautilus/scripts/`.
- **Istanza unica, solo con `--add`:**
  - a ogni avvio con `--add` Explorer lancia un processo per elemento selezionato, quindi la seconda istanza passa i percorsi alla prima ed esce;
  - un file di lock nella cartella privata (`fs4`) individua la prima istanza;
  - Windows:
    - la pipe con nome casuale si crea con `FILE_FLAG_FIRST_PIPE_INSTANCE` e una DACL esplicita (solo l'utente corrente);
    - il client la apre con `security_qos_flags(SECURITY_IDENTIFICATION)` di `OpenOptionsExt`;
  - Linux: socket Unix in `$XDG_RUNTIME_DIR` con permessi 0600;
  - autenticazione: ogni messaggio porta un token casuale da 128 bit letto da un file privato e confrontato in tempo costante;
  - ogni messaggio ha una dimensione massima e si accettano solo percorsi esistenti, aggiunti all'elenco senza avviare la ricerca;
  - la seconda istanza chiama `AllowSetForegroundWindow` prima di chiedere alla prima di mettersi in primo piano.
- **Eseguibile Windows:** manifest con `asInvoker` e `longPathAware`, icona .ico 16–256 e informazioni di versione con `winresource`.

### Qualità, build e rilascio

- **Profili:** `overflow-checks = true` nei crate di dominio e `panic = "unwind"`. Un panic hook scrive un rapporto nella cartella dati; `tracing` scrive in file con rotazione.
- **File di progetto:** `Cargo.lock` committato, `rust-toolchain.toml`, `.gitattributes`, `.gitignore`, `LICENSE` (testo GPLv3), `README.md`, `docs/specifica.md`.
- **Licenze delle dipendenze:** `deny.toml` con allowlist (MIT, Apache-2.0, BSD, 0BSD, Zlib, Unicode-3.0, MPL-2.0, OFL per i font), `unsound = "all"`, `yanked = "deny"`. Tutte le librerie scelte sono compatibili con GPLv3.
- **CI:**
  - `.github/workflows/ci.yml` su Windows e Ubuntu esegue esattamente `cargo xtask check`, con action fissate a SHA e token in sola lettura;
  - un job pianificato fa `cargo deny check` ogni giorno e il fuzzing (`cargo-fuzz`, nightly su Ubuntu) dei parser propri: .doc, .ppt, RTF, frame del protocollo, lettore della cache;
  - la CI parte quando il repository sarà su GitHub; fino ad allora in locale gira `cargo check --target x86_64-unknown-linux-gnu`.
- **Opzioni del compilatore:** solo in file versionati (`.cargo/config.toml` con `-C control-flow-guard` e `/CETCOMPAT` per il target Windows). Per togliere i percorsi locali:
  - `trim-paths` nel profilo release, se è stabile nella toolchain;
  - altrimenti `--remap-path-prefix` aggiunto da xtask con `--config`, con valori calcolati al momento e mai scritti in un file.
- **`cargo xtask dist`:**
  - build `--locked`;
  - verifica di CFG e CETCOMPAT con `dumpbin`, trovato per percorso assoluto tramite `vswhere`;
  - controllo finale che l'eseguibile non contenga il nome utente né percorsi locali;
  - zip con exe, LICENSE e README, più `SHA256SUMS`.
- **Firma: eccezione permanente alla regola 54, decisa dall'utente.**
  - Il `CLAUDE.md` del progetto la scrive con il motivo: progetto personale senza certificato di firma del codice.
  - Accanto allo zip si pubblica il file `SHA256SUMS`.
  - Il README spiega l'avviso di SmartScreen e come verificare l'hash con `Get-FileHash` o `sha256sum`.
  - Il resto della regola 54 vale per intero.
- **Git:**
  - `git init` con `user.name MR` e `user.email MR1999develop@users.noreply.github.com` prima del primo commit;
  - un commit locale alla fine di ogni fase con controlli verdi, nessun push.

## Fasi (checklist)

Ogni fase finisce con `cargo xtask check` verde e un commit. Il lavoro si orchestra con i sub-agenti di `.claude/agents/`, in parallelo dove i file non si sovrappongono.

### Squadra di agenti
- [x] File degli agenti in `.claude/agents/`

### Fase 0 — Fondamenta
- [x] Repository git con autore MR
- [x] `LICENSE`, `.gitignore`, `.gitattributes`, `rust-toolchain.toml`, `.cargo/config.toml`, `deny.toml`
- [x] `xtask` (`check`, `dist`) e CI `.github/workflows/ci.yml`
- [x] `README.md`, `CLAUDE.md`, `CLAUDE.local.md` (ignorato), skill `.claude/skills/lynkeus-internals/`
- [x] `docs/specifica.md`, `docs/threat-model.md`, `docs/dipendenze.md`
- [x] Workspace e crate con i contratti delle API (core, extract, engine, app)
- [x] Scheletro end-to-end: finestra egui, una cartella, un termine, ricerca nei .txt, testi in Fluent
- [x] Test d'accettazione avviato da `Application::build`
- [x] `cargo xtask check` verde e commit

### Fase 1 — Motore di ricerca (core)
- [x] Compilazione dei termini: letterali, jolly, frasi, maiuscole, accenti (anche NFD)
- [x] Validazione dei termini
- [x] Parola fino agli spazi e punteggiatura ai bordi; troncamento e contesto
- [x] Unione dei risultati per parola e per termine; OR/AND con `FileAccumulator`
- [x] `Location` per tutti i formati
- [x] Test a partizioni e `proptest` (parola contiene la corrispondenza, niente a capo, confini UTF-8, blocchi = intero)
- [x] Benchmark `criterion`
- [x] Controlli verdi e commit

### Fase 2 — Scansione e file di testo
- [x] Radici miste (cartelle e file)
- [x] Filtri: estensioni, cartelle da saltare, nascosti e di sistema, dimensione massima, file solo online
- [x] Rilevamento dei binari e delle codifiche (BOM, UTF-8, chardetng, Windows-1252)
- [x] Lettura a blocchi con numeri di riga
- [x] Parallelismo, annulla, avanzamento, avvisi, limite dei risultati
- [x] Test d'integrazione su cartelle di prova
- [x] Controlli verdi e commit

### Fase 3 — Interfaccia principale
- [x] Layout (menu, modulo a sinistra, risultati/avvisi al centro, stato in basso) e `theme.rs`
- [x] Radici con drag & drop e dialoghi `rfd`
- [x] Righe dei termini con validazione, opzioni, OR/AND, filtri
- [x] Tabella virtualizzata ordinabile con evidenziazione e selezione multipla
- [x] Barra di stato e scheda Avvisi
- [x] Menu e scorciatoie
- [x] Persistenza in `settings.toml` (scrittura durevole)
- [x] Test `egui_kittest` ("ar" trova "aragosta")
- [x] Controlli verdi e commit

### Fase 4 — Processo isolato e documenti moderni
- [x] Threat model del processo estrattore e degli archivi; ADR isolamento
- [x] Prototipi in `experiments/` (`pdf_oxide`, Job Object) e ADR libreria PDF
- [ ] Processo estrattore, protocollo, pool, Job Object / `RLIMIT_AS`, timeout, stop
- [ ] Estrattori docx, xlsx, pptx, ODF, PDF
- [ ] Archivi zip annidati con limiti
- [ ] Cache su disco
- [ ] File di prova (LibreOffice + casi malevoli)
- [ ] Controlli verdi e commit

### Fase 5 — Altri formati
- [ ] .doc, .xls, .ppt
- [ ] RTF
- [ ] EML con allegati
- [ ] EPUB
- [ ] 7z, tar, gz, tgz
- [ ] File di prova
- [ ] Controlli verdi e commit

### Fase 6 — Azioni sui risultati
- [ ] Anteprima con navigazione
- [ ] Apri (anche voci dentro archivi) e mostra nella cartella
- [x] Esporta CSV/TXT e copia TSV
- [ ] Controlli verdi e commit

### Fase 7 — Lingue, integrazione e rilascio
- [ ] Threat model della pipe e del registro
- [ ] Traduzione inglese completa
- [ ] Clic destro su Windows e script Nautilus
- [ ] Istanza unica con pipe autenticata
- [ ] Cartelle private con descrittore esplicito
- [ ] Icona, manifest, informazioni di versione
- [ ] `xtask dist` con `SHA256SUMS` e controllo dei percorsi locali
- [ ] README d'uso e `SECURITY.md`
- [ ] Verifica finale (sezione Verifica) e commit

## Verifica

- **Ogni fase:** `cargo xtask check` (fmt, clippy `-Dwarnings`, test, `cargo deny check`) e `cargo check --target x86_64-unknown-linux-gnu`.
- **Fine a fine:** avvio dell'app su una cartella `fixtures/demo` con "aragosta" (txt), "carta" (docx), "chitarra" (PDF dentro uno zip):
  - con "ar": tre righe con parola, file, posizione e contesto evidenziato;
  - con "c?rt*", con una frase ("la car") e con AND/OR tra due termini;
  - con accenti ("citta" e "città") e maiuscole.
- **Robustezza:** la cartella dei file malevoli produce avvisi e nessun blocco; Annulla durante un PDF lento risponde entro 1 s.
- **Prestazioni:** una cartella generata con 100.000 file di testo, misurando il tempo e la memoria massima in release; la UI resta fluida con 200.000 risultati.
- **Clic destro (Windows):** si attiva dalle Impostazioni, si fa clic destro su una cartella, l'app si apre con la cartella in elenco; selezionando 3 cartelle si ottiene una sola finestra.
- **Sicurezza:**
  - un client con un token sbagliato viene rifiutato (test);
  - la DACL della pipe e delle cartelle private si controlla con `icacls` o `Get-Acl`;
  - quando l'app si chiude, i processi estrattori spariscono, anche se l'app viene terminata da Task Manager.
- **Rilascio:** `cargo xtask dist`, poi la ricerca del nome utente e del percorso del progetto nell'exe deve dare zero risultati, e `SHA256SUMS` deve corrispondere allo zip.
- **Linux:** test completi dalla CI di GitHub dopo la pubblicazione del repository; fino ad allora solo il controllo di compilazione.

## Rischi e limiti noti

- **Qualità del testo nei PDF:** colonne, legature e ordine di lettura dipendono da `pdf_oxide`. Il prototipo della fase 4 decide tra `pdf_oxide` e `pdf-extract`.
- **.doc e .ppt:** si estrae il testo principale; commenti, note e caselle di testo potrebbero mancare.
- **Frasi spezzate:** una frase divisa da un a capo (frequente nei PDF) non viene trovata, come concordato.
- **Esclusi:** .rar (licenza) e PDF scansionati (serve OCR).
- **Linux:** senza WSL o CI attiva l'interfaccia su Ubuntu non è provata finché il repository non va su GitHub.
