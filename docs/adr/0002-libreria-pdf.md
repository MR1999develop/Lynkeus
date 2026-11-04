# 0002. Libreria PDF

Stato: accettata

## Contesto

Il processo estrattore deve leggere il testo dei PDF pagina per pagina, con il numero di pagina e di
riga (`PageLine`), su file non fidati. Il piano indicava `pdf_oxide`, con `pdf-extract` come ripiego,
da decidere con un prototipo. Vincoli: licenza compatibile con GPLv3, niente codice C, niente panic
o consumo di memoria senza limite sui file ostili (il processo è comunque isolato in un Job Object),
ricerca per parola (una parola spezzata o incollata è un risultato perso).

Prototipo: `experiments/pdf/` (si butta dopo questa decisione). Comandi:

- `pdf-experiment gen <cartella>` scrive PDF sintetici ostili (vuoto, intestazione con spazzatura,
  ciclo nel page tree, `/Count` falso, annidamento di 200.000 array, bomba Flate da 1 GiB in 1 MB);
- `pdf-experiment run <file>...` estrae ogni file con ogni variante in un processo separato, con
  timeout di 60 s, e misura tempo, memoria impegnata di picco e working set di picco;
- `jobobject-experiment wrap <MiB> <programma> ...` (da `experiments/jobobject/`) ripete l'estrazione
  dentro un Job Object con limite di memoria per processo.

Varianti confrontate:

| Nome | Chiamata |
|---|---|
| `oxide` | `pdf_oxide` 0.3.78, `PdfDocument::from_bytes`, `page_count`, `extract_text(i)` |
| `oxide-lines` | come sopra, `extract_text_lines(i)` e `TextLine::text` per riga |
| `oxide-plain` | come sopra, `to_plain_text(i, &ConversionOptions::default())` |
| `extract` | `pdf-extract` 0.12.1, `extract_text_from_mem_by_pages` |

File: le fixture (`formati/catalogo.pdf`, `elenco.pdf` estratto da `demo/musica/archivio.zip`,
`malevoli/troncato.pdf`, `malevoli/immagine-finta.pdf`), i sintetici ostili, due varianti di
`catalogo.fodt` esportate da LibreOffice in una cartella temporanea (con password d'apertura e senza
PDF "tagged"), e cinque PDF già presenti nel sistema, letti sul posto e non copiati: un'immagine
vettoriale di Firefox (1 pagina, nessun testo), la licenza di NVIDIA Nsight (6 pagine, Word, tagged),
il manuale di VirtualBox (1.200 pagine, 10 MB) e due documenti tecnici di VeraCrypt (6 e 7 pagine).

Macchina: Ryzen 5 7600, 32 GB, Windows 10 22H2, build `release`, Rust 1.99.1. Tempi di una sola
esecuzione (sul manuale due esecuzioni differiscono fino al 40%).

## Dati

### Testo corretto (accenti, ordine, a capo)

| File | `oxide` / `oxide-plain` | `oxide-lines` | `extract` |
|---|---|---|---|
| `catalogo.pdf` (LibreOffice, tagged) | **paragrafi incollati in una riga**: "pane e latte.Fine della prima pagina"; `città` corretta | 7 righe corrette; `aragosta` pagina 2 riga 3, `città` pagina 2 riga 2, come nel README delle fixture | 7 righe corrette, più due righe vuote in testa a ogni pagina |
| `elenco.pdf` | "Elenco dei brani in programma Suonare la chitarra classica.Cantiamo..." | 3 righe; `chitarra` pagina 1 riga 2 | 3 righe, più righe vuote |
| `catalogo.pdf` senza tag | 7 righe corrette | 7 righe corrette | 7 righe corrette |
| licenza NVIDIA (Word, tagged) | righe corrette, 0 frammenti | **23 frammenti** su 5.101 parole: "y ou", "aca demic", "D ocumentation", "arisi ng" | 6 frammenti: "AFFILIATE S", "NON -INFRINGE ME NT" |
| manuale VirtualBox (non tagged) | `oxide`: 64.271 righe, 92 parole solo sue (soprattutto "////"); `oxide-plain`: paragrafi interi su una riga (15.542 righe) | 65.121 righe; spazi in più dentro la sintassi dei comandi (`<##provider= name>`), 3.210 token solo suoi | 64.448 righe; 830 token solo suoi |
| VeraCrypt (2 file) | identico agli altri | identico | identico |
| accenti (`città`, `è`, `perché`) | corretti | corretti | corretti |
| legature, U+FFFD | nessuna nei file provati | nessuna | nessuna |

