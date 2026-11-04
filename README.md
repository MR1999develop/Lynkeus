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

- **Apri**: doppio clic su una riga o Invio apre il file con il programma predefinito del sistema. I programmi e gli script (per esempio `.exe`, `.bat`, `.ps1`, `.sh`) non vengono mai aperti da qui: si può solo mostrarli nella cartella. **Mostra nella cartella**: Ctrl+Maiusc+R o clic destro.
- **Anteprima**: sotto la tabella, mostra le righe intorno al risultato con le corrispondenze evidenziate; per i documenti e gli archivi mostra per ora solo il contesto del risultato. Ctrl+3 la mostra o la nasconde; l'altezza si può cambiare e viene ricordata.
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

## Limiti noti

- Una frase spezzata da un a capo (frequente nei PDF) non viene trovata.
- I PDF scansionati non hanno testo: servirebbe l'OCR, che non c'è. Il file finisce negli avvisi.
- Dei file .doc e .ppt si estrae il testo principale; commenti, note e caselle di testo potrebbero mancare.
- I file .rar non sono supportati.
- I file protetti da password o danneggiati, e quelli che superano i limiti di tempo o di dimensione (anche degli archivi), non vengono letti: compaiono nella scheda Avvisi con il loro motivo.
- Il numero di risultati è limitato (200.000 per impostazione predefinita); al limite la ricerca si ferma e un avviso invita a restringerla.
- I file presenti solo online (per esempio in OneDrive) sono saltati per non scaricarli, salvo diversa scelta.

## Cache e privacy

Per non rileggere i documenti a ogni ricerca, il programma conserva su disco il **testo estratto** da documenti e archivi. La cache contiene quindi frammenti del contenuto dei tuoi file: sta in una cartella privata dell'utente. Dalle Impostazioni si può spegnere la cache e svuotarla in qualsiasi momento. Il programma non invia dati in rete.

## Firma dell'eseguibile e verifica

L'eseguibile **non è firmato** con un certificato di firma del codice (è un progetto personale). Su Windows, SmartScreen può quindi mostrare l'avviso "Windows ha protetto il PC": per avviare il programma si sceglie "Ulteriori informazioni" e poi "Esegui comunque", dopo aver verificato l'integrità del file.

Insieme allo zip si pubblica il file `SHA256SUMS`. Per verificare lo zip:

- Windows (PowerShell): `Get-FileHash .\<file>.zip -Algorithm SHA256`
- Linux: `sha256sum -c SHA256SUMS`

L'hash calcolato deve coincidere con quello scritto in `SHA256SUMS`.

## Licenza

GPLv3 (`GPL-3.0-only`), testo completo nel file `LICENSE`.

Copyright (C) 2026 MR
