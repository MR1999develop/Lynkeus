# Decisioni di architettura (ADR)

Un ADR registra una decisione difficile da cambiare. File: `docs/adr/NNNN-titolo.md`.

## Modello

```
# NNNN. Titolo

Stato: proposta | accettata | sostituita da NNNN

## Contesto
Il problema e i vincoli.

## Opzioni
Per ciascuna: descrizione e lati negativi.

## Decisione
Cosa si fa e perché.

## Conseguenze
Cosa diventa più facile e cosa più difficile.

## Come si verifica il rispetto
Test, controlli o comandi che falliscono se la decisione viene violata.
```

## ADR previsti

- Isolamento in processo dei decoder (fase 4).
- Libreria PDF: `pdf_oxide` o `pdf-extract`, dopo il prototipo (fase 4).
- Formato delle impostazioni (`settings.toml` proprio, versionato).
