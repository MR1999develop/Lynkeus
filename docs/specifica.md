# Specifica di Lynkeus Searcher

Requisiti decisi con l'utente e semantica esatta della ricerca. La fonte delle decisioni è `PLAN.md`.

## 1. Scopo

Cercare una porzione di testo in file singoli o in cartelle (con sottocartelle) e restituire **le parole che la contengono**, con file, posizione e contesto. Esempio: `ar` → "aragosta" in X, "carta" in Y, "chitarra" in Z.

## 2. Requisiti

- Rust + egui 0.37, Windows 10/11 e Ubuntu, eseguibile portatile.
- Interfaccia in italiano e inglese; licenza GPLv3 (`GPL-3.0-only`).
- Partenza: elenco misto di cartelle (ricorsive) e file, anche trascinati nella finestra.
- Lettura diretta a ogni ricerca: i file di testo si leggono nel processo principale, i documenti e gli archivi (PDF, Office, ODF, zip) in processi estrattori isolati (ADR 0001), con cache su disco del testo estratto.

## 3. Termini

- Una riga per termine, aggiunta con il pulsante +.
- Ogni termine è cercato così com'è, spazi interni compresi; gli spazi (White_Space Unicode) iniziali e finali sono tolti, altrimenti la corrispondenza inizierebbe con uno spazio e la parola si estenderebbe oltre.
- Un termine vuoto o di soli spazi, fatto solo di `*`, `?` e spazi (con i jolly accesi), con un a capo dentro, oltre 256 caratteri, o oltre 32 termini, è un errore mostrato sotto il campo.
- Con i jolly spenti `*` e `?` sono caratteri letterali.

## 4. Regole di confronto

- **Jolly:** `*` = zero o più caratteri non spazio; `?` = un carattere non spazio. Un "carattere" è una lettera con i suoi segni combinanti: `?` = `[^\s\p{Mn}]\p{Mn}*`, `*` = `(?:[^\s\p{Mn}]\p{Mn}*)*`. Così su testo decomposto (NFD) un jolly non separa una lettera dai suoi accenti: "e*" non trova la "é" decomposta, come non trova quella precomposta. Più `*` di fila valgono uno solo.
- **Punteggiatura e `*` di bordo:** se il termine inizia (finisce) con `*`, dalla parte catturata da quel `*` si tolgono i segni di punteggiatura iniziali (finali): "c?rt*" su "carta," evidenzia "carta". Quella dentro la parola resta ("c*" su "c.a.r." evidenzia "c.a.r") e quella scritta nel termine non si tocca mai. Gli spazi ai bordi della parte catturata si tolgono allo stesso modo.
- **Spazi:** uno spazio nel termine corrisponde a uno o più spazi orizzontali; nessuna corrispondenza attraversa un a capo (una frase spezzata da un a capo non si trova).
- **Maiuscole:** ignorate per impostazione predefinita; una casella le distingue.
- **Accenti:** distinti per impostazione predefinita; una casella li ignora.
  - Accenti distinti: una lettera accentata nel termine accetta anche la sua forma decomposta (NFD) nel testo, quindi il testo non viene normalizzato e le posizioni restano quelle originali. Il termine è prima portato in NFC. Una corrispondenza non può essere seguita da un segno combinante: "citta" non trova la "città" decomposta. Il crate `regex` non ha il lookahead, quindi il pattern finisce con un gruppo che consuma il carattere seguente e il chiamante taglia la corrispondenza all'inizio del gruppo.
  - Accenti ignorati: ogni lettera accetta le sue varianti accentate e gli eventuali segni combinanti. Le lettere senza decomposizione canonica (ø, ł, æ) restano distinte da "o", "l", "ae" anche con gli accenti ignorati.
- **Parola restituita:** il testo tra due spazi (White_Space Unicode) attorno alla corrispondenza, senza i segni di punteggiatura ai bordi; la punteggiatura dentro la corrispondenza non si toglie mai.
- **Righe:** più corrispondenze nella stessa parola, anche di termini diversi, danno una sola riga con tutti gli evidenziati.
- **Troncamenti:** parole oltre 200 caratteri troncate con "…"; contesto di circa 120 caratteri intorno alla parola.

