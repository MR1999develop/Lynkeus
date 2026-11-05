# Lynkeus Searcher

Lynkeus Searcher è un'app desktop che cerca una porzione di testo in file singoli o in intere cartelle (sottocartelle comprese) e restituisce **le parole che la contengono**, con file, posizione e contesto.

Esempio: cercando `ar` si ottiene "aragosta" nel file X, "carta" nel file Y, "chitarra" nel file Z.

**Stato: in sviluppo.** Non esiste ancora una versione pubblicata.

## Cosa fa

- Parte da un elenco misto di cartelle (lette ricorsivamente) e file, anche trascinati nella finestra.
- Accetta più termini, uno per riga, cercati così come sono, spazi compresi. Un interruttore sceglie tra "qualsiasi termine" (OR) e "tutti i termini nello stesso file" (AND).
- Caratteri jolly `*` e `?`; le maiuscole sono ignorate (si può distinguerle); gli accenti sono distinti (si possono ignorare).
- Mostra una tabella Parola | File | Posizione | Contesto, con la corrispondenza evidenziata. Dalla tabella si apre il file, lo si mostra nella cartella, si vede l'anteprima, si esportano i risultati in CSV o TXT e si copiano le righe.
- Interfaccia in italiano e in inglese.
- Filtri su estensioni, cartelle da saltare, file nascosti e di sistema, dimensione massima.

## Uso dei risultati

