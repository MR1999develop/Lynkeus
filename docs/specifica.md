# Specifica di Lynkeus Searcher

Requisiti decisi con l'utente e semantica esatta della ricerca. La fonte delle decisioni è `PLAN.md`.

## 1. Scopo

Cercare una porzione di testo in file singoli o in cartelle (con sottocartelle) e restituire **le parole che la contengono**, con file, posizione e contesto. Esempio: `ar` → "aragosta" in X, "carta" in Y, "chitarra" in Z.

## 2. Requisiti

- Rust + egui 0.37, Windows 10/11 e Ubuntu, eseguibile portatile.
- Interfaccia in italiano e inglese; licenza GPLv3 (`GPL-3.0-only`).
- Partenza: elenco misto di cartelle (ricorsive) e file, anche trascinati nella finestra.
- Lettura diretta a ogni ricerca, con cache del testo estratto da documenti e archivi.

## 3. Termini

- Una riga per termine, aggiunta con il pulsante +.
- Ogni termine è cercato così com'è, spazi compresi.
- Un termine vuoto, o fatto solo di `*` e `?`, è un errore mostrato sotto il campo.
- Con i jolly spenti `*` e `?` sono caratteri letterali.

## 4. Regole di confronto

- **Jolly:** `*` = zero o più caratteri non spazio; `?` = un carattere non spazio.
- **Spazi:** uno spazio nel termine corrisponde a uno o più spazi orizzontali; nessuna corrispondenza attraversa un a capo (una frase spezzata da un a capo non si trova).
- **Maiuscole:** ignorate per impostazione predefinita; una casella le distingue.
- **Accenti:** distinti per impostazione predefinita; una casella li ignora.
  - Accenti distinti: una lettera accentata nel termine accetta anche la sua forma decomposta (NFD) nel testo, quindi il testo non viene normalizzato e le posizioni restano quelle originali. Il termine è prima portato in NFC.
  - Accenti ignorati: ogni lettera accetta le sue varianti accentate e gli eventuali segni combinanti.
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

## 8. Filtri

- Estensioni.
- Cartelle da saltare (predefinite: `.git`, `node_modules`, `target`, `$RECYCLE.BIN`, `System Volume Information`).
- File nascosti e di sistema (attributi Windows o punto iniziale).
- Dimensione massima.
- File solo online (OneDrive): saltati per impostazione predefinita, una casella li include.
- Link simbolici e junction non seguiti; i filtri gitignore non si applicano.

## 9. Limiti

Vedi `.claude/skills/lynkeus-internals/references/limiti.md`: risultati (200.000), archivi (profondità 5, 100.000 voci, 512 MB per voce, 4 GB per archivio, rapporto 200:1), timeout 60 s per file, cache 500 MB. I limiti sugli archivi sono modificabili in "Avanzate".

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

- Doppio clic o Invio apre il file; una voce dentro un archivio è prima estratta in una cartella temporanea dell'app, cancellata all'uscita.
- "Mostra nella cartella".
- Anteprima: unità di testo con ±20 unità vicine, corrispondenze evidenziate, precedente/successiva.
- Esporta CSV (UTF-8 con BOM; `;` in italiano, `,` in inglese) o TXT tabulato.
- Ctrl+C copia le righe selezionate come TSV.

## 13. Integrazione con il sistema

- Voce "Cerca con Lynkeus" nel clic destro, attivabile dalle impostazioni: su Windows nel registro dell'utente (visibile in "Mostra altre opzioni" su Windows 11), su Linux come script Nautilus.
- Con `--add` un'unica finestra riceve i percorsi di più elementi selezionati (istanza unica); i percorsi sono aggiunti all'elenco senza avviare la ricerca.

## 14. Persistenza

`settings.toml` proprio, versionato: radici e termini recenti, opzioni, filtri, larghezza dei pannelli, ordinamento, lingua, tema, impostazioni della cache. Cache su disco spegnibile e svuotabile dalle Impostazioni.
