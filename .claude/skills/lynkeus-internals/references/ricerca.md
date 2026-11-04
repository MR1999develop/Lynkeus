# Ricerca (lynkeus-core)

Crate puro, senza I/O. Ogni termine diventa una regex del crate `regex` (tempo lineare); la regex non arriva mai grezza dall'utente.

## Compilazione di un termine

1. Il termine è portato in NFC.
2. I letterali sono escapati.
3. `*` → `\S*` (greedy); `?` → `\S`. Con i jolly spenti, `*` e `?` sono letterali.
4. Spazi → `[^\S\r\n]+`: nessuna corrispondenza attraversa una riga.
5. Maiuscole ignorate → flag `(?i)`; distinte → nessun flag.
6. Accenti ignorati: ogni lettera diventa la classe delle sue varianti accentate più `\p{Mn}*`, con una tabella costruita una volta dalla decomposizione Unicode (`LazyLock`).
7. Accenti distinti: ogni lettera accentata accetta anche la forma decomposta (NFD). Il testo non si normalizza, quindi le posizioni restano quelle originali.

## Validazione

Termine vuoto, o solo `*`/`?` (con i jolly accesi): errore mostrato sotto il campo. Tipo `Term` valido per costruzione.

## Parola e righe dei risultati

- Dalla corrispondenza si estende fino agli spazi (White_Space Unicode) a sinistra e a destra; poi si tolgono i segni di punteggiatura ai bordi, mai dentro la corrispondenza.
- Più corrispondenze nella stessa parola (anche di termini diversi): una sola riga con tutti gli evidenziati.
- Parole oltre 200 caratteri: troncate con "…". Contesto: circa 120 caratteri intorno alla parola.

## OR e AND

`FileAccumulator` trattiene i risultati di un file finché è finito e li scarta se manca un termine (AND). Un file dentro un archivio conta come file a sé.

## Location

`Line` (testo), `PageLine` (PDF), `Cell { sheet, cell }` (fogli), `Slide` (presentazioni), `Paragraph` (Word, ODT, RTF, EPUB), `Mail(part)` (e-mail).

## Test previsti (fase 1)

Partizioni e `proptest`: la parola contiene la corrispondenza, nessun a capo, confini UTF-8, blocchi = intero.
