# Fixture di prova

Tutto si rigenera con `python -I fixtures/src/genera_demo.py` (richiede LibreOffice in
`C:\Program Files\LibreOffice\program\soffice.exe`). Sorgenti dei documenti: `fixtures/src/demo/`.
Nessun dato personale nei contenuti e nei metadati.

Convenzione: "parola" = testo tra due spazi senza punteggiatura ai bordi (l'apostrofo interno resta
nella parola). Le parole attese per la ricerca di "ar" (senza distinguere maiuscole e accenti) sono
elencate per ogni file; qualsiasi altra parola NON deve comparire nei risultati.

## demo/

| File | Produzione | Codifica | Parole con "ar" |
|---|---|---|---|
| `ricette/mare.txt` | script Python, 15 righe | UTF-8 senza BOM, LF | riga 5: `calamari`; riga 12: `l'aragosta` (la riga è "Cuocere l'aragosta per 10 minuti.") |
| `ufficio/note.docx` | `src/demo/note.fodt` convertito con LibreOffice, poi ripulito dallo script | DOCX (OOXML) | paragrafo 3: `Comprare`, `carta` (frase "Comprare la carta per la stampante."). Paragrafi: 1 titolo, 2, 3, 4; gli altri non hanno "ar" |
| `musica/archivio.zip` | ZIP con `elenco.pdf` (da `src/demo/elenco.fodt` via LibreOffice, testo vero, 1 pagina); data della voce zip fissa | ZIP/PDF | `elenco.pdf` pagina 1: `Suonare`, `chitarra` (frase "Suonare la chitarra classica.") |
| `viaggi/citta.txt` | script Python, 4 righe | UTF-8 senza BOM, LF | riga 1: `d'arte` ("Firenze è una città d'arte."); riga 3: `ARMADIO` ("ARMADIO grande"). Riga 2: "Perché no?" (nessuna). Riga 4: "La città di notte è bella." con `città` in forma NFD (`a` + U+0300), nessuna parola con "ar" |
| `legacy/vecchio-1252.txt` | script Python | Windows-1252 (é=E9, à=E0), LF | riga 2: `carità` ("Un atto di carità."); riga 1 "Fatelo perché è giusto." (nessuna) |
| `legacy/utf16.txt` | script Python | UTF-16 LE con BOM (FF FE), CRLF | riga 1: `carta` ("Una carta d'identità") |
| `binario/immagine.png` | PNG 8x8 RGB generato con `zlib`/`struct` | binario | nessuna: da saltare |

## Metadati

- `note.docx`: `docProps/core.xml` con creator e lastModifiedBy vuoti, date fisse 2026-01-01; `app.xml`
  senza Application/Company. Date delle voci zip fisse.
- `elenco.pdf`: Creator "Writer", Producer "LibreOffice"; nessun Author/Title. La CreationDate è
  quella della generazione (il PDF, e quindi lo zip, non è byte per byte deterministico).

## formati/ e malevoli/

Si rigenerano con `python -I fixtures/src/genera_formati.py` (LibreOffice in
`C:\Program Files\LibreOffice\program\soffice.exe`, 7-Zip in `C:\Program Files\7-Zip\7z.exe`).
Sorgenti in `src/formati/` (`.fodt`, `.fods`, `.fodp`, scritti dallo script) e `src/cifra_docx.py`
(cifratura via UNO con il python di LibreOffice). Ogni file di `formati/` contiene `aragosta` in una
posizione nota e almeno una parola accentata (`città`, `perché`). Metadati: autore vuoto, date fisse
2026-01-01 nei formati OOXML/ODF/zip; nei formati binari 97-2003 e nei PDF restano i metadati scritti
da LibreOffice, senza autore. Le date interne dei PDF possono variare tra le generazioni.
"Paragrafo N" conta i paragrafi non vuoti del documento.

### formati/

