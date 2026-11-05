# Ricerca (lynkeus-core)

Crate puro, senza I/O. Ogni termine diventa una regex del crate `regex` (tempo lineare); la regex non arriva mai grezza dall'utente. Codice: `query.rs` (termini e query), `pattern.rs` (traduzione), `matcher.rs` (dalle corrispondenze alle parole), `unicode.rs` (tabelle `Mn`, `P`, varianti accentate).

## Termine

`Term::parse` toglie gli spazi (White_Space Unicode) iniziali e finali: con `" car"` la corrispondenza inizierebbe con uno spazio e la parola si estenderebbe oltre. Poi controlla, nell'ordine: lunghezza (256 caratteri, senza quegli spazi), a capo (errore), vuoto, solo `*`/`?`/spazi con i jolly accesi. Vuoto e solo jolly si giudicano dopo la normalizzazione, senza i segni combinanti `Mn` (`without_marks`: NFD senza `Mn`), con qualunque opzione degli accenti, perché un termine valido lo resti cambiando opzione: un termine fatto solo di segni combinanti (U+0301) è vuoto (`TermError::Empty`), e uno che si riduce a soli jolly (`*` più U+0301) è `TermError::OnlyWildcards`, invece di diventare un `*` che trova ogni parola. Una `Query` ha al massimo 32 termini. Tipo valido per costruzione.

## Compilazione di un termine

1. Il termine è portato in NFC. Con gli accenti ignorati: NFD, tolti i segni `Mn`, poi di nuovo NFC. Il ritorno a NFC protegge l'Hangul: la NFD scompone la sillaba in jamo (che non sono `Mn`) e senza la ricomposizione "한국" non troverebbe "한국 사람". I letterali sono escapati.
2. Jolly accesi: `?` -> `[^\s\p{Mn}]\p{Mn}*` (un carattere non spazio con i suoi segni combinanti; nel testo hangul decomposto contano come segni anche i jamo di continuazione); `*` -> `(?:[^\s\p{Mn}]\p{Mn}*)*` (greedy). Non sono `\S` e `\S*` perché su testo NFD `\S` potrebbe prendere da solo il segno di una lettera: "e*" non deve trovare la "é" decomposta, come non trova quella precomposta. `*` consecutivi valgono uno. Jolly spenti: `*` e `?` letterali.
3. Ogni sequenza di spazi -> `[^\S\r\n]+`: nessuna corrispondenza attraversa una riga.
4. Maiuscole ignorate -> flag `(?i)` del `RegexBuilder` (non del pattern); distinte -> nessun flag.
5. Accenti distinti: una lettera con decomposizione canonica diventa `(?:à|a\x{300})`. Il testo non si normalizza, quindi le posizioni restano quelle originali.
6. Accenti ignorati: ogni carattere diventa la classe di sé e delle sue varianti accentate precomposte, seguita da `\p{Mn}*`. La tabella delle varianti si costruisce una volta dalla decomposizione Unicode (`LazyLock`). Le lettere senza decomposizione canonica (ø, ł, æ) non hanno varianti: restano distinte da o, l, ae anche con gli accenti ignorati.
7. Gruppo finale: il pattern termina con `(?P<next>[^\p{Mn}]|\z)`, perché una corrispondenza non sia seguita da ciò che continua il suo ultimo carattere. Con gli accenti distinti sono i segni `Mn` e i jamo ("citta" non trova la "città" decomposta); con gli accenti ignorati solo i jamo, perché i segni li assorbe già `\p{Mn}*`. Il crate `regex` non ha il lookahead: il gruppo consuma il carattere seguente e `find_words` taglia la corrispondenza all'inizio del gruppo (la ricerca successiva riparte da lì). Dentro il termine non serve.
8. Un `*` iniziale o finale (con più di un pezzo nel termine) è un gruppo con nome (`lead`, `trail`), per la regola della punteggiatura di bordo.

## Dalla corrispondenza alla parola (`find_words`)

1. Ogni termine trova le sue corrispondenze (non sovrapposte, da sinistra); quelle vuote si saltano.
2. Punteggiatura con `*` di bordo: dalla parte catturata da `lead` si tolgono i segni di punteggiatura (categoria `P`) iniziali, da `trail` quelli finali: "c?rt*" su "carta," evidenzia "carta"; "*arta" su "«carta" evidenzia "carta". Quella catturata dentro la parola resta ("c*" su "c.a.r." evidenzia "c.a.r"); quella scritta nel termine non si tocca. Gli spazi ai bordi dell'evidenziato (possibili con un `*` vuoto accanto a uno spazio del termine) si tolgono. Se non resta nulla, la corrispondenza si scarta.
3. La corrispondenza si estende a sinistra e a destra fino agli spazi (White_Space Unicode) o ai bordi del testo. Le parole che si sovrappongono (più corrispondenze nella stessa parola, anche di termini diversi, o un termine con spazi che copre più parole) diventano un solo `WordMatch`, con tutti gli evidenziati (non vuoti, ordinati, disgiunti, non adiacenti) e l'insieme dei termini (`TermSet`).
4. Dai bordi della parola si toglie la punteggiatura (`P`), mai dentro una corrispondenza. I simboli (`S`: `$`, `+`, `<`) restano.

Tutti gli intervalli sono byte su confini UTF-8. `line` è 0-based; `line_range` esclude `\n` e `\r\n`. Il testo può avere più righe: cercare per blocchi di righe dà gli stessi risultati. Tempo lineare anche con parole lunghissime (la parola dell'ultima corrispondenza si riusa).

## Testo mostrato

- `word_text(max_chars)`: parola oltre 200 caratteri troncata; se la prima corrispondenza sta nei primi 200 si tiene l'inizio, altrimenti un pezzo centrato su di essa. Il "..." lo aggiunge chi mostra, secondo `truncated_start/end`.
- `snippet(max_chars)`: circa 120 caratteri della riga intorno alla parola, il posto avanzato da un lato va all'altro; se la parola è più lunga, centrato sulla prima corrispondenza.

## OR e AND

`FileAccumulator` trattiene i risultati di un file finché è finito e li scarta se manca un termine (AND); con OR li lascia partire a lotti. Un file dentro un archivio conta come file a sé. Nel motore (`textfile.rs`) oltre il limite dei risultati si trattengono solo i risultati che portano un termine nuovo.

## Location

`Line` (testo), `PageLine` (PDF), `Cell { sheet, cell }` (fogli), `Slide` (presentazioni), `Paragraph` (Word, ODT, RTF, EPUB), `Mail(part)` (e-mail).

## Test (fasi 1-3)

Partizioni e `proptest`: la parola contiene la corrispondenza, nessun a capo, confini UTF-8, blocchi = intero; test di linearità su parole e righe lunghissime.