"Frammento" = parola che compare solo in quella variante (confronto tra le quattro). Il manuale
contiene molta sintassi di comandi, dove le varianti differiscono solo nella spaziatura attorno a
`<`, `=` e `>`; nella prosa coincidono.

### Numero di pagina

Tutte e quattro le varianti danno il testo per pagina. `pdf_oxide` apre il documento una volta e
estrae una pagina alla volta (`page_count` più `extract_text_lines(i)`): si può fermare tra una pagina
e l'altra e mandare i risultati man mano. L'API pubblica di `pdf-extract` estrae tutte le pagine in una
chiamata (`Vec<String>`); per andare pagina per pagina bisognerebbe usare `lopdf` e `output_doc_page`.

### File troncati, falsi e ostili

| File | `pdf_oxide` (tutte le varianti) | `pdf-extract` |
|---|---|---|
| `troncato.pdf` | errore "Invalid cross-reference table", 1,5 ms | errore "failed parsing cross reference table", 0,04 ms |
| `immagine-finta.pdf` (PNG) | errore "Invalid cross-reference table" | errore "invalid file header" |
| file vuoto | errore "Invalid PDF header ... File is empty" | errore "invalid file header" |
| intestazione + 4 KB di spazzatura | errore xref | errore xref |
| ciclo nel page tree | pagina 1 letta, pagina 2 con errore "Page index 1 not found" | **0 pagine, nessun errore** |
| `/Count 2147483647` con una pagina | 1 pagina, corretto | 1 pagina, corretto |
| 200.000 array annidati nel contenuto | testo letto (`aragosta`) | **panic** (`unwrap` su `InvalidContentStream`), intercettato da `catch_unwind` |
| bomba Flate (1 GiB di spazi in 1 MB) | nessun errore, **testo perso** (0 caratteri), 776 MiB impegnati, 0,3–0,5 s | testo letto, **3.081 MiB impegnati**, 1,1 s |
| bomba nel job da 256 MiB | processo terminato (0xC0000409) a 198 MiB, 122 ms | processo terminato (0xC0000409) a 194 MiB, 120 ms |
| PDF con password d'apertura | **testo vuoto senza errore**; `is_encrypted()` = vero e `authenticate(b"")` = falso lo distinguono | errore "the supplied password is incorrect" |

Nessun blocco e nessun timeout in nessun caso.

### Tempo e memoria

| File | `oxide-lines` | `oxide` | `extract` |
|---|---|---|---|
| `catalogo.pdf` (2 pagine) | 1,9–2,1 ms; 1,5 MiB impegnati | 2,0 ms | 1,6–1,8 ms; 1,1 MiB |
| licenza NVIDIA (6 pagine) | 62 ms; 18 MiB | 61 ms; 18 MiB | 39 ms; 10 MiB |
| manuale VirtualBox (1.200 pagine, 10 MB) | 3,2–3,4 s; 65 MiB | 2,1 s; 68 MiB | 1,6–2,9 s; 71 MiB |
| VeraCrypt (6 pagine) | 16 ms; 7 MiB | 13 ms; 6 MiB | 14 ms; 3 MiB |

`pdf_oxide` costa circa 2,7 ms per pagina sul manuale: un PDF arriva al timeout di 60 s solo oltre
circa 20.000 pagine. `PdfDocument::from_bytes` copia i byte del file (`data.clone()`), quindi tiene
in memoria due volte il file; `open(path)` passa da `from_bytes`.

