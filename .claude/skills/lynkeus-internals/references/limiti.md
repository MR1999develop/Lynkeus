# Limiti e valori predefiniti

| Cosa | Valore |
|---|---|
| Risultati massimi | 200.000 (modificabile); al limite la ricerca si ferma e un banner lo segnala |
| Parola nei risultati | oltre 200 caratteri: troncata con "…" |
| Contesto | circa 120 caratteri intorno alla parola |
| Anteprima | unità di testo con ±20 unità vicine |
| Rilevamento binari | NUL nei primi 8 KB (senza schema UTF-16) o firma nota |
| Lettura dei file di testo | blocchi di circa 8 MB tagliati a fine riga; la codifica si decide sul primo blocco; una riga più lunga di un blocco si taglia dopo l'ultimo spazio (file con soli CR = una riga); passi di ricerca di 10.000 righe e 64 KiB (`step.rs`), usati anche per le unità dei documenti: limitano la memoria dei risultati di `find_words` di un passo (circa 240 byte per parola trovata, al più una parola ogni due byte: circa 8 MB per thread) |
| Dimensione massima di un file | 200 MiB (modificabile o senza limite); vale anche per le radici; in `settings.toml` da 1 a 1.048.576 MiB |
| Luoghi di ricerca (radici) | al più 256 (`MAX_ROOTS`, `form.rs`); vale per l'elenco, per i percorsi dell'istanza unica e per `roots` di `settings.toml` (oltre, scartati); il 257° non entra e compare l'avviso `notice-roots-limit` |
| Termini | 32 per ricerca, 256 caratteri ciascuno (spazi esterni esclusi) |
| Estensioni nei filtri | 32 caratteri, senza punti interni |
| Recenti | 10 radici, 10 termini |
| `settings.toml` | letto fino a 1 MiB, elenchi fino a 256 voci, stringhe fino a 4096 byte |
| Processi estrattori | `min(core/2, 4)`, avviati al primo documento |
| Timeout per file | 60 s, poi processo terminato e sostituito |
| Annulla | la ricerca chiude lo stdin dei processi e risponde subito (`Stopped`); il thread del pool termina i processi che non escono entro 500 ms (`STOP_GRACE`) e li raccoglie in altri 500 ms. Budget dei test: 1 s dall'Annulla a `Finished`, anche con la CPU satura (`tests/annulla.rs`) |
| Memoria per processo estrattore | 1 GiB (minimo accettato dal figlio 64 MiB); Windows: memoria impegnata, Job Object; Linux: `RLIMIT_AS` |
| Messaggio del protocollo | 1,25 MiB per frame (`MAX_FRAME_BYTES`, versione e corpo: un'unità di testo o un pezzo di voce da 1 MiB più 256 KiB di margine) |
| Unità di testo | 1 MiB (`MAX_UNIT_TEXT_BYTES`); il padre lo rivalida, come la profondità delle voci |
| Fogli di calcolo ods e xls | memoria stimata in byte, al più 512 MiB (`MAX_WORKBOOK_BYTES`, `extract/sheet_memory.rs`); oltre, avviso "memoria". Conta: il rettangolo di ogni foglio (un valore per cella, più il testo di ogni cella, clonato per ogni ripetizione), le stringhe condivise (SST) degli xls e, per il foglio più grande, la parte transitoria (celle lette o copia del rettangolo). Le formule degli xls si svuotano prima di misurare. Gli xlsx si leggono a flusso, senza questa stima. Sta dentro il limite di 1 GiB del processo: ci stanno un xls denso di 65.536 righe per 40 colonne e un ods di 100.000 per 40 con valori brevi. Falso positivo noto: un xls con record sovrapposti, dove una formula sfugge allo svuotamento, è rifiutato con lo stesso avviso |
| Appunti | al più 8 MiB di testo (`MAX_CLIPBOARD_BYTES`, `app.rs`); oltre, niente appunti e un messaggio propone Esporta con le sole righe selezionate |
| Coda dei messaggi per processo | 16 messaggi non ancora consumati (con frame da 1,25 MiB, al più circa 20 MiB) |
| Archivi: profondità | 5 |
| Archivi: voci | 100.000 |
| Archivi: byte per voce | 512 MB |
| Archivi: byte per archivio | 4 GB |
| Archivi: rapporto di compressione | 200:1 |
| RTF: profondità dei gruppi | 256 (`MAX_GROUP_DEPTH`); oltre, documento "danneggiato" |
| EML: allegati | contano come voci (100.000), un livello di profondità in più, 512 MB per allegato decodificato e totali; il messaggio si legge in memoria entro il massimo per voce |
| 7z | voci contate sull'elenco; dimensione dichiarata controllata prima di leggere; in un blocco solido, superato un limite il resto del blocco ha lo stesso avviso; AES: "protetto da password" |
| tar | voci contate man mano (non c'è elenco in testa); dimensione dichiarata controllata prima; link, dispositivi e file sparsi saltati |
| gz / tgz | stessi limiti degli zip; rapporto di compressione sui byte del file compresso; più membri concatenati letti; contenuto = un documento, o le voci di un tar |
| Cache | LRU, 500 MB predefiniti (`[cache] max_size_mib`, da 16 a 65.536 MiB; menu: 100, 250, 500, 1000, 2000) |
| Voce di cache | al più 64 MiB di frame decompressi; oltre, il documento non si salva |
| Temporanei di cache | più vecchi di 1 ora: cancellati |
| Errori di fila nell'attesa di un client (istanza unica) | 20 (`MAX_ACCEPT_ERRORS`): poi il server si ferma, registrandolo |
| Voce d'archivio aperta | byte originali della voce (qualsiasi formato), al più il limite per voce degli archivi (512 MiB), in pezzi da 1 MiB |
| Barra di avanzamento | compare dopo 1 s |
| Token dell'istanza unica | 128 bit casuali |

I limiti sugli archivi sono contati sui byte decompressi con `Read::take` (anche per docx e odt) e modificabili in "Avanzate".

File nascosti: su Windows solo per attributo `HIDDEN` o `SYSTEM`, su Linux per il punto iniziale. File solo online: attributi `RECALL_ON_DATA_ACCESS`, `RECALL_ON_OPEN`, `OFFLINE`; saltati anche se sono una radice. Il confronto dei nomi di cartella da saltare ignora le maiuscole su Windows.

Cartelle saltate per impostazione predefinita: `.git`, `node_modules`, `target`, `$RECYCLE.BIN`, `System Volume Information`.

Opzioni predefinite: maiuscole ignorate, accenti distinti, jolly accesi, file solo online saltati, OR.
