# Threat model (prima versione)

Metodo: le Quattro Domande e STRIDE. Le sezioni sul processo estrattore e sulla pipe dell'istanza unica sono provvisorie: verranno approfondite nella fase 4 (processo estrattore e archivi) e nella fase 7 (pipe e registro).

## 1. Le Quattro Domande

1. **Cosa stiamo costruendo?** Un'app desktop che legge file e archivi scelti dall'utente, ne estrae il testo (parte in un processo figlio isolato), cerca e mostra i risultati; conserva una cache del testo, impostazioni e, a richiesta, integra una voce nel clic destro.
2. **Cosa può andare storto?** Vedi sezione 4.
3. **Cosa facciamo al riguardo?** Mitigazioni elencate per ogni confine nella sezione 4.
4. **Abbiamo fatto un buon lavoro?** Si verifica con i test di robustezza (file malevoli, fuzzing dei parser propri), la verifica delle ACL e dei processi figli, la ricerca di dati personali nell'eseguibile; l'esito si registra qui a ogni fase.

## 2. Asset

- **Documenti dell'utente e il loro testo nella cache:** riservatezza.
- **Integrità del sistema dell'utente:** il programma non deve eseguire codice o scrivere fuori dalle sue cartelle a causa di un file letto.
- **Disponibilità dell'app:** un file ostile non deve bloccarla né esaurire memoria, disco o CPU.

## 3. Trust boundary

1. File e archivi letti (input non fidato).
2. Processo estrattore (figlio che decodifica i formati complessi).
3. Pipe (Windows) o socket (Linux) dell'istanza unica.
4. Registro di Windows e script Nautilus (voce del clic destro).
5. Cartella dati e cache (altri utenti o processi dello stesso computer).
6. File esportati (CSV/TXT letti poi da altri programmi).

## 4. STRIDE per confine

### 4.1 File e archivi letti

- **S**: un file finge un'estensione. Mitigazione: formato verificato dal contenuto; incongruenze negli avvisi.
- **T**: nomi di voci con `..` o assoluti. Mitigazione: i nomi sono solo etichette, mai usati per scrivere su disco.
- **R**: non rilevante.
- **I**: nessuna uscita in rete; i risultati restano locali.
- **D**: zip bomb, archivi annidati, file enormi, regex costose. Mitigazione: limiti su profondità, voci, byte decompressi (`Read::take`) e rapporto 200:1; regex lineare del crate `regex`; lettura a blocchi; timeout.
- **E**: bug nei decoder. Mitigazione: decoder in un processo isolato (4.2), `forbid(unsafe_code)` nei crate di dominio, DTD XML rifiutati.

### 4.2 Processo estrattore (da approfondire nella fase 4)

- **S**: avvio di un eseguibile diverso. Mitigazione: percorso assoluto di `current_exe()`.
- **T**: messaggi alterati sul canale stdin/stdout. Mitigazione: protocollo `postcard` con lunghezza in testa e dimensione massima.
- **I**: ambiente e handle ereditati. Mitigazione: `env_clear` salvo poche variabili di sistema, nessun handle oltre stdin e stdout.
- **D**: processo bloccato o vorace. Mitigazione: timeout di 60 s, Job Object con limite di memoria e `KILL_ON_JOB_CLOSE` (Windows), `RLIMIT_AS` (Linux), uscita alla chiusura dello stdin, pool limitato a `min(core/2, 4)`.
- **E**: compromissione del figlio. Mitigazione: risorse limitate e figlio senza scritture sui dati dell'utente; isolamento più forte da valutare nell'ADR.

### 4.3 Pipe e socket dell'istanza unica (da approfondire nella fase 7)

- **S**: processo ostile che si spaccia per la prima istanza o per un client. Mitigazione: pipe con nome casuale creata con `FILE_FLAG_FIRST_PIPE_INSTANCE`; client con `SECURITY_IDENTIFICATION`; token casuale da 128 bit in un file privato, confronto in tempo costante.
- **T/E**: percorsi malevoli iniettati. Mitigazione: dimensione massima dei messaggi, solo percorsi esistenti, aggiunti all'elenco senza avviare la ricerca.
- **I/D**: altri utenti sul computer. Mitigazione: DACL esplicita solo per l'utente corrente (Windows), socket 0600 in `$XDG_RUNTIME_DIR` (Linux); attivo solo con `--add`.

### 4.4 Registro e script Nautilus

- **T/E**: voce del registro che punta a un eseguibile modificato. Mitigazione: scrittura solo in `HKCU` (nessun amministratore); percorso dell'eseguibile aggiornato all'avvio e voce rimossa quando l'opzione è spenta.
- **T**: percorsi con caratteri speciali nella riga di comando. Mitigazione: argomento `"%1"` tra virgolette, trattato come dato.

### 4.5 Cartella dati e cache

- **I**: la cache contiene testo dei documenti. Mitigazione: cartelle private (descrittore esplicito solo utente e SYSTEM su Windows; 0700 e 0600 su Linux), cache spegnibile e svuotabile, avviso nel README.
- **T**: voci di cache alterate o corrotte. Mitigazione: intestazione con versione e CRC; voce non valida ignorata e cancellata; creazione dei file che fallisce se esistono.
- **D**: crescita illimitata. Mitigazione: limite LRU di 500 MB.

### 4.6 File esportati

- **T/E**: iniezione di formule nei CSV aperti con un foglio di calcolo (testo che inizia con `=`, `+`, `-`, `@`). Da trattare nella fase 6.
- **I**: l'esportazione contiene contesto dei documenti: scelta esplicita dell'utente.

## 5. Rischi accettati

- **Eseguibile non firmato:** progetto personale senza certificato di firma del codice. Compensazione: `SHA256SUMS` e istruzioni nel README; SmartScreen mostrerà un avviso.
- **Voce del clic destro in "Mostra altre opzioni" su Windows 11:** scelta concordata.
- **Processo estrattore con gli stessi diritti dell'utente:** accettato con limiti di risorse; da rivedere nell'ADR sull'isolamento.
- **PDF scansionati e .rar non letti:** limite funzionale, non di sicurezza.
