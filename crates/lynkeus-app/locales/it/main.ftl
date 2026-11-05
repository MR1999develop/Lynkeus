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
menu-open = Apri
menu-reveal = Mostra nella cartella
menu-export = Esporta…
menu-export-settings = Impostazioni di esportazione…
menu-copy = Copia
menu-preview = Anteprima
menu-next-in-file = Risultato successivo nel file
menu-previous-in-file = Risultato precedente nel file
menu-next-file = File successivo
menu-previous-file = File precedente
menu-next-pane = Riquadro successivo
menu-previous-pane = Riquadro precedente
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
reason-no-selection = Seleziona almeno una riga.
reason-no-current-row = Fai clic su una riga dei risultati.
reason-last-in-file = È l'ultimo risultato di questo file.
reason-first-in-file = È il primo risultato di questo file.
reason-last-file = È l'ultimo file dei risultati.
reason-first-file = È il primo file dei risultati.
reason-nothing-to-export = Non ci sono risultati da esportare.
reason-export-running = Un'esportazione è già in corso.
reason-export-no-selection = Nessuna riga selezionata: selezionane qualcuna, o scegli "Tutte le righe" in Impostazioni di esportazione.
reason-dialog-open = Una finestra di dialogo è già aperta.
reason-copy-running = Una copia negli appunti è già in corso.

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
root-kind-pending = da verificare
form-path-label = Percorso
form-path-hint = Incolla un percorso e premi Invio
form-path-checking = Verifica del percorso…
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
# Bottone accanto a un messaggio: rimette com'era prima.
action-undo = Annulla
drop-hint = Rilascia per aggiungere all'elenco
dialog-add-folder = Aggiungi cartelle in cui cercare
dialog-add-file = Aggiungi file in cui cercare
dialog-export = Esporta i risultati

## Errori del modulo

error-no-folder = Aggiungi almeno una cartella o un file in cui cercare.
error-path-not-found = Nessun file o cartella con questo percorso.
# $max (numero): numero massimo di luoghi nell'elenco.
error-path-too-many-roots = L'elenco ha già { $max } luoghi, il massimo. Togline qualcuno per aggiungerne altri.
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
# Sotto la barra di stato quando Cerca non parte per un campo sbagliato.
error-form-invalid = Ricerca non avviata: correggi i campi segnalati nel pannello di ricerca.

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
# Nome di una riga dei risultati per i lettori di schermo.
# $word (testo): parola trovata; $file (testo): file; $location (testo): posizione, es. "riga 12".
table-row-name = { $word }, { $file }, { $location }
empty-add-root-title = Da dove cominciamo?
empty-add-root-text = Aggiungi una cartella o un file in cui cercare: trascinali qui o usa i pulsanti.
empty-write-term-title = Scrivi che cosa cercare
empty-write-term-text = Scrivi un termine nel pannello a sinistra e premi Cerca (o F5). Troverai le parole che lo contengono.
empty-searching-title = Ricerca in corso…
empty-searching-text = Le parole trovate compariranno qui man mano.
empty-none-title = Nessuna parola trovata
empty-none-text = Prova un termine più corto, i caratteri jolly, "Ignora gli accenti", o controlla i filtri.
empty-cancelled-title = Ricerca annullata
empty-cancelled-text = La ricerca è stata annullata prima di trovare parole. Per ripeterla premi "Ripeti la ricerca" o F5.
empty-interrupted-title = Ricerca interrotta
empty-interrupted-text = La ricerca si è interrotta per un errore interno prima di trovare parole. Ripetila; se succede ancora, segnala il problema.
empty-search-again = Ripeti la ricerca
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
# $searched (numero): file letti; $hits (numero): parole trovate; $skipped (numero): file saltati; $binary (numero): file binari; $warnings (numero): avvisi; $seconds (numero): secondi trascorsi.
status-running = Ricerca in corso: { $searched ->
        [one] 1 file letto
       *[other] { $searched } file letti
    }, { $hits ->
        [one] 1 parola trovata
       *[other] { $hits } parole trovate
    }, { $skipped ->
        [one] 1 saltato
       *[other] { $skipped } saltati
    }, { $binary ->
        [one] 1 binario
       *[other] { $binary } binari
    }, { $warnings ->
        [one] 1 avviso
       *[other] { $warnings } avvisi
    } ({ $seconds } s)
# Stesse variabili di status-running.
status-completed = Ricerca completata: { $searched ->
        [one] 1 file letto
       *[other] { $searched } file letti
    }, { $hits ->
        [one] 1 parola trovata
       *[other] { $hits } parole trovate
    }, { $skipped ->
        [one] 1 saltato
       *[other] { $skipped } saltati
    }, { $binary ->
        [one] 1 binario
       *[other] { $binary } binari
    }, { $warnings ->
        [one] 1 avviso
       *[other] { $warnings } avvisi
    } ({ $seconds } s)
