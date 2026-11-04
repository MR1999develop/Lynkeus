# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 MR
#
# Testi dell'interfaccia in italiano. Gli ID sono stabili: lo stesso insieme è in en/main.ftl.

app-title = Lynkeus Searcher

## Menu

menu-file = File
menu-edit = Modifica
menu-search-menu = Ricerca
menu-view = Visualizza
menu-tools = Strumenti
menu-help = Aiuto
menu-add-folder = Aggiungi cartella…
menu-add-file = Aggiungi file…
menu-recent-roots = Cartelle e file recenti
menu-quit = Esci
menu-select-all = Seleziona tutte le righe
menu-select-none = Deseleziona tutto
menu-search = Cerca
menu-cancel = Annulla la ricerca
menu-find-terms = Scrivi un termine
menu-add-term = Aggiungi un termine
menu-recent-terms = Termini recenti
menu-filters = Filtri
menu-show-results = Risultati
menu-show-warnings = Avvisi
menu-search-panel = Pannello di ricerca
menu-theme = Tema
menu-language = Lingua
menu-about = Informazioni su Lynkeus Searcher
theme-system = Come il sistema
theme-light = Chiaro
theme-dark = Scuro
language-system = Lingua del sistema
language-italian = Italiano
language-english = English

## Motivi delle voci disabilitate

reason-search-running = Una ricerca è già in corso.
reason-no-search = Nessuna ricerca in corso.
reason-no-results = Non ci sono risultati da selezionare.
reason-not-results-tab = Apri la scheda Risultati per selezionare le righe.
# Il numero massimo di termini è nel messaggio error-query-too-many.
reason-too-many-terms = Hai raggiunto il numero massimo di termini.
reason-no-recent = Ancora nessun elemento recente.
reason-one-term = Aggiungi almeno due termini per scegliere come combinarli.
reason-filters-default = I filtri sono già quelli predefiniti.

## Modulo di ricerca

form-roots-title = Dove cercare
form-roots-empty = Trascina qui cartelle o file, oppure usa i pulsanti.
form-add-folder = Aggiungi cartella…
form-add-file = Aggiungi file…
form-root-remove-tooltip = Togli dall'elenco
# $path (testo): percorso della cartella o del file.
form-root-remove-name = Togli { $path } dall'elenco
root-kind-folder = cartella
root-kind-file = file
root-kind-missing = non trovato
form-path-label = Percorso
form-path-hint = Incolla un percorso e premi Invio
form-terms-title = Termini
form-term-hint = Testo da cercare, es. ar
form-term-hint-more = Un altro termine
# $number (numero): posizione della riga, da 1.
form-term-name = Termine { $number }
form-term-remove-tooltip = Togli questo termine
# $number (numero): posizione della riga, da 1.
form-term-remove-name = Togli il termine { $number }
form-add-term = + Aggiungi un termine
form-options-title = Opzioni
option-case = Distingui maiuscole e minuscole
option-case-tooltip = Se attiva, "ar" non trova "AR".
option-accents = Ignora gli accenti
option-accents-tooltip = Se attiva, "citta" trova anche "città".
option-wildcards = Caratteri jolly * e ?
option-wildcards-tooltip = * sta per zero o più caratteri, ? per un carattere. Se spenta, * e ? si cercano così come sono.
combine-title = Con più termini, mostra i file che contengono:
combine-any = Almeno un termine
combine-any-tooltip = Ogni parola che contiene uno dei termini.
combine-all = Tutti i termini nello stesso file
combine-all-tooltip = Solo i file in cui compaiono tutti i termini.
filters-title = Filtri
filters-title-customized = Filtri (modificati)
filters-extensions = Estensioni
filters-extensions-all = Tutte
filters-extensions-include = Solo queste
filters-extensions-exclude = Tutte tranne queste
filters-extensions-hint = es. txt, md, docx
filters-skip-dirs = Cartelle da saltare
filters-skip-dirs-caption = Un nome per riga; vale a ogni livello.
filters-hidden = Includi file nascosti e di sistema
filters-online = Includi i file solo online
filters-online-tooltip = Leggerli li scarica dal cloud (per esempio da OneDrive).
filters-max-size = Salta i file più grandi di
filters-max-size-unit = { " " }MB
filters-reset = Ripristina i filtri predefiniti
action-search = Cerca
action-cancel = Annulla
action-close = Chiudi
drop-hint = Rilascia per aggiungere all'elenco
dialog-add-folder = Aggiungi cartelle in cui cercare
dialog-add-file = Aggiungi file in cui cercare