| File | Produzione | Posizione di `aragosta` (e `città`) |
|---|---|---|
| `relazione.docx` | `relazione.fodt` -> LibreOffice, metadati ripuliti | paragrafo 4 ("Per cena c'è l'aragosta con il limone."); `città` paragrafo 2 |
| `appunti.odt` | `appunti.fodt` -> odt | paragrafo 2; `città` e `Perché` paragrafo 3 |
| `vecchia-lettera.doc` | `vecchia-lettera.fodt` -> doc (Word 97-2003) | paragrafo 3; `città` paragrafo 2 |
| `catalogo.pdf` | `catalogo.fodt` con interruzione di pagina -> pdf, 2 pagine, testo vero | pagina 2, riga 3 ("Oggi abbiamo l'aragosta fresca."); `città` pagina 2 riga 2 |
| `spesa.xlsx` | `spesa.fods` -> xlsx | foglio `Spesa`, cella B7; `città` in C2 |
| `bilancio.ods` | `bilancio.fods` -> ods | foglio `Conti`, cella C4; `città` in C3 |
| `vecchia-spesa.xls` | `vecchia-spesa.fods` -> xls (Excel 97-2003) | foglio `Spesa`, cella B7; `città` in C2 |
| `presentazione.pptx` | `presentazione.fodp` -> pptx | diapositiva 2 ("Stasera aragosta e vino."); `città` diapositiva 3 |
| `lezione.odp` | `lezione.fodp` -> odp | diapositiva 2; `città` diapositiva 1 |
| `vecchia-presentazione.ppt` | `vecchia-presentazione.fodp` -> ppt (PowerPoint 97-2003) | diapositiva 3; `città` diapositiva 2 |
| `lettera.rtf` | byte scritti dallo script, ASCII | riga/paragrafo 3 (`aragosta`); paragrafo 2: `città` scritta `citt\'e0` e `è`/`perché` scritti con `\u232?`/`\u233?` |
| `in-realta-rtf.doc` | stesso RTF con estensione `.doc` | paragrafo 3; atteso: letto come RTF (contenuto diverso dall'estensione, caso innocuo) |
| `ordine.eml` | modulo `email` di Python, data fissa | oggetto (`Ordine di aragosta`); corpo testo (riga 2); allegato `allegato.docx` paragrafo 2; allegato `nota.txt` riga 1. `città` in corpo e allegati |
| `saluti.eml` | idem | solo corpo (riga 2); `città` nel corpo; oggetto senza la parola |
| `libro.epub` | zipfile, EPUB 3 minimo (`mimetype` non compresso per primo) | `OEBPS/cap2.xhtml` (capitolo 2, paragrafo 2: il titolo `h1` è il paragrafo 1, come i titoli di docx e odt); `città` paragrafo 2 e `Perché` paragrafo 3 in `OEBPS/cap1.xhtml` |
| `spesa.txt` | script Python, UTF-8 | riga 6 ("Oggi compriamo l'aragosta in città.") |
| `appunti.txt.gz` | gzip, nome interno vuoto, mtime 0 | riga 2 del testo decompresso (`aragosta`, `città`) |
| `archivio.tar` | tarfile, mtime fisso | voce `cartella/ricetta.txt` riga 2 (`aragosta`, `città`); `cartella/nota.txt` senza la parola |
| `archivio.tar.gz` | stesso tar compresso con gzip | come `archivio.tar` |
| `archivio.7z` | 7-Zip (`7z a -t7z`), senza date | come `archivio.tar` (percorsi con `/`; 7-Zip mette `nota.txt` prima di `ricetta.txt`) |
| `annidato.zip` | zipfile, 3 zip annidati | `annidato.zip` > `livello2.zip` > `livello3.zip` > `interno.docx`, paragrafo 2 (`aragosta`, `città`); profondità 3, entro il limite 5 |
| `protetto-password.docx` | `protetto.fodt` cifrato con `cifra_docx.py` (password `segreto`, contenitore OOXML cifrato) | atteso: avviso "protetto da password", nessun testo (contenuto nascosto: paragrafo 1) |
| `protetto-password.pdf` | `catalogo.fodt` esportato con `python -I fixtures/src/genera_pdf_protetto.py` (password d'apertura `segreto`, RC4 128 bit); date e ID del PDF cambiano a ogni generazione | atteso: avviso "protetto da password", nessun testo |

### malevoli/

| File | Produzione | Comportamento atteso |
|---|---|---|
| `bomba.zip` | 5 MiB di zeri in una voce (circa 5 KB su disco, rapporto circa 1000:1) | avviso "limite superato" (rapporto > 200:1), nessun blocco né esaurimento di memoria |
| `percorsi.zip` | voci `../../evil.txt`, `/assoluto/evil.txt`, `C:/Windows/evil.txt`, `ok/normale.txt`, ognuna con `aragosta` nella prima riga | nomi usati solo come etichette: nessun file scritto su disco; le voci si possono leggere e trovare (o avvisare), mai creare file fuori dalla cartella di lavoro |
| `annidato-profondo.zip` | 7 zip annidati (`livello2.zip` ... `livello7.zip`), `fondo.txt` in fondo con `aragosta` | avviso "limite superato" (profondità 5), `fondo.txt` non raggiunto, nessun blocco |
| `molte-voci.zip` | non generato di default (100.001 voci arrivano a circa 9 MB, oltre 5 MB). `python -I fixtures/src/genera_formati.py --vuote 100001` lo crea | avviso "limite superato" (100.000 voci), nessun blocco |
| `troncato.pdf` | prima metà di `catalogo.pdf` | avviso "danneggiato", nessun blocco |
| `entita.docx` | `relazione.docx` con `<!DOCTYPE>` e entità annidate (`lol3` = 1000 volte `aragosta `) nel primo `w:t` di `word/document.xml` | DTD rifiutato: avviso "danneggiato"/non valido, nessuna espansione delle entità, nessun blocco |
| `immagine-finta.pdf` | PNG 8x8 copiato da `demo/binario/immagine.png` con estensione `.pdf` | avviso "contenuto diverso dall'estensione", nessun blocco |
| `riga-lunga.txt` | script Python: riga 1 di 2.000.000 caratteri senza spazi (`abcdefghij` ripetuto), riga 2 `aragosta` | nessun blocco, memoria limitata: `aragosta` trovata a riga 2; la riga 1 non contiene parole con `ar` |

I file seguenti si rigenerano con `python -I fixtures/src/genera_malevoli.py` (solo libreria standard, byte per byte uguali; `dizionario-enorme.7z` richiede 7-Zip in `C:\Program Files\7-Zip\7z.exe`).

| File | Produzione | Comportamento atteso |
|---|---|---|
| `sparso.xlsx` | foglio `Sparso` con due sole celle: A1 `aragosta` e XFD1048576 `città lontana` (stringhe condivise), dimensione dichiarata `A1:XFD1048576` | le due celle lette a flusso, nessun avviso; mai il rettangolo intero in memoria |
| `sparso.zip` | `sparso.xlsx` seguito da `nota.txt` (`aragosta in nota`) | il processo estrattore non termina, `nota.txt` letta |
| `slide-ripetute.pptx` | presentazione con 10.000 `p:sldId` verso la stessa diapositiva (`Stasera aragosta`) | una sola diapositiva letta (diapositiva 1) |
| `dizionario-enorme.7z` | 7z di 7-Zip (LZMA2, intestazione non compressa) con `nota.txt` (`aragosta nel dizionario enorme`), dizionario dichiarato riscritto a 2 GiB e CRC ricalcolati | voce letta; il dizionario non si alloca in anticipo |