# $searched, $hits, $seconds come in status-running.
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
# $done (numero): file già esaminati; $seen (numero): file trovati finora (il totale non è noto).
status-progress = { $done } di { $seen } file trovati finora
# $done (numero): file già esaminati; $total (numero): file da esaminare in tutto (la scansione delle cartelle è finita).
status-progress-total = { $done } di { $total } file
# $done e $total come in status-progress-total; $seconds (numero): secondi che restano, stimati.
status-progress-seconds-left = { $done } di { $total } file, ancora circa { $seconds } s
# $done e $total come in status-progress-total; $minutes (numero): minuti che restano, stimati.
status-progress-minutes-left = { $done } di { $total } file, ancora circa { $minutes } min
# $count (numero): righe selezionate.
status-selected = { $count ->
        [one] 1 riga selezionata
       *[other] { $count } righe selezionate
    }
# Nella barra di stato mentre la copia di molte righe è in preparazione.
status-copying = Copia delle righe negli appunti…

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
warning-extractor-unavailable = Lettore dei documenti non avviato

## Anteprima

preview-empty = Fai clic su un risultato per vederne l'anteprima.
preview-loading = Lettura del file…
preview-context-only = L'anteprima completa di questo documento non è disponibile; ecco il contesto trovato.
preview-previous-file = « File
preview-previous = ‹ Precedente
preview-next = Successivo ›
preview-next-file = File »
preview-open = Apri
# $file (testo): percorso del file; $location (testo): posizione, es. "riga 12".
preview-title = { $file } — { $location }
# $reason (testo): motivo, come nella scheda Avvisi.
preview-unreadable = Anteprima non disponibile: { $reason }.
preview-changed = Anteprima non disponibile: il file è cambiato dopo la ricerca. Ripeti la ricerca per aggiornare i risultati.
preview-no-thread = Anteprima non disponibile: il sistema non ha risorse libere. Chiudi qualche programma e riprova.

## Messaggi sopra la tabella

notice-executable = Per sicurezza i programmi e gli script non si aprono da qui: usa "Mostra nella cartella" e aprilo da lì se ti fidi.
notice-archive-entry = Le voci degli archivi non si possono aprire ora, perché il lettore dei documenti non è attivo: usa "Mostra nella cartella" per aprire l'archivio.
# $file (testo): percorso del file.
notice-open-failed = Errore: { $file } non si è aperto. Controlla che esista ancora e che ci sia un programma per aprirlo.
# $file (testo): percorso del file.
notice-reveal-failed = Errore: non è stato possibile mostrare { $file } nella cartella. Controlla che esista ancora.
# $count (numero): righe copiate.
notice-copied = { $count ->
        [one] 1 riga copiata negli appunti.
       *[other] { $count } righe copiate negli appunti.
    }
# $count (numero): righe scritte; $file (testo): file esportato.
notice-exported = { $count ->
        [one] 1 riga esportata in { $file }.
       *[other] { $count } righe esportate in { $file }.
    }
# $file (testo): file che si voleva scrivere.
notice-export-failed = Errore: esportazione in { $file } non riuscita; un file già presente non è stato toccato. Scegli un'altra cartella o controlla lo spazio libero.
notice-no-thread = Errore: il sistema non ha risorse libere. Chiudi qualche programma e riprova.
notice-filters-reset = Filtri riportati ai valori predefiniti.
notice-settings-newer = Attenzione: le impostazioni sono state salvate da una versione più recente di Lynkeus Searcher e non verranno sovrascritte: le modifiche fatte ora non si salvano. Aggiorna il programma per conservarle.
notice-settings-unreadable = Attenzione: il file delle impostazioni non si è potuto leggere (permessi o dimensione) e non verrà sovrascritto: le modifiche fatte ora non si salvano. Controlla il file settings.toml nella cartella dell'app.
notice-settings-write-failed = Errore: le impostazioni non si sono salvate. Controlla lo spazio libero e i permessi della cartella dell'app; il salvataggio si riprova più tardi.
notice-settings-replaced = Attenzione: il file delle impostazioni era danneggiato ed è stato messo da parte come settings.toml.invalid; si riparte dalle impostazioni predefinite.

## Esportazione

export-dialog-title = Impostazioni di esportazione
export-format-title = Formato
# $separator (testo): carattere che separa le colonne.
export-format-csv-choice = CSV per i fogli di calcolo (colonne separate da { $separator })
export-format-csv = CSV
export-format-txt = Testo con colonne separate da tabulazioni
export-scope-title = Righe
# $count (numero): righe della tabella.
export-scope-visible = Tutte le righe ({ $count })
# $count (numero): righe selezionate.
export-scope-selected = Solo le righe selezionate ({ $count })
export-dialog-confirm = Esporta…
# Nome proposto per il file, senza estensione.
export-file-name = risultati

## Cache dei documenti (menu Strumenti)

