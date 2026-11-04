# Fuzzing

Target di `cargo-fuzz` (libFuzzer) per i parser propri e il decodificatore del protocollo:

| Target | Cosa prova |
|---|---|
| `doc_piece_table` | piece table e FIB dei .doc |
| `ppt_records` | scansione dei record dei .ppt |
| `rtf` | tokenizer RTF |
| `protocol_frame` | lettura dei frame del protocollo (richieste e risposte) |

Il crate sta fuori dal workspace principale: `cargo xtask check` non lo compila. Serve la toolchain nightly e Linux.

```
cargo install cargo-fuzz
cd fuzz
cargo +nightly fuzz run rtf
```

Si cambia `rtf` con il nome del target; `cargo +nightly fuzz list` li elenca. Opzioni utili dopo `--`: `-max_total_time=300` per fermarsi dopo 300 s, `-max_len=65536` per limitare la lunghezza degli input. Come corpus iniziale si possono copiare i file di `fixtures/formati/` nella cartella `fuzz/corpus/<target>/`. Un input che fa fallire il target resta in `fuzz/artifacts/<target>/`; per riprodurlo: `cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<file>`.

`corpus/`, `artifacts/` e `coverage/` non si committano.
