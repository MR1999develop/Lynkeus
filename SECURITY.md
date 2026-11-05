# Sicurezza

## Versioni supportate

Si corregge solo l'ultima versione pubblicata. Chi usa una versione precedente è invitato ad aggiornare.

## Come segnalare una vulnerabilità

Non aprire una issue pubblica e non descrivere il problema in un luogo visibile a tutti.

- Quando il repository sarà pubblicato su GitHub: usa "Report a vulnerability" nella scheda Security del repository (Private Vulnerability Reporting). La segnalazione resta privata.
- Finché non è disponibile, o se preferisci scrivere per e-mail: `MR1999develop@users.noreply.github.com`.

Nella segnalazione indica la versione, il sistema operativo, i passi per riprodurre il problema e, se puoi, un file di prova (senza dati personali). Per i file ostili è utile il file stesso o lo script che lo genera.

## Tempi indicativi

Il progetto è mantenuto da una sola persona, senza impegni contrattuali:

- conferma di ricezione entro 7 giorni;
- prima valutazione (riproduzione e gravità) entro 14 giorni;
- correzione di una vulnerabilità confermata entro 90 giorni, prima se è grave; la divulgazione pubblica avviene dopo la correzione, d'accordo con chi ha segnalato.

## Nel perimetro

- **File ostili letti dal programma**: documenti, archivi e file di testo costruiti per far crollare, bloccare o saturare il programma, per fargli leggere fuori dai limiti, per aggirare i limiti di dimensione, tempo e memoria, o per far uscire dati.
- **Processo estrattore**: l'isolamento, i limiti, il protocollo con il processo principale, e l'apertura delle voci d'archivio (nomi dei file generati, estensioni rese innocue, nessuna apertura di programmi o script, cartella temporanea privata).
- **Istanza unica**: il canale con cui una seconda copia del programma passa i percorsi alla prima (pipe su Windows, socket su Linux), compresa l'autenticazione e l'integrità dei messaggi.
- **Cache**: il testo estratto conservato su disco (permessi, voci alterate o danneggiate, crescita illimitata).
- Voci del menu del clic destro (registro di Windows, script Nautilus) e file esportati (iniezione di formule nei CSV).

## Fuori dal perimetro

- Vulnerabilità dei programmi che il sistema usa per aprire un file (lettori PDF, suite Office, …) o del sistema operativo.
- Attacchi che richiedono già il controllo dell'account dell'utente o l'accesso in scrittura alla sua cartella dati, alla cache o alle impostazioni (per esempio sostituire l'eseguibile).
- Il fatto che l'eseguibile non sia firmato e che SmartScreen mostri un avviso: è una scelta nota, descritta nel README, con `SHA256SUMS` per verificare l'integrità.
- Limiti di funzionalità già documentati (per esempio PDF scansionati senza OCR, file `.rar` non supportati).
- Segnalazioni sulle dipendenze senza un percorso sfruttabile nel programma; sono controllate con `cargo deny` a ogni modifica.
- Attacchi di negazione del servizio che richiedono di cercare in una cartella già sotto il controllo dell'attaccante oltre i limiti configurati dall'utente.