## Errori del modulo

error-no-folder = Aggiungi almeno una cartella o un file in cui cercare.
error-path-not-found = Nessun file o cartella con questo percorso.
error-term-empty = Scrivi il testo da cercare, o togli questa riga.
error-term-only-wildcards = Il termine contiene solo * e ?: aggiungi almeno una lettera o una cifra.
# $max (numero): caratteri ammessi.
error-term-too-long = Il termine è troppo lungo: al massimo { $max } caratteri.
error-term-line-break = Il termine non può andare a capo: cerca una riga alla volta.
error-query-too-complex = Il termine è troppo complesso: accorcialo o usa meno caratteri jolly.
# $max (numero): termini ammessi.
error-query-too-many = Puoi cercare al massimo { $max } termini insieme.
error-extensions-missing = Scrivi almeno un'estensione, o scegli "Tutte".
# $extension (testo): l'estensione scritta dall'utente.
error-extension-invalid = "{ $extension }" non è un'estensione valida: scrivila senza punti interni, spazi o caratteri jolly, es. txt.
# $extension (testo): l'estensione scritta; $max (numero): caratteri ammessi.
error-extension-too-long = "{ $extension }" è troppo lunga: al massimo { $max } caratteri.
error-start-failed = Non è stato possibile avviare la ricerca: il sistema non ha risorse libere. Chiudi qualche programma e riprova.

## Schede, tabelle e stati vuoti

# $count (numero): righe di risultato.
tab-results = Risultati ({ $count })
# $count (numero): avvisi.
tab-warnings = Avvisi ({ $count })
column-word = Parola
column-term = Termine
column-file = File
column-position = Posizione
column-context = Contesto
column-reason = Motivo
column-sort-tooltip = Ordina per questa colonna; un altro clic inverte l'ordine.
empty-add-root-title = Da dove cominciamo?
empty-add-root-text = Aggiungi una cartella o un file in cui cercare: trascinali qui o usa i pulsanti.
empty-write-term-title = Scrivi che cosa cercare
empty-write-term-text = Scrivi un termine nel pannello a sinistra e premi Cerca (o F5). Troverai le parole che lo contengono.
empty-searching-title = Ricerca in corso…
empty-searching-text = Le parole trovate compariranno qui man mano.
empty-none-title = Nessuna parola trovata
empty-none-text = Prova un termine più corto, i caratteri jolly, "Ignora gli accenti", o controlla i filtri.
warnings-empty = Nessun avviso: tutti i file sono stati letti.
banner-limit-title = Troppi risultati.
# $max (numero): limite dei risultati.
banner-limit-text = La ricerca si è fermata a { $max } righe: usa un termine più preciso, meno cartelle o i filtri.

## Informazioni

# $version (testo): versione del programma.
about-version = Versione { $version }
about-description = Trova le parole che contengono un testo, in file e cartelle.
about-license = Software libero, distribuito con licenza GNU GPL versione 3.

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
# $done (numero): file già esaminati; $seen (numero): file trovati finora.
status-progress = { $done } di { $seen } file
# $count (numero): righe selezionate.
status-selected = { $count ->
        [one] 1 riga selezionata
       *[other] { $count } righe selezionate
    }

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
