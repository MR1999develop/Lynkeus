# Esiti del prototipo Job Object

Da riportare nell'ADR sull'isolamento in processo (non ancora presente in questo ramo). Il prototipo
si butta dopo la decisione.

Comandi (da una console, build `release`): `jobobject-experiment all`, oppure `test-mem`,
`test-kill`, `test-env`, `test-raw`, `bench <n>`, `wrap <MiB> <programma> [argomenti]`.
Macchina: Ryzen 5 7600, Windows 10 22H2, Rust 1.99.1, `win32job` 2.0.3, `windows-sys` 0.61.

Il figlio è lo stesso eseguibile con l'opzione `child`: non fa nulla finché non riceve un comando su
stdin ed esce quando stdin si chiude, come il processo estrattore previsto.

## Limite di memoria per processo

- `win32job` 2.0.3 **non offre il limite di memoria per processo**: `ExtendedLimitInfo` ha solo il
  limite del working set, il campo interno è `pub(crate)` e non si può completare. Il limite si
  imposta con una chiamata diretta a `SetInformationJobObject`
  (`JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, `ProcessMemoryLimit`),
  cioè con `unsafe` nel modulo FFI.
- Con 256 MiB:
  - `Vec::try_reserve_exact(128 MiB)` riesce; `try_reserve_exact(512 MiB)` restituisce errore e il
    processo resta vivo e risponde;
  - un'allocazione normale da 512 MiB (`vec![1u8; n]`) termina il processo in 40–67 ms con codice
    `0xC0000409`, lo stesso di qualsiasi `abort` di Rust: il codice d'uscita da solo non distingue
    "memoria finita" da un altro aborto. Il job però riporta il picco (`PeakProcessMemoryUsed`,
    qui 128,9 MiB per il figlio sopravvissuto); in alternativa la notifica
    `JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT` su una completion port (non provata).
- Il solo limite del working set (l'unico di `win32job`) non fa fallire le allocazioni; in questa
  configurazione `SetInformationJobObject` lo rifiuta del tutto ("Failed to set info for job").
- Estrattori PDF dentro il job (`wrap 256`): la bomba Flate da 1 GiB termina il figlio a
  194–198 MiB in 120 ms, con `pdf_oxide` e con `pdf-extract`; il manuale di VirtualBox (1.200 pagine,
  10 MB) passa con 65–72 MiB di picco.

## Morte del figlio con il padre

| Caso | Figlio |
|---|---|
| Il proprietario chiude l'handle del job (uscita normale), stdin del figlio ancora aperto | morto in 1,0–1,4 ms |
| Padre con job terminato con `taskkill /F`, figlio bloccato che non legge stdin | morto (209 ms dal lancio di `taskkill.exe`, avvio di `taskkill` compreso) |
| Controllo: padre senza job terminato con `taskkill /F`, figlio bloccato | **vivo** dopo 5 s (terminato poi a mano) |
| Senza job, stdin chiuso a un figlio in attesa su stdin | uscito in 1,0 ms |

`KILL_ON_JOB_CLOSE` funziona anche quando il padre è ucciso: il kernel chiude i suoi handle e con
l'ultimo handle del job muoiono i figli. L'handle del job creato con attributi nulli non è
ereditabile, quindi i figli non lo tengono aperto. L'uscita su EOF di stdin copre solo i figli che
leggono: un decoder bloccato sopravvive senza il job (vale anche per il ripiego su Linux).

## Ambiente e handle ereditati

- `Command::env_clear()` più `SystemRoot`: il figlio vede solo `SystemRoot=C:\Windows`, e funziona.
- **`std::process::Command` fa ereditare al figlio tutti gli handle ereditabili del padre**
  (`CreateProcessW` con `bInheritHandles = TRUE` e senza `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`): nella
  prova 14 su 14, tra cui file e pipe che il padre aveva ricevuto da chi lo aveva avviato e un file
  reso ereditabile apposta. I file aperti da std non sono ereditabili e non passano. `Stdio::null()`
  per stderr apre `NUL` e lo passa al figlio.
- **Con `CreateProcessW` e le liste di attributi** (`ffi::spawn_raw`):
  - `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` con i soli capi della pipe di stdin e stdout: 0 su 14 handle
    ereditabili del padre arrivano al figlio; il figlio ha solo stdin, stdout e l'handle della
    propria cartella corrente; stderr è assente (le scritture di std su uno stderr assente si
    perdono senza errore);
  - `PROC_THREAD_ATTRIBUTE_JOB_LIST`: `IsProcessInJob` è vero fin dalla creazione, senza finestra
    in cui il figlio gira fuori dal job;
  - ambiente con la sola `SystemRoot`, `DETACHED_PROCESS` (nessuna console), cartella corrente =
    cartella dell'eseguibile (altrimenti il figlio tiene aperta la cartella corrente del padre, che
    potrebbe essere una cartella dell'utente).

## Avvio sospeso con std

Con `Command` il figlio parte prima di essere assegnato al job. Per chiudere la finestra serve
`CREATE_SUSPENDED` e poi la ripresa del thread principale, ma `ChildExt::main_thread_handle` è
instabile in Rust 1.99: il prototipo trova il thread con un'istantanea Toolhelp, che costa circa
35 ms per avvio. `JOB_LIST` evita sia la finestra sia il costo.

## Costo di avvio

Tempo dall'avvio alla prima risposta su stdout (50 avvii per variante, mediana / p90 / max):

| Variante | Mediana | p90 | Max |
|---|---|---|---|
| `Command`, senza job | 2,2 ms | 2,4 ms | 10,5 ms |
| `Command`, job condiviso assegnato dopo l'avvio | 2,3 ms | 2,4 ms | 2,5 ms |
| `Command`, sospeso, assegnato, ripreso con Toolhelp (job condiviso o nuovo) | 37–38 ms | 39–40 ms | 48 ms |
| `CreateProcessW` con `JOB_LIST` e `HANDLE_LIST`, `DETACHED_PROCESS` | 2,2 ms | 2,3 ms | 10,5 ms |
| come sopra ma con `CREATE_NO_WINDOW` (console nascosta) | 12,7 ms | 13,0 ms | 20,9 ms |

Il job in sé non costa nulla di misurabile. L'eseguibile del prototipo è piccolo: l'eseguibile del
prodotto (egui, estrattori) va rimisurato, e l'antivirus può aggiungere tempo al primo avvio.

## Raccomandazioni per il processo estrattore

1. Avviare il figlio con `CreateProcessW` in `platform/windows_ffi.rs`, con `JOB_LIST` e
   `HANDLE_LIST` (solo stdin e stdout), ambiente con la sola `SystemRoot`, `DETACHED_PROCESS` o
   sottosistema GUI, cartella corrente = cartella dell'eseguibile. `std::process::Command` non
   garantisce "solo stdin e stdout ereditati".
2. Job unico per il pool, creato dal processo principale, non ereditabile, con
   `KILL_ON_JOB_CLOSE | PROCESS_MEMORY` (più `DIE_ON_UNHANDLED_EXCEPTION`, da provare, per evitare la
   finestra di Segnalazione errori).
3. `win32job` aggiunge poco: il limite di memoria e le liste di attributi richiedono comunque
   `windows-sys` e `unsafe`; `CreateJobObjectW`, `SetInformationJobObject`, `IsProcessInJob` e
   `QueryInformationJobObject` sono quattro chiamate. Proposta: togliere `win32job` dal piano e usare
   solo `windows-sys` (da decidere nell'ADR sull'isolamento).
4. Distinguere "limite superato" da "danneggiato" con il picco di memoria del job o la sua notifica,
   non con il codice d'uscita.
5. Nel figlio, allocazioni grandi e prevedibili (byte del file, buffer) con `try_reserve`: un errore
   diventa un avviso invece di un aborto.
