# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 MR
#
# Testi dell'interfaccia in italiano. Gli ID sono stabili: lo stesso insieme è in en/main.ftl.

app-title = Lynkeus Searcher

## Modulo di ricerca

form-folder-label = Cartella
form-folder-hint = Percorso della cartella in cui cercare
form-term-label = Termine
form-term-hint = Testo da cercare dentro le parole
action-search = Cerca
action-cancel = Annulla

## Errori del modulo

error-no-folder = Scegli la cartella in cui cercare.
error-term-empty = Scrivi il testo da cercare.
error-term-only-wildcards = Il termine contiene solo * e ?: aggiungi almeno una lettera o una cifra.
# $max (numero): caratteri ammessi.
error-term-too-long = Il termine è troppo lungo: al massimo { $max } caratteri.
error-term-line-break = Il termine non può andare a capo: cerca una riga alla volta.
error-query-too-complex = Il termine è troppo complesso: accorcialo o usa meno caratteri jolly.
# $max (numero): termini ammessi.
error-query-too-many = Puoi cercare al massimo { $max } termini insieme.
error-start-failed = Non è stato possibile avviare la ricerca: il sistema non ha risorse libere. Chiudi qualche programma e riprova.

## Tabella dei risultati

column-word = Parola
column-file = File
column-position = Posizione
column-context = Contesto
results-empty-idle = Scegli una cartella, scrivi un termine e premi Cerca.
results-empty-none = Nessuna parola trovata.

## Posizioni

# $line (numero): riga, da 1.
location-line = riga { $line }
# $page, $line (numeri): pagina e riga nella pagina.
location-page-line = pagina { $page }, riga { $line }
# $sheet (testo): nome del foglio; $cell (testo): cella, es. B3.
location-cell = { $sheet }!{ $cell }
# $slide (numero): diapositiva.
location-slide = diapositiva { $slide }
# $paragraph (numero): paragrafo.
location-paragraph = paragrafo { $paragraph }
# $part (testo): parte del messaggio (mail-part-*); $line (numero): riga.
location-mail = { $part }, riga { $line }
mail-part-subject = oggetto
mail-part-from = mittente
mail-part-to = destinatari
mail-part-body = testo

## Stato

status-idle = Pronto.
# $searched, $hits, $skipped, $binary, $warnings, $seconds (numeri).
status-running = Ricerca in corso: { $searched ->
        [one] 1 file letto
       *[other] { $searched } file letti
    }, { $hits ->
        [one] 1 parola trovata
       *[other] { $hits } parole trovate
    }, { $skipped } saltati, { $binary } binari, { $warnings ->
        [one] 1 avviso
       *[other] { $warnings } avvisi
    } ({ $seconds } s)
# Come status-running.
status-completed = Ricerca completata: { $searched ->
        [one] 1 file letto
       *[other] { $searched } file letti
    }, { $hits ->
        [one] 1 parola trovata
       *[other] { $hits } parole trovate
    }, { $skipped } saltati, { $binary } binari, { $warnings ->
        [one] 1 avviso
       *[other] { $warnings } avvisi
    } ({ $seconds } s)
# Come status-running.
status-cancelled = Ricerca annullata: { $searched ->
        [one] 1 file letto
       *[other] { $searched } file letti
    }, { $hits ->
        [one] 1 parola trovata
       *[other] { $hits } parole trovate
    } ({ $seconds } s)
# $max (numero): limite dei risultati.
status-limit-reached = Raggiunto il limite di { $max } risultati: restringi la ricerca con una cartella o un termine più precisi.
status-worker-lost = La ricerca si è interrotta per un errore interno. Riprova; se succede ancora, segnala il problema.
# $file (testo): percorso del file in lettura.
status-current-file = In lettura: { $file }

## Avvisi

warning-not-found = Non trovato
warning-access-denied = Accesso negato
warning-in-use = In uso da un altro programma
warning-io = Errore di lettura
warning-encrypted = Protetto da password
warning-corrupt = File danneggiato
warning-timeout = Tempo scaduto
warning-limit-depth = Troppi archivi uno dentro l'altro
warning-limit-entries = Troppi file nell'archivio
warning-limit-entry-size = File nell'archivio troppo grande
warning-limit-archive-size = Archivio troppo grande una volta decompresso
warning-limit-ratio = Compressione sospetta, possibile archivio malevolo
warning-limit-memory = Memoria esaurita durante la lettura
warning-content-mismatch = Il contenuto non corrisponde all'estensione
warning-no-text = Nessun testo, forse una scansione
