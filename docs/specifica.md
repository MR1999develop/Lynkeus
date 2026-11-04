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

**Documenti cercati.** Ogni documento (il file stesso o una voce d'archivio) è cercato per conto suo: OR e AND valgono per documento, e il risultato porta il file più il percorso delle voci. Il testo arriva dalla cache, se c'è una voce valida per quel file (stesso percorso, dimensione, data di modifica, versione del programma e limiti), altrimenti da un processo estrattore. Un guasto dell'estrattore (tempo scaduto, memoria superata, uscita improvvisa) dà un avviso per quel documento e la ricerca continua. Formati ancora non letti: doc, xls, ppt, RTF, EML, EPUB, 7z, tar, gz (fase 5).

## 8. Filtri

- Estensioni.
- Cartelle da saltare (predefinite: `.git`, `node_modules`, `target`, `$RECYCLE.BIN`, `System Volume Information`).
- File nascosti e di sistema: su Windows solo per attributo (`HIDDEN` o `SYSTEM`, un nome con il punto non basta); su Linux per il punto iniziale.
- Dimensione massima.
- File solo online (OneDrive; attributi `RECALL_ON_DATA_ACCESS`, `RECALL_ON_OPEN`, `OFFLINE`): saltati per impostazione predefinita, anche se sono una radice scelta dall'utente; una casella li include. Gli attributi si leggono dall'elenco della cartella, senza aprire il file e quindi senza scaricarlo.
- Una radice (cartella o file) scelta dall'utente non è mai esclusa per nome, estensione o attributi nascosto e di sistema; dimensione massima (predefinita 200 MiB) e stato solo online valgono anche per lei.
- Radici duplicate (stesso percorso, anche scritto in modo diverso o con altre maiuscole su Windows; il confronto è sul testo, senza risolvere i link) sono una sola, la prima. Una radice dentro un'altra resta una radice a sé: l'esterna salta quel ramo e l'interna lo visita con le sue regole. Ogni file si legge una volta sola.
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

- Doppio clic o Invio apre il file con il programma predefinito del sistema; una voce di testo dentro un archivio è prima ricomposta in un file `.txt` di una cartella temporanea privata dell'app, svuotata all'avvio e all'uscita; per una voce che è un documento (PDF, Office) servono i byte originali, che il processo estrattore non restituisce ancora, e un messaggio avvisa che non è disponibile. I file che il sistema eseguirebbe (programmi, script, collegamenti: `exe`, `bat`, `ps1`, `lnk`, `sh`, `py`, …) non si aprono: resta "Mostra nella cartella".
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

- Voce "Cerca con Lynkeus" nel clic destro, attivabile dalle impostazioni: su Windows nel registro dell'utente (visibile in "Mostra altre opzioni" su Windows 11), su Linux come script Nautilus.
- Con `--add` un'unica finestra riceve i percorsi di più elementi selezionati (istanza unica); i percorsi sono aggiunti all'elenco senza avviare la ricerca.

## 14. Persistenza

`settings.toml` proprio, versionato (versione 1): lingua, tema, opzioni, radici e termini correnti e recenti (10), filtri, larghezza dei pannelli e delle colonne, ordinamento, impostazioni della cache. Formato e regole di lettura in `.claude/skills/lynkeus-internals/references/protocollo-e-cache.md`. Cache su disco accesa per impostazione predefinita, con limite 500 MB (LRU), spegnibile, ridimensionabile e svuotabile dal menu Strumenti > Cache (accesa, dimensione 100, 250, 500, 1000 o 2000 MiB, svuota); il sottomenu è disabilitato, con il motivo, se non c'è una cartella per la cache. Le impostazioni stanno nella sezione `[cache]` di `settings.toml`.