### Licenza, codice C, `unsafe`, dipendenze

| | `pdf_oxide` 0.3.78 | `pdf-extract` 0.12.1 |
|---|---|---|
| Licenza | MIT OR Apache-2.0 | MIT |
| `cargo deny` con il `deny.toml` del progetto | licenze e ban ok | licenze e ban ok |
| Codice C | nessuno (`flate2` con `zlib-rs`, `brotli` in Rust) | nessuno (`lopdf`, `flate2` con `zlib-rs`) |
| `unsafe` | nessuno nei moduli di parsing ed estrazione (molti con `#![forbid(unsafe_code)]`, ma non il crate); `src/ffi.rs` con 458 funzioni `#[no_mangle] extern "C"` sempre compilato fuori da wasm, non esportate dall'eseguibile (nomi assenti dal binario); `transmute` solo nel rendering, feature non attiva | nessuno; `lopdf` ha `#![forbid(unsafe_code)]` |
| Crate (normali e di build, Windows) | 144 senza feature predefinite, 145 con: tra gli altri `office_oxide`, `zip`, `quick-xml`, `image`, `tiff`, `png`, `taffy`, `rayon`, `regex`, `chrono`, `env_logger` | 65 |
| Advisory | RUSTSEC-2026-0192 (`ttf-parser` non mantenuto) | lo stesso, tramite `lopdf` |
| Rilasci | molto frequenti (0.3.78) | rari |

Feature predefinite di `pdf_oxide`: `icc` (profili colore, inutile per il testo) e `legacy-crypto`
(MD5 e RC4, necessari per aprire i PDF cifrati con password utente vuota, frequenti).

## Opzioni

