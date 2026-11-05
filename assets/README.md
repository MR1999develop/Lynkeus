# Icona di Lynkeus Searcher

Una lente d'ingrandimento (unico colore d'accento, blu) sopra un foglio di
testo. Disegno originale, generato da codice.

## Contenuto

- `src/genera_icona.py`: genera tutti i file (solo libreria standard).
- `icon.ico`: icona di Windows con 16, 24, 32, 48 e 256 px (PNG incorporati).
- `icon-256.png`: icona per Linux e per la finestra (`ViewportBuilder::with_icon`).
- `png/icon-<N>.png`: 16, 24, 32, 48, 64, 128 e 256 px.

## Rigenerare

Dalla radice del repository:

    python -I assets/src/genera_icona.py

I file generati sono deterministici e vanno versionati.

## Licenza

GPL-3.0-only, come il resto del progetto. Copyright (C) 2026 MR.