## 5. OR e AND

- OR: la riga compare se corrisponde almeno un termine.
- AND: il file compare solo se contiene tutti i termini. I risultati di un file si trattengono finché il file è finito e si scartano se manca un termine. Un file dentro un archivio conta come file a sé.
- La scelta è attiva solo con almeno 2 termini; altrimenti un tooltip ne spiega il motivo.

## 6. Posizioni per formato

| Formato | Posizione |
|---|---|
| File di testo | riga |
| PDF | pagina e riga |
| Fogli di calcolo (xlsx, xls, ods) | foglio e cella |
| Presentazioni (pptx, ppt, odp) | diapositiva |
| Word, ODT, RTF, EPUB | paragrafo |
| E-mail (EML) | parte del messaggio |

## 7. Formati

Testo e codice, docx, xlsx, pptx, odt, ods, odp, PDF, doc, xls, ppt, RTF, EML (con allegati estratti a loro volta), EPUB, zip, 7z, tar, gz e tgz anche annidati. Esclusi: .rar (licenza) e PDF scansionati (serve l'OCR).

Il formato si sceglie dall'estensione e si verifica dal contenuto; se non coincidono si usa l'estrattore del contenuto solo nei casi comuni e innocui (.doc che è RTF, .xls che è HTML o CSV), altrimenti il file va negli avvisi.

**File di testo.** La codifica si decide sul primo blocco letto (almeno 8 KB): BOM, firme binarie, NUL (salvo schema UTF-16), UTF-8 valido, altrimenti codifica a 8 bit rilevata (ripiego Windows-1252). Se il file è ASCII o UTF-8 nel primo blocco (fino a 8 MB) e ha byte non validi più avanti, questi diventano U+FFFD. Le righe finiscono con `\n`; un file con soli `\r` (vecchio Mac) è una riga sola, tagliata dopo l'ultimo spazio a ogni blocco, per cui una frase o una parola a cavallo del taglio può non essere trovata.

**Documenti cercati.** Ogni documento (il file stesso o una voce d'archivio) è cercato per conto suo: OR e AND valgono per documento, e il risultato porta il file più il percorso delle voci. Il testo arriva dalla cache, se c'è una voce valida per quel file (stesso percorso, dimensione, data di modifica, versione del programma e limiti), altrimenti da un processo estrattore. Un guasto dell'estrattore (tempo scaduto, memoria superata, uscita improvvisa) dà un avviso per quel documento e la ricerca continua. Tutti i formati dell'elenco sono letti.

**Formati della fase 5.**

- **doc (Word 97-2003):** il testo principale, un paragrafo per unità. Non si estraggono note, intestazioni e piè di pagina, commenti, caselle di testo; Word 6 e 95 (FIB più vecchia) non sono letti e danno l'avviso "danneggiato".
- **xls:** celle come per xlsx. Un `.xls` che è una pagina HTML o testo (CSV, tabulato) si legge lo stesso (casi innocui, vedi sopra).
- **ppt (PowerPoint 97-2003):** il testo delle diapositive, nell'ordine della presentazione. Non si estraggono note del relatore, schemi (master), commenti e testo negli oggetti incorporati (grafici, tabelle OLE).
- **RTF:** un'unità per paragrafo; intestazioni, piè di pagina, note e commenti, immagini e oggetti sono saltati. Gruppi annidati oltre 256: "danneggiato".
- **EML:** oggetto, mittente, destinatari (To e Cc) e corpo (testo, o HTML ridotto a testo). Gli allegati vengono dopo come documenti a sé (un messaggio allegato si legge come EML), con i limiti degli archivi: gli allegati contano come un livello di profondità.
- **EPUB:** ogni capitolo della spine è un documento, un'unità per blocco di testo. Capitoli cifrati: "protetto da password".
- **7z:** come zip, con i limiti degli archivi; voci cifrate (AES): "protetto da password"; collegamenti saltati.
- **tar, gz, tgz, tar.gz:** come zip. Un `.gz` contiene un documento (nome senza `.gz`); se contiene un tar, le voci stanno direttamente sotto il file. Collegamenti, dispositivi e file sparsi sono saltati.

## 8. Filtri

- Estensioni.
- Cartelle da saltare (predefinite: `.git`, `node_modules`, `target`, `$RECYCLE.BIN`, `System Volume Information`).
- File nascosti e di sistema: su Windows solo per attributo (`HIDDEN` o `SYSTEM`, un nome con il punto non basta); su Linux per il punto iniziale.
- Dimensione massima.
- File solo online (OneDrive; attributi `RECALL_ON_DATA_ACCESS`, `RECALL_ON_OPEN`, `OFFLINE`): saltati per impostazione predefinita, anche se sono una radice scelta dall'utente; una casella li include. Gli attributi si leggono dall'elenco della cartella, senza aprire il file e quindi senza scaricarlo.
- Una radice (cartella o file) scelta dall'utente non è mai esclusa per nome, estensione o attributi nascosto e di sistema; dimensione massima (predefinita 200 MiB) e stato solo online valgono anche per lei.
- Radici duplicate (stesso percorso, anche scritto in modo diverso o con altre maiuscole su Windows; il confronto è sul testo, senza risolvere i link) sono una sola, la prima. Una radice dentro un'altra resta una radice a sé: l'esterna salta quel ramo e l'interna lo visita con le sue regole. Ogni file si legge una volta sola. I luoghi sono al più 256: oltre, il nuovo luogo non entra (anche se arriva dal clic destro) e un avviso sotto il campo lo dice; le impostazioni lette si fermano a 256.
- Link simbolici e junction non seguiti; i filtri gitignore non si applicano.

## 9. Limiti

Vedi `.claude/skills/lynkeus-internals/references/limiti.md`: risultati (200.000), archivi (profondità 5, 100.000 voci, 512 MB per voce, 4 GB per archivio, rapporto 200:1), timeout 60 s per file, memoria 1 GiB per processo estrattore, cache 500 MB. I limiti sugli archivi sono modificabili in "Avanzate".

## 10. Avvisi

Ogni avviso ha il suo motivo: protetto da password, danneggiato, tempo scaduto, limite superato, contenuto diverso dall'estensione, PDF senza testo (probabile scansione), accesso negato o file in uso. I file binari sono saltati e contati.

## 11. Interfaccia

- Menu (File, Modifica, Ricerca, Visualizza, Strumenti, Aiuto); modulo di ricerca a sinistra; schede Risultati e Avvisi al centro con l'anteprima ridimensionabile sotto la tabella; barra di stato in basso.
- Cerca diventa Annulla durante la ricerca.
- Opzioni: "Distingui maiuscole e minuscole" (spenta), "Ignora gli accenti" (spenta), "Caratteri jolly * e ?" (accesa).
- Tabella virtualizzata e ordinabile, selezione multipla; la colonna Termine compare solo con più termini; percorso relativo alla radice, completo nel tooltip; a fine ricerca ordine File → Posizione, salvo scelta dell'utente.
- Stato: file letti, saltati, binari, corrispondenze, avvisi, tempo trascorso, file in lettura; barra di avanzamento dopo 1 s.
- Al raggiungimento del limite dei risultati un banner invita a restringere la ricerca.
- Tema Sistema/Chiaro/Scuro, accessibilità con AccessKit, layout provato a 1024×600.

## 12. Azioni sui risultati

- Doppio clic o Invio apre il file con il programma predefinito del sistema, mai con un programma cercato per nome.
  - Una voce dentro un archivio, di qualsiasi formato (testo, PDF, Office, …), è prima estratta dal processo estrattore con i suoi byte originali nella cartella temporanea privata dell'app, in una sottocartella propria di questa istanza: all'avvio si cancellano solo le sottocartelle delle istanze finite, all'uscita la propria. Il file ha un nome generato dal programma (`voce-<cifre esadecimali>.<estensione>`), non il nome della voce; l'estensione è quella della voce se fatta di lettere e cifre ASCII (al più 16), altrimenti `txt` (voce senza formato noto) o `bin`. Vale il limite di dimensione per voce degli archivi (512 MiB predefiniti). Se l'estrazione non è disponibile o fallisce, un messaggio lo dice e il file a metà è cancellato.
  - Un capitolo di un EPUB non è un file: "Apri" apre il libro (estraendolo prima, se sta in un archivio).
  - Le voci con lo stesso nome nello stesso contenitore sono distinte: la prima tiene il nome, le seguenti diventano "nome (2).ext", "nome (3).ext" ("nome (2)" senza estensione).
  - Non si aprono i file che il sistema eseguirebbe o interpreta (si guarda l'estensione, senza distinguere maiuscole e minuscole e ignorando punti e spazi finali; per una voce, il suo nome): `exe`, `com`, `bat`, `cmd`, `pif`, `scr`, `cpl`, `msc`, `msi`, `msp`, `mst`, `ps1`, `psm1`, `psd1`, `ps1xml`, `psc1`, `vbs`, `vbe`, `js`, `jse`, `wsf`, `wsh`, `wsc`, `hta`, `lnk`, `url`, `reg`, `inf`, `scf`, `jar`, `application`, `appref-ms`, `settingcontent-ms`, `library-ms`, `desktop`, `sh`, `run`, `appimage`, `py`, `pyw`, `chm`, `xll`, `msix`, `msixbundle`, `appx`, `appxbundle`, `appinstaller`, `diagcab`, `ws`, `rdp`, `wsb`, `search-ms`, `searchconnector-ms`, `website`, `gadget`. Resta "Mostra nella cartella"; una voce d'archivio con una di queste estensioni, se estratta, avrebbe comunque estensione `bin`.
- "Mostra nella cartella" (Ctrl+Maiusc+R); per una voce d'archivio mostra l'archivio. Le stesse azioni sono nel menu del clic destro sulla riga.
- Anteprima (Ctrl+3 la mostra o nasconde; altezza ridimensionabile): per un file di testo le righe da −20 a +20 intorno al risultato (ognuna al massimo 4 KiB), con le corrispondenze evidenziate, ricalcolate con la stessa query della ricerca; precedente/successiva. Per i documenti e le voci d'archivio mostra le unità di testo (paragrafi, celle, righe di pagina, diapositive) fino a 20 prima e 20 dopo, lette dalla cache o rilette dal processo estrattore; se il documento non si legge più, resta il contesto del risultato. Le unità di più righe con posizione a righe si dividono in una per riga.
- Esporta (Ctrl+E; Ctrl+Maiusc+E apre le impostazioni di esportazione): tutte le righe mostrate o solo le selezionate, nell'ordine della tabella. Colonne Parola, Termine (solo con più termini), File (percorso completo), Posizione, Contesto.
  - CSV: UTF-8 con BOM, `;` in italiano e `,` in inglese, righe CRLF, campi tra virgolette secondo RFC 4180.
  - TXT: UTF-8 con BOM, colonne separate da tabulazioni, tabulazioni e a capo nelle celle resi spazi.
  - Le celle che iniziano con `=`, `+`, `-`, `@`, tabulazione o CR hanno un apostrofo davanti (iniezione di formule, threat model 4.6).
  - Scrittura atomica e durevole; formato, righe e ultima cartella si ricordano.
- Ctrl+C copia le righe selezionate come TSV (senza intestazione né BOM, stessa neutralizzazione delle formule).

Scorciatoie (tabella in `crates/lynkeus-app/src/ui/commands.rs`):

| Comando | Scorciatoia |
|---|---|
| Aggiungi cartella / file | Ctrl+O / Ctrl+Maiusc+O |
| Apri | Invio |
| Mostra nella cartella | Ctrl+Maiusc+R |
| Esporta / impostazioni di esportazione | Ctrl+E / Ctrl+Maiusc+E |
| Esci | Ctrl+Q |
| Copia | Ctrl+C |
| Seleziona tutto / niente | Ctrl+A / Ctrl+Maiusc+A |
| Cerca nei termini | Ctrl+F |
| Cerca | F5 |
| Annulla | Esc |
| Risultato successivo / precedente nel file | F3 / Maiusc+F3 |
| File successivo / precedente | Ctrl+F3 / Ctrl+Maiusc+F3 |
| Scheda Risultati / Avvisi | Ctrl+1 / Ctrl+2 |
| Mostra o nascondi anteprima | Ctrl+3 |
| Mostra o nascondi il modulo di ricerca | F9 |

Ctrl+A, Ctrl+Maiusc+A, Esc, Ctrl+C e Invio, con il focus in un campo di testo, restano al campo. Aggiungi termine, Filtri e Informazioni non hanno scorciatoia.

## 13. Integrazione con il sistema

- Voce "Cerca con Lynkeus" (in inglese "Search with Lynkeus") nel clic destro, spenta per impostazione predefinita e accesa dal menu Strumenti > "Cerca con Lynkeus" nel clic destro (con spunta). Su Windows sta nel registro dell'utente (`HKCU\Software\Classes`, chiavi `LynkeusSearcher` sotto `*\shell`, `Directory\shell` e `Directory\Background\shell`; nessun diritto di amministratore; visibile in "Mostra altre opzioni" su Windows 11); su Linux è uno script Nautilus in `~/.local/share/nautilus/scripts`. All'avvio, con l'opzione accesa, la voce si riallinea (eseguibile spostato, lingua cambiata). La voce non si crea, e un messaggio ne spiega il motivo, se il programma o la sua cartella hanno un proprietario diverso dall'utente, dagli amministratori o dal sistema, o sono scrivibili da altri utenti, se il suo percorso ha caratteri non ammessi o è troppo lungo (riga oltre 260 caratteri), o se esiste già una voce con lo stesso nome di un altro programma, che non si tocca. Il comando per lo sfondo di una cartella passa anche la radice di un'unità (`C:\`); lo script di Nautilus, lanciato senza elementi, passa la cartella corrente. Lo stesso menu ha "Dettaglio del log".
- Con `--add <percorso>` (ripetibile; `--add -- <percorsi…>` per lo script di Linux) la finestra si apre con quei luoghi tra quelli in cui cercare. Il primo avvio con `--add` è la prima istanza e riceve i percorsi degli avvii successivi con `--add`, che escono subito (istanza unica); i percorsi sono aggiunti all'elenco senza avviare la ricerca e solo se assoluti, locali (no UNC né dispositivi) ed esistenti. Il canale è una named pipe (Windows) o un socket Unix in `$XDG_RUNTIME_DIR` (Linux), riservato all'utente e protetto da un token; se non si può creare, o la prima istanza non risponde in 2 s, l'avvio apre una finestra propria. Senza `--add` non c'è istanza unica. Un argomento non riconosciuto dà il codice d'uscita 2.
- Cartelle private: configurazione, dati, cache, log, rapporti di crash e temporanea sono create accessibili solo all'utente (Windows: solo utente e SYSTEM; Linux: 0700 e 0600). Se una cartella esiste già ed è accessibile ad altri, non si usa per la sessione (impostazioni solo in memoria, o cache, log, rapporti, istanza unica o apertura delle voci d'archivio spenti) e un messaggio avvisa l'utente.
- Log: file giornalieri in `logs` (al più 7), livello `warn` predefinito, scelto da Strumenti > Dettaglio del log (solo errori, errori e avvisi, anche le informazioni, tutto); mai testo dei documenti. Rapporti di crash: un panic scrive un file di testo in `crash-reports` (al più 10) con versione, sistema, thread, posizione e messaggio ripulito da percorsi e testi tra virgolette; niente viene inviato.

## 14. Persistenza

`settings.toml` proprio, versionato (versione 1): lingua, tema, opzioni, radici e termini correnti e recenti (10), filtri, larghezza dei pannelli e delle colonne, ordinamento, impostazioni della cache. Formato e regole di lettura in `.claude/skills/lynkeus-internals/references/protocollo-e-cache.md`. Cache su disco accesa per impostazione predefinita, con limite 500 MB (LRU), spegnibile, ridimensionabile e svuotabile dal menu Strumenti > Cache (accesa, dimensione 100, 250, 500, 1000 o 2000 MiB, svuota); il sottomenu è disabilitato, con il motivo, se non c'è una cartella per la cache. Le impostazioni stanno nella sezione `[cache]` di `settings.toml`; la voce del clic destro in `[integration]` (`context_menu`) e il livello del log in `[diagnostics]` (`log_level`).