1. **`pdf_oxide` con `extract_text`** (l'API consigliata dalla libreria). Contro: sui PDF tagged di
   LibreOffice, cioè la maggior parte dei documenti italiani prodotti con software libero, segue
   l'albero di struttura e incolla i paragrafi senza separatore: righe e parole sbagliate,
   fixture fallite.
2. **`pdf_oxide` con `to_plain_text`.** Contro: stesso incollamento sui PDF tagged e paragrafi su una
   sola riga anche sugli altri: il numero di riga perde senso.
3. **`pdf_oxide` con `extract_text_lines`.** Contro: in qualche testo giustificato spezza parole
   (lo 0,45% delle parole della licenza NVIDIA, 0 negli altri file); spazi prima della punteggiatura
   ("page 41 ."); il più lento (2,7 ms per pagina); molte dipendenze; perde in silenzio il testo di
   una bomba di decompressione dopo 776 MiB; i PDF cifrati danno testo vuoto senza errore.
4. **`pdf-extract`.** Contro: `panic` su contenuti malformati, nessun limite alla decompressione
   (3 GiB per un file da 1 MB), ciclo nel page tree letto come documento vuoto senza errore, solo
   l'estrazione di tutte le pagine in un colpo (niente stop tra le pagine né risultati man mano),
   anche lui spezza qualche parola.
5. **`pdf_oxide` più `pdf-extract` come ripiego** quando il primo fallisce. Contro: su tutti i file
   provati, quando `pdf_oxide` fallisce fallisce anche `pdf-extract`; due parser non fidati nel
   processo, il doppio delle dipendenze da seguire.

## Decisione

`pdf_oxide`, versione bloccata `=0.3.78`, `default-features = false`, `features = ["legacy-crypto"]`,
con **`extract_text_lines`**. Nessuna libreria di ripiego.

API per pagina da usare nell'estrattore:

1. `PdfDocument::from_bytes(bytes)`: un errore qui è "danneggiato" (anche per i file che non sono
   PDF: la verifica del contenuto avviene prima, dalla firma `%PDF-`).
2. Se `doc.is_encrypted()` e `doc.authenticate(b"")?` è falso: avviso "protetto da password",
   nessun testo. Senza questo controllo un PDF cifrato sembrerebbe "senza testo".
3. `doc.page_count()`, poi per ogni pagina `i` (controllando lo stop tra una pagina e l'altra)
   `doc.extract_text_lines(i)`: ogni `TextLine::text` non vuoto è una riga; la posizione è
   `PageLine { page: i + 1, line: k + 1 }` con `k` contato sulle righe non vuote. Un errore su una
   pagina salta quella pagina e dà l'avviso "danneggiato" alla fine, senza fermare le altre.
4. Nessun carattere in tutte le pagine: avviso "PDF senza testo (probabile scansione)".
5. Mai `extract_text`, `to_plain_text`, `to_markdown`: vietati con `disallowed-methods` di Clippy.

Perché: è l'unica variante che dà righe corrette sui PDF di LibreOffice e su quelli non tagged
(il numero di riga è parte del risultato), non va in panic su nessun file provato, legge una pagina
alla volta, gestisce i cicli nel page tree e i conteggi falsi. Le parole spezzate in alcuni testi
giustificati sono un limite noto e misurato (sotto l'1% sul caso peggiore), accettato come le frasi
spezzate dagli a capo. La memoria senza limite sulle bombe di decompressione riguarda entrambe le
librerie e la ferma il limite del Job Object (verificato: 0xC0000409 entro 0,12 s con 256 MiB).

## Conseguenze

- Più facile: risultati per pagina mandati man mano, stop tra le pagine, avvisi distinti per PDF
  cifrati, danneggiati e senza testo.
- Più difficile:
  - 144 crate in più da seguire; `ttf-parser` non mantenuto (RUSTSEC-2026-0192) va aggiunto a
    `ignore` in `deny.toml` con il motivo, finché `pdf_oxide` non passa a `skrifa`;
  - con rilasci così frequenti la versione resta bloccata e si aggiorna solo di proposito,
    rieseguendo i test sulle fixture e, se serve, questo prototipo (che si può ricostruire dalla
    storia del repository);
  - il limite di memoria del processo estrattore deve tenere conto della copia dei byte fatta da
    `from_bytes` (circa due volte la dimensione del file, più le strutture: 65 MiB per un PDF da
    10 MB), da decidere nell'ADR sull'isolamento;
  - un processo terminato dal limite di memoria esce con 0xC0000409 come ogni `abort`: per dire
    "limite superato" invece di "danneggiato" serve il picco di memoria del job o la notifica del
    job, non il codice d'uscita.
- Da rivedere se una versione futura di `pdf_oxide` separa i paragrafi nell'albero di struttura
  (allora `extract_text` diventerebbe la scelta migliore) o se le parole spezzate risultano frequenti
  sui PDF italiani reali.

## Come si verifica il rispetto

- `Cargo.toml` del workspace: `pdf_oxide = { version = "=0.3.78", default-features = false,
  features = ["legacy-crypto"] }`; `deny.toml` vieta `pdf-extract` e `lopdf` in `[bans]`, controllato
  da `cargo deny check` in `cargo xtask check`.
- `clippy.toml`: `disallowed-methods` per `pdf_oxide::document::PdfDocument::extract_text`,
  `extract_text_with_options`, `extract_all_text`, `to_plain_text`, `to_plain_text_all`; clippy con
  `-D warnings` in `cargo xtask check`.
- Test dell'estrattore PDF in `lynkeus-extract`:
  - `formati/catalogo.pdf`: `aragosta` in `PageLine { page: 2, line: 3 }`, `città` in pagina 2 riga 2;
  - `elenco.pdf` dentro `demo/musica/archivio.zip`: `chitarra` in pagina 1 riga 2;
  - `malevoli/troncato.pdf` e `malevoli/immagine-finta.pdf`: avviso, nessun panic;
  - un PDF con password d'apertura (fixture da aggiungere: `catalogo.fodt` esportato da LibreOffice
    con `EncryptFile` e `DocumentOpenPassword`): avviso "protetto da password";
  - PDF sintetici generati dal test (ciclo nel page tree, annidamento profondo): nessun panic.
- Test del processo estrattore: la bomba Flate generata dal test termina il figlio per il limite di
  memoria e produce l'avviso "limite superato".