menu-cache = Cache dei documenti
menu-cache-enabled = Usa la cache
menu-cache-enabled-hint = Conserva il testo letto da PDF, documenti e archivi, così la ricerca successiva negli stessi file è più veloce. Il testo resta nella cartella dell'app finché non svuoti la cache.
menu-cache-size = Dimensione massima
# $mib (numero): dimensione in megabyte.
menu-cache-size-choice = { $mib } MB
menu-cache-clear = Svuota la cache
reason-no-cache = La cartella della cache non è disponibile (il sistema non la indica o non è privata).
reason-cache-clearing = La cache si sta svuotando.
# $mib (numero): megabyte liberati.
notice-cache-cleared = Cache svuotata: { $mib } MB liberati.
notice-cache-clear-failed = Errore: la cache non si è svuotata del tutto. Chiudi le altre finestre di Lynkeus Searcher e riprova.
notice-cache-not-written = Attenzione: la cache dei documenti non si riesce a scrivere. Le ricerche funzionano, ma rileggono ogni volta PDF, documenti e archivi. Controlla lo spazio libero e i permessi della cartella dell'app, oppure spegni la cache in Strumenti › Cache dei documenti.

## Voci d'archivio che non si aprono

# $file (testo): voce d'archivio, come mostrata nella tabella.
notice-entry-encrypted = Errore: { $file } è protetto da password e non si può estrarre per aprirlo. Apri l'archivio con il suo programma ("Mostra nella cartella") e inserisci la password lì.
# $file (testo): voce d'archivio, come mostrata nella tabella.
notice-entry-limit = Errore: { $file } supera i limiti di sicurezza per gli archivi (dimensione, compressione o tempo) e non è stato estratto. Apri l'archivio con il suo programma ("Mostra nella cartella").
# $file (testo): voce d'archivio, come mostrata nella tabella.
notice-entry-corrupt = Errore: { $file } non si può estrarre perché l'archivio è danneggiato o è cambiato dopo la ricerca. Ripeti la ricerca o apri l'archivio con il suo programma.
# $file (testo): voce d'archivio, come mostrata nella tabella.
notice-entry-unreadable = Errore: { $file } non si è potuto estrarre: l'archivio non si legge (spostato, in uso o senza permessi) o non c'è spazio libero. Controlla e riprova.

## Clic destro e diagnostica (menu Strumenti)

# Testo della voce nel menu del clic destro di Explorer e Nautilus.
context-menu-label = Cerca con Lynkeus
menu-context-menu = "Cerca con Lynkeus" nel clic destro
menu-context-menu-hint = Aggiunge "Cerca con Lynkeus" al menu del clic destro su file e cartelle; la voce apre questa finestra con l'elemento scelto tra i luoghi in cui cercare. Su Windows 11 è in "Mostra altre opzioni".
reason-no-context-menu = La voce del clic destro non è disponibile su questo sistema.
reason-context-menu-updating = La voce del clic destro si sta aggiornando.
notice-context-menu-exe-path = Errore: la voce del clic destro non è stata aggiunta perché il percorso del programma contiene caratteri non ammessi. Sposta Lynkeus Searcher in una cartella con un nome semplice e riprova.
notice-context-menu-too-long = Errore: la voce del clic destro non è stata aggiunta perché il percorso del programma è troppo lungo. Sposta Lynkeus Searcher in una cartella più vicina alla radice del disco e riprova.
notice-context-menu-exe-dir-off = Errore: la voce del clic destro è stata tolta (o non aggiunta) e l'opzione è spenta, perché altri utenti possono modificare il programma o la sua cartella. Sposta Lynkeus Searcher nella tua cartella utente o in Programmi e riaccendi l'opzione.
notice-context-menu-foreign = Errore: nel clic destro c'è già una voce con lo stesso nome di un altro programma; non è stata modificata.
notice-context-menu-io = Errore: la voce del clic destro non è stata aggiornata. Riprova; se succede ancora, controlla i permessi del tuo profilo.
# $max (numero): numero massimo di luoghi nell'elenco.
notice-roots-limit = Attenzione: l'elenco dei luoghi ha già { $max } voci, il massimo, e gli altri luoghi non sono stati aggiunti. Togline qualcuno e riprova.
# $count (numero): cartelle dell'app rifiutate.
notice-private-dirs = { $count ->
        [one] Attenzione: una cartella dell'app è accessibile ad altri utenti e per sicurezza non viene usata in questa sessione (cache, log o impostazioni potrebbero non essere salvati). Controlla i permessi delle cartelle di Lynkeus Searcher nel tuo profilo.
       *[other] Attenzione: { $count } cartelle dell'app sono accessibili ad altri utenti e per sicurezza non vengono usate in questa sessione (cache, log o impostazioni potrebbero non essere salvati). Controlla i permessi delle cartelle di Lynkeus Searcher nel tuo profilo.
    }
menu-log-level = Dettaglio del log
log-level-error = Solo errori
log-level-warn = Errori e avvisi
log-level-info = Anche le informazioni
log-level-debug = Tutto (per la diagnosi)