- **Apri**: doppio clic su una riga o Invio apre il file con il programma predefinito del sistema. Funziona anche per le voci dentro un archivio, di qualsiasi formato (per esempio un PDF dentro uno zip): il programma le estrae in una cartella temporanea privata, svuotata all'avvio e all'uscita, e le apre da lì. Per un capitolo di un EPUB si apre il libro. Se più voci di un archivio hanno lo stesso nome, compaiono come "nome (2).ext", "nome (3).ext". I programmi e gli script (per esempio `.exe`, `.bat`, `.cmd`, `.ps1`, `.vbs`, `.js`, `.lnk`, `.msi`, `.jar`, `.sh`, `.py`; l'elenco completo è largo di proposito) non vengono mai aperti da qui: si può solo mostrarli nella cartella. **Mostra nella cartella**: Ctrl+Maiusc+R o clic destro.
- **Anteprima**: sotto la tabella, mostra le righe intorno al risultato con le corrispondenze evidenziate; per i documenti e gli archivi mostra le unità di testo vicine (paragrafi, celle, righe di pagina) fino a 20 prima e 20 dopo il risultato. Ctrl+3 la mostra o la nasconde; l'altezza si può cambiare e viene ricordata.
- **Esporta**: Ctrl+E salva le righe mostrate o solo quelle selezionate in CSV (UTF-8; separatore `;` in italiano, `,` in inglese) o in TXT tabulato. Le celle che inizierebbero come una formula (`=`, `+`, `-`, `@`) ricevono un apostrofo davanti, così il foglio di calcolo non le esegue.
- **Copia**: Ctrl+C copia le righe selezionate come testo con colonne separate da tabulazioni, pronto da incollare in un foglio di calcolo.

## Requisiti

- Windows 10 o 11, oppure Ubuntu.
- Per compilare: la toolchain Rust indicata in `rust-toolchain.toml`.

## Compilare ed eseguire

```
cargo run -p lynkeus-app --release
```

Controlli del progetto (formattazione, clippy, test, `cargo deny`):

```
cargo xtask check
```

## Formati supportati

Testo e codice (con rilevamento della codifica), docx, xlsx, pptx, odt, ods, odp, PDF, doc, xls, ppt, RTF, EML, EPUB, e gli archivi zip, 7z, tar e gz, anche annidati.

Sono letti anche i file `.tgz`/`.tar.gz`. Le e-mail (EML) sono lette con i loro allegati, che sono cercati come documenti a sé. Un `.doc` che è in realtà RTF e un `.xls` che è una pagina HTML o un CSV si leggono lo stesso.

## Limiti noti

- Una frase spezzata da un a capo (frequente nei PDF) non viene trovata.
- I PDF scansionati non hanno testo: servirebbe l'OCR, che non c'è. Il file finisce negli avvisi.
- Dei file .doc si estrae il testo principale: note, intestazioni e piè di pagina, commenti e caselle di testo non si leggono; Word 6 e 95 non sono supportati (avviso "danneggiato").
- Dei file .ppt si estrae il testo delle diapositive: note del relatore, schemi, commenti e oggetti incorporati (grafici, tabelle) non si leggono.
- I file .rar non sono supportati.
- Una voce di archivio molto grande (oltre il limite per voce, 512 MiB predefiniti) non si legge né si apre.
- I file protetti da password o danneggiati, e quelli che superano i limiti di tempo o di dimensione (anche degli archivi), non vengono letti: compaiono nella scheda Avvisi con il loro motivo.
- Il numero di risultati è limitato (200.000 per impostazione predefinita); al limite la ricerca si ferma e un avviso invita a restringerla.
- I file presenti solo online (per esempio in OneDrive) sono saltati per non scaricarli, salvo diversa scelta.

## Cache e privacy

Per non rileggere i documenti a ogni ricerca, il programma conserva su disco il **testo estratto** da documenti e archivi. La cache contiene quindi frammenti del contenuto dei tuoi file: sta in una cartella privata dell'utente. Dal menu Strumenti > Cache si può spegnerla, scegliere la dimensione massima (predefinita 500 MB, le voci meno usate si cancellano per prime) e svuotarla in qualsiasi momento. Il programma non invia dati in rete.

I documenti (PDF, Office, archivi) sono letti da processi separati con memoria e tempo limitati: un file danneggiato o ostile non blocca il programma, ma compare negli avvisi.

## Clic destro (facoltativo)

Dal menu Strumenti si può accendere "Cerca con Lynkeus" nel clic destro su file e cartelle (spento per impostazione predefinita). La voce apre il programma con l'elemento scelto tra i luoghi in cui cercare; con più elementi selezionati si usa un'unica finestra. Su Windows 11 compare in "Mostra altre opzioni". Non servono diritti di amministratore. Il programma non crea la voce se la sua cartella è modificabile da altri utenti o se esiste già una voce con lo stesso nome di un altro programma, e lo dice con un messaggio.

La voce si toglie spegnendo l'opzione. Se hai cancellato il programma senza spegnerla, togli a mano le voci rimaste:

- Windows (Editor del Registro di sistema, solo sotto `HKEY_CURRENT_USER`): elimina le tre chiavi `LynkeusSearcher` in `Software\Classes\*\shell`, `Software\Classes\Directory\shell` e `Software\Classes\Directory\Background\shell`. In PowerShell: `Remove-Item -Recurse 'HKCU:\Software\Classes\Directory\shell\LynkeusSearcher'` e lo stesso per le altre due (per `*` usa `-LiteralPath 'HKCU:\Software\Classes\*\shell\LynkeusSearcher'`).
- Linux (Nautilus): elimina da `~/.local/share/nautilus/scripts` il file "Cerca con Lynkeus" o "Search with Lynkeus".

La voce segue la lingua del programma e si aggiorna al riavvio.

## Log e rapporti di crash

Il programma scrive un log (un file al giorno, gli ultimi 7) e, se si blocca per un errore interno, un rapporto di crash (gli ultimi 10). Il log è nella cartella `logs` e i rapporti in `crash-reports`, dentro la cartella dati locale dell'utente (Windows: `%LOCALAPPDATA%\Lynkeus Searcher\data`; Linux: `~/.local/share/lynkeus-searcher`). Il livello del log si cambia da Strumenti > Dettaglio del log (predefinito: errori e avvisi). Log e rapporti contengono motivi, percorsi e contatori, non il testo dei tuoi documenti; non vengono inviati da nessuna parte: se segnali un problema puoi allegarli, dopo averli riletti.

## Cartelle private

Impostazioni, cache, log, rapporti e file temporanei stanno in cartelle accessibili solo al tuo utente (su Windows anche a SYSTEM). Se una di queste cartelle esiste già ed è aperta ad altri utenti, per sicurezza il programma non la usa in quella sessione (cache, log o impostazioni potrebbero non essere salvati) e te lo segnala: controlla i permessi della cartella.

## Firma dell'eseguibile e verifica

L'eseguibile **non è firmato** con un certificato di firma del codice (è un progetto personale). Su Windows, SmartScreen può quindi mostrare l'avviso "Windows ha protetto il PC": per avviare il programma si sceglie "Ulteriori informazioni" e poi "Esegui comunque", dopo aver verificato l'integrità del file.

Insieme allo zip si pubblica il file `SHA256SUMS`. Per verificare lo zip:

- Windows (PowerShell): `Get-FileHash .\<file>.zip -Algorithm SHA256`
- Linux: `sha256sum -c SHA256SUMS`

L'hash calcolato deve coincidere con quello scritto in `SHA256SUMS`.

## Licenza

GPLv3 (`GPL-3.0-only`), testo completo nel file `LICENSE`.

Copyright (C) 2026 MR
