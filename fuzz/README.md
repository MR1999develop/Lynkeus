# Fuzzing

Target di `cargo-fuzz` (libFuzzer) per i parser propri, il decodificatore del protocollo, il lettore XML, la lettura dei testi e la ricerca. Oltre al crash, dove indicato, un oracolo controllato con un'asserzione:

| Target | Cosa prova | Oracolo oltre al crash |
|---|---|---|
| `doc_piece_table` | piece table e FIB dei .doc | nessuno |
| `ppt_records` | scansione dei record dei .ppt | nessuno; il tempo (`-timeout`) per le catene di modifiche |
| `rtf` | tokenizer RTF | nessuno |
| `protocol_frame` | lettura dei frame del protocollo (richieste e risposte) | andata e ritorno: ogni messaggio letto, riscritto, si rilegge uguale |
| `find_words` | ricerca su termini e testo qualsiasi | gli invarianti di `crates/lynkeus-core/tests/proprieta.rs`, compreso "a blocchi di righe = intero" |
| `xml_reader` | lettore XML (pacchetti e capitoli XHTML) | chiusure corrispondenti alle aperture, due letture uguali |
| `text_sniff` | codifica e lettura a blocchi dei testi | blocchi piccoli = blocchi predefiniti, `first_line` giusto, UTF-8 riletto identico |

Ogni input che fa fallire un target si riduce (`cargo +nightly fuzz tmin`) e diventa un test normale nel crate del codice provato.

Il crate sta fuori dal workspace principale: `cargo xtask check` non lo compila. Serve la toolchain nightly e Linux.

```
cargo install cargo-fuzz
cd fuzz
cargo +nightly fuzz run rtf
```

Si cambia `rtf` con il nome del target; `cargo +nightly fuzz list` li elenca. Opzioni utili dopo `--`: `-max_total_time=300` per fermarsi dopo 300 s, `-max_len=65536` per limitare la lunghezza degli input. Come corpus iniziale si possono copiare i file di `fixtures/formati/` nella cartella `fuzz/corpus/<target>/`. Un input che fa fallire il target resta in `fuzz/artifacts/<target>/`; per riprodurlo: `cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<file>`.

`corpus/`, `artifacts/` e `coverage/` non si committano.
