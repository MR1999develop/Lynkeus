# Piano di test

## Scopo

Verificare che Lynkeus Searcher funzioni come descritto in `docs/specifica.md` e che i problemi trovati in `docs/analisi-conformita.md` (commit `9788999`, 2026-11-05) siano rilevati da un test e, una volta corretti, non tornino. Il piano ha cinque livelli:

1. test automatici già presenti;
2. test automatici da scrivere;
3. prestazioni, con metodo e soglie;
4. test manuali;
5. usabilità con tre persone.

Gli ID dei problemi (A-01, B-01, C-01, D-01 e i plausibili A-P1...) sono quelli dell'analisi.

## Ambienti

| Sigla | Ambiente | Uso |
|---|---|---|
| W10 | Windows 10 22H2, utente standard | livelli 1-5; test del clic destro "diretto"; VM pulita per il redistributable |
| W11 | Windows 11 | livelli 1, 4, 5; clic destro con "Mostra altre opzioni"; Narrator |
| UBU | Ubuntu LTS con GNOME (Nautilus, Orca) | livelli 1 (incluso il fuzzing), 4, 5 |

La CI esegue `cargo xtask check` su Windows e Ubuntu; i controlli manuali su Linux si fanno su una macchina reale o una VM, perché in locale c'è solo il controllo di compilazione.

## Prerequisiti

- Toolchain Rust dichiarata in `rust-toolchain.toml` (con i target Windows e Linux); per il fuzzing anche `cargo +nightly` e `cargo install cargo-fuzz`, solo su Linux.
- LibreOffice: serve solo per rigenerare i file di prova (`python -I fixtures/src/genera_demo.py`, `python -I fixtures/src/genera_formati.py`); non serve per eseguire i test.
- Windows: Process Explorer e `accesschk` (Sysinternals) per i test M-18, M-19, M-21; `dumpbin` (Build Tools di Visual Studio) per M-33; un secondo utente locale per `runas`.
- Ubuntu: `sha256sum`, Nautilus, Orca.
- Per le misure: modalità release, alimentazione di rete, nessun'altra attività pesante.

## Dati di prova

- `fixtures/demo/`: sei cartelle (`ricette`, `ufficio`, `musica`, `viaggi`, `legacy`, `binario`). La ricerca di "ar" (senza distinguere maiuscole e accenti) trova solo le parole elencate in `fixtures/README.md`: `calamari`, `l'aragosta`, `Comprare`, `carta`, `Suonare`, `chitarra`, `d'arte`, `ARMADIO`, `carità`, `carta`.
- `fixtures/formati/`: un file per ogni formato (txt, gz, tar, tar.gz, 7z, zip annidato, doc, xls, ppt, docx, xlsx, pptx, odt, ods, odp, rtf, eml, epub, pdf, file protetti da password).
- `fixtures/malevoli/`: `annidato-profondo.zip`, `bomba.zip`, `entita.docx`, `immagine-finta.pdf`, `percorsi.zip`, `riga-lunga.txt`, `troncato.pdf`.
- Rigenerazione: `python -I fixtures/src/genera_demo.py`, `genera_formati.py`, `genera_pdf_protetto.py` (dettagli e contenuti attesi in `fixtures/README.md`). I contenuti non contengono dati personali.
- File da creare per il livello 2 (generatore in `fixtures/src/`, riproducibili): `malevoli/sparso.xlsx` (A1 e XFD1048576), `malevoli/rtf-destinazioni.rtf`, `malevoli/slide-ripetute.pptx` (10.000 `sldId` uguali), `malevoli/zip-imbottito.zip`, tar con file di 20.000 righe per le misure.
- Pacchetto: `cargo xtask dist` produce in `dist/` lo zip e `SHA256SUMS`; i test M-xx sull'eseguibile usano quello estratto dallo zip, non `target/`.

## Livello 1: automatici già presenti

Comandi (dalla radice del repository):

| Comando | Cosa fa |
|---|---|
| `cargo xtask check` | fmt, clippy `-Dwarnings`, tutti i test, `cargo deny check` |
| `cargo xtask check-linux` | compilazione per il target Linux |
| `cargo xtask dist` | build di rilascio, verifiche (mitigazioni, scansione dell'exe), zip e `SHA256SUMS` |
| `cargo test --release -p lynkeus-app --test accettazione` | accettazione end-to-end con il processo estrattore vero (17 test) |
| `cargo test --release -p lynkeus-app --test interfaccia` | kittest sulla finestra (6 test) |
| `cargo test -p lynkeus-engine` | motore: scansione, pool, cache, documenti, processi |
| `cargo test -p lynkeus-extract` | formati, archivi, voci, protocollo |
| `cargo test -p lynkeus-core` | ricerca pura e proprietà |
| `cargo test --release -p lynkeus-app --lib sort_200k_timing -- --ignored --nocapture` | misura dell'ordinamento di 200.000 risultati (obiettivo nel codice: meno di ~50 ms per colonna) |
| `cargo test -p <crate> -- --list` | elenco dei test di un crate |
| `cd fuzz; cargo +nightly fuzz run <target> -- -max_total_time=300` | fuzzing (Linux); target: `doc_piece_table`, `ppt_records`, `rtf`, `protocol_frame` (vedi `fuzz/README.md`) |

Nota: i test `#[ignore]` di `context_menu.rs` e `windows_ffi.rs` sono figli di altri test e non si lanciano a mano; l'unico `#[ignore]` di misura è `sort_200k_timing`.

Copertura per area (nomi verificati nel codice):

| Area | Suite o file | Test principali |
|---|---|---|
| A core | crates/lynkeus-core/tests/proprieta.rs | `word_contains_matches_and_offsets_are_valid`, `blocks_of_lines_give_the_same_results`, `literal_wildcards_are_a_literal_search`, `nfc_and_nfd_text_give_the_same_words` |
| A estrattori | crates/lynkeus-extract/tests/formats.rs | `encrypted_doc_is_a_warning`, `truncated_office_97_files_are_corrupt`, `rtf_too_deep_is_corrupt`, `epub_doctype_entities_and_bad_chapters`, `tgz_and_gz_with_tar_content`, `gz_bomb_hits_ratio_limit`, `archives_nest_inside_zip_with_depth_limit`, `sevenz_limits_and_damage` |
| A estrattori | crates/lynkeus-extract/tests/fixtures.rs, archives.rs, voci.rs | `zip_bomb_hits_ratio_limit`, `total_size_counts_all_levels`, `docx_three_levels_down_is_byte_identical`, `entry_partitions` |
| A protocollo | crates/lynkeus-extract/src/protocol.rs | `random_bytes_never_panic`, `every_truncation_is_detected` |
| A parser | crates/lynkeus-extract/src/extract/ppt.rs, archive.rs | `never_panics_on_arbitrary_bytes` (anche word97, rtf), `panic_inside_a_nested_archive_keeps_the_labels` |
| B scansione | crates/lynkeus-engine/tests/scansione.rs | `the_result_limit_is_exact_across_threads`, `mixed_duplicate_and_nested_roots_read_each_file_once`, `links_and_junctions_are_not_followed_below_a_root`, `cancel_during_the_search`, `or_and_and_between_two_terms` |
| B documenti e pool | crates/lynkeus-engine/tests/documenti.rs | `cancel_stops_a_busy_worker_within_one_second` (instabile sotto carico, B-01/B-13), `interrupted_cache_writes_are_ignored_and_the_document_is_read_again`, `misbehaving_workers_are_replaced_and_reported` |
| B processi | crates/lynkeus-engine/tests/processi.rs | `closing_stdin_stops_a_worker_while_a_sibling_lives`, `worker_inherits_only_the_standard_descriptors`, `an_orphaned_worker_exits_when_its_parent_dies` (solo figlio inattivo) |
| B cache | crates/lynkeus-engine/src/cache.rs, textfile.rs | `corrupt_entries_are_ignored_and_removed`, `blocks_equal_whole` |
| B demo | crates/lynkeus-engine/tests/ricerca_demo.rs | `ar_in_the_text_files_of_demo`, `stops_at_the_result_limit`, `missing_root_becomes_a_warning_and_the_others_are_searched` |
| C accettazione | crates/lynkeus-app/tests/accettazione.rs | `ar_in_demo_finds_the_expected_words`, `the_second_search_reads_documents_from_the_cache`, `preview_of_a_pdf_inside_a_zip_shows_the_nearby_units`, `archive_entries_open_from_a_private_copy`, `phrase_with_a_space_finds_the_whole_phrase_in_docx`, `and_terms_in_different_files_finds_nothing`, `accents_ignored_both_spellings_find_both_forms`, `malicious_files_end_with_the_expected_warnings_and_write_nothing_outside` |
| C interfaccia | crates/lynkeus-app/tests/interfaccia.rs | `ar_finds_aragosta_through_the_window`, `clicking_aragosta_opens_the_preview_and_ctrl_c_copies_the_row`, `empty_term_shows_the_error_under_the_field`, `the_window_fits_1024_by_600` (solo stato vuoto), `language_menu_switches_the_interface_to_english` |
| C libreria | crates/lynkeus-app/src | `text_has_at_least_4_5_to_1` (theme.rs), `both_languages_have_the_same_ids` (i18n.rs), `formulas_are_neutralized_in_every_format`, `csv_round_trip_any_cells` (export.rs), `shortcuts_are_unique_and_specific_first` (ui/commands.rs), `empty_state_partitions` (app.rs, fissa il comportamento sbagliato di C-10) |
| D piattaforma | crates/lynkeus-app/src/platform/windows_ffi.rs | `private_dirs_have_an_explicit_protected_dacl`, `an_existing_dir_open_to_others_is_refused`, `closing_the_job_kills_the_child`, `the_child_sees_only_system_root`, `pipe_round_trip_with_timeouts`, `a_pipe_name_already_in_use_is_refused` |
| D rilascio | xtask (`cargo xtask dist`) | verifica delle mitigazioni (oggi passa senza `dumpbin`, D-07), scansione dell'exe per nome utente e percorsi, `SHA256SUMS` |

Esito atteso: tutto verde a macchina scarica. Con la CPU satura `cancel_stops_a_busy_worker_within_one_second` fallisce (B-01).

## Livello 2: automatici da scrivere

Tipo: U unitario, P proprietà, I integrazione, K kittest, S sicurezza sull'artefatto, F fuzzing. "Oggi" dice se il test manca o fallirebbe (comportamento attuale sbagliato).

### Area A

| ID | Nome | Tipo | Cosa verifica | Dati | Esito atteso | Rileva | Oggi |
|---|---|---|---|---|---|---|---|
| T-A01 | `term_finds_itself_in_every_mode` | P (core) | Ogni stringa NFC senza spazi (latino accentato, Hangul, greco, CJK di compatibilità) trova se stessa nelle 8 combinazioni di opzioni | generati | sempre trovata | A-02 | fallirebbe (Hangul con accenti ignorati) |
| T-A02 | `nfc_and_nfd_text_give_the_same_words` esteso | P (core) | Estendere il test esistente a Hangul e vietnamita | generati | stesse parole | A-02 | fallirebbe per Hangul |
| T-A03 | `terms_normalizing_to_wildcards_are_rejected` | U (core) | "*\u{301}", "\u{301}", "?\u{301}" con accenti ignorati sono errore | stringhe | errore di termine | A-03 | manca |
| T-A04 | `find_words_matches_naive_oracle` | P differenziale (core) | Confronto con un matcher ingenuo per jolly, spazi, punteggiatura | generati | risultati uguali | A-03, A-02 | manca |
| T-A05 | `paragraph_marks_inside_skipped_destinations` | U (rtf) | `\footnote`, `\*\annotation`, `\header` con `\par` danno un solo paragrafo "La cena di aragosta" | `malevoli/rtf-destinazioni.rtf` o stringa | frase trovata, numeri di paragrafo corretti | A-04 | fallirebbe |
| T-A06 | `gz_with_tar_content_has_tar_gz_limits` | I (extract) | Lo stesso tar come `x.gz` e `x.tar.gz` con limiti piccoli dà eventi uguali | tar generato | eventi uguali | A-05 | fallirebbe |
| T-A07 | `epub_respects_depth_and_entry_budget` | I (extract) | `libro.epub` in uno zip con `max_depth` 1 dà `ArchiveDepth`; più EPUB oltre `max_entries` dà `ArchiveEntries` | `formati/libro.epub` | avvisi attesi | A-06 | fallirebbe |
| T-A08 | `sparse_xlsx_is_a_limit_not_a_crash` | I (pool, processo vero) | `sparso.xlsx` (A1 + XFD1048576) in uno zip seguito da `nota.txt`: avviso per l'xlsx, `nota.txt` letta, processo non terminato | `malevoli/sparso.xlsx` | avviso di limite e voce successiva letta | A-01, A-P4 | fallirebbe (abort) |
| T-A09 | `non_utf8_name_keeps_its_format` | U (unix) | `for_path("rel\xff.docx")` dà Docx | nome non UTF-8 | Docx | A-07 | fallirebbe su Linux |
| T-A10 | `labels_neutralize_separators_and_zero_width` | U | U+2028, U+200B, U+FEFF nelle etichette diventano U+FFFD | stringhe | U+FFFD; una sola funzione | A-09 | fallirebbe |
| T-A11 | `worker_refuses_missing_memory_limit` | U | `from_args([])` è un errore | nessuno | errore | A-08 | fallirebbe |
| T-A12 | `pptx_repeated_slide_ids_read_once` | U | 10.000 `sldId` uguali: una sola lettura | `malevoli/slide-ripetute.pptx` | una lettura, tempo breve | A-P2 | manca |
| T-A13 | fuzzing con oracoli | F | `find_words` con gli invarianti di `proprieta.rs` come asserzioni, `protocol_frame` con andata e ritorno, `xml_reader`, `text_sniff`; ogni input che fallisce diventa test normale; esiti nel threat model | corpus da `fixtures/formati/` | nessun fallimento in 10 minuti per target | A-P5 | oggi 4 target con il solo oracolo del crash |
| T-A14 | `padded_zip_hits_ratio_limit` (derivato) | I (extract) | Zip con voce imbottita: il rapporto 200:1 si calcola anche sul dichiarato non compresso reale | `malevoli/zip-imbottito.zip` | `LimitExceeded` | A-P1 | manca |

### Area B

| ID | Nome | Tipo | Cosa verifica | Dati | Esito atteso | Rileva | Oggi |
|---|---|---|---|---|---|---|---|
| T-B01 | `cancel_stops_a_busy_worker_within_one_second` stabilizzato | I | Il worker di test segnala "richiesta ricevuta" invece di `sleep(200 ms)`; budget in una costante | worker "lento" | Annulla < 1 s anche con CPU satura | B-01, B-13 | fallirebbe sotto carico |
| T-B02 | `unit_over_1_mib_is_a_protocol_violation` | I | Worker di test che invia un'unità da 2 MiB | worker di test | avviso Corrupt, processo sostituito | B-03 | fallirebbe |
| T-B03 | `flooding_worker_times_out_within_budget` | I | Unità valide senza fine (previsto dal TM 4.7 D) | worker di test | Timeout entro la scadenza; picco di memoria del padre entro il budget | B-03, B-P3 | manca |
| T-B04 | `worker_speaking_while_idle_is_replaced` | I | Il figlio scrive senza richiesta | worker di test | processo non riusato, avviso | B-03 | manca |
| T-B05 | `entry_data_out_of_sequence_is_rejected` | I | `EntryData` fuori sequenza | worker di test | processo non riusato, avviso | B-03 | manca |
| T-B06 | `busy_orphan_exits_with_its_parent` | I (Linux) | Padre intermedio ucciso mentre il figlio è su "lento" | worker "lento" | figlio uscito entro 5 s | B-04 | fallirebbe |
| T-B07 | `cache_clear_during_search_is_safe` | I | `clear()` a metà ricerca | demo | Annulla < 1 s, nessuna voce corrotta letta | B-07 | manca |
| T-B08 | `two_instances_share_the_cache_safely` | I | Due `TextCache` sulla stessa cartella con put/get/sfratto concorrenti | documenti demo | ogni `get` dà eventi validi o `None`; disco entro il massimo | B-P4 | manca |
| T-B09 | `failed_cache_put_is_counted` | U | Cartella della cache sostituita da un file | cartella finta | errore contato e registrato una volta | B-08 | fallirebbe |
| T-B10 | `anteprima_senza_cache_si_ferma_a_finestra_completa` (derivato) | I | `UnitWindow` con cache spenta o voce > 64 MiB | worker di test | `Err(Stopped)`, processo libero entro 1 s | B-10 | fallirebbe |

I problemi B-05, B-06, B-09, B-11, B-12 sono refactoring o misure: sono coperti dai test esistenti (`the_result_limit_is_exact_across_threads`, `blocks_equal_whole`) e dalle misure P-04 e P-05; B-05 si verifica con un controllo delle dipendenze tra moduli (nessun ciclo) da aggiungere a `xtask`.

### Area C

| ID | Nome | Tipo | Cosa verifica | Dati | Esito atteso | Rileva | Oggi |
|---|---|---|---|---|---|---|---|
| T-C01 | `esc_closes_menu_while_searching` | K | Menu aperto durante una ricerca, Esc | ricerca lunga | menu chiuso, ricerca ancora in corso | C-01, C-P2 | fallirebbe |
| T-C02 | `form_error_opens_filters` | K | "Solo queste" con elenco vuoto, Filtri chiusi, F5 | impostazioni | Filtri aperti, messaggio, focus sul campo | C-04 | fallirebbe |
| T-C03 | `form_error_with_hidden_panel` | K | Pannello nascosto (F9), termine non valido, F5 | impostazioni | il pannello ricompare | C-04 | fallirebbe |
| T-C04 | `row_selection_is_exposed` | K | Nodo AccessKit con `selected=true`; freccia giù sposta il focus | risultati demo | stato selezionato e focus sulla riga | C-03 | fallirebbe |
| T-C05 | `numbers_grouped_per_language` | U | "200.000" in italiano, "200,000" in inglese | numeri | formato per lingua | C-05 | fallirebbe |
| T-C06 | `plural_skipped_binary` | U | "1 saltato", "2 saltati", "1 binario" | numeri | plurali corretti in entrambe le lingue | C-12 | fallirebbe |
| T-C07 | `selection_contrast` | U | Contrasto della riga selezionata ≥ 3:1 o segno sulla riga, temi chiaro e scuro | token del tema | soglia rispettata | C-02 | fallirebbe (1,07-1,13:1) |
| T-C08 | `progress_indeterminate_while_walking` | U | Con totale ignoto, barra indeterminata con i conteggi | stato di avanzamento | nessuna percentuale | C-06 | fallirebbe |
| T-C09 | `the_window_fits_1024_by_600` esteso | K | Con risultati, anteprima, banner e messaggio, in italiano e inglese: Cerca visibile, almeno 5 righe, nessun testo tagliato | demo | tutto visibile | regola 34, regola 45 | manca (oggi solo stato vuoto) |
| T-C10 | `empty_state_partitions` corretto | U | Stati vuoti distinti: nessun risultato, annullata, interrotta | app | messaggio proprio per ogni stato | C-10 | fallirebbe (fissa il comportamento sbagliato) |
| T-C11 | `focus_ring_contrast` (derivato) | U | Contrasto del bordo di focus sul pulsante primario e sulle intestazioni ≥ 3:1 | token del tema | soglia rispettata | C-09 | fallirebbe (1,00:1) |
| T-C12 | `empty_term_row_is_ignored_when_another_is_valid` (derivato) | U | Una riga vuota accanto a una valida | form | ricerca avviata | C-16 | fallirebbe |
| T-C13 | `settings_newer_version_is_reported` (derivato) | U | `settings.toml` più recente o illeggibile | file di prova | messaggio visibile; nessun `sync_all` sul thread UI | C-11 | fallirebbe |

C-07 e C-08 sono coperti dalle misure P-07 e P-08. Per C-13, C-14, C-15, C-17, C-18 e i plausibili C-P1..C-P6 non c'è test automatico proposto dai rapporti: si verificano con M-10, M-11, M-12, M-13 e vanno promossi a test automatici se emerge un difetto.

### Area D

| ID | Nome | Tipo | Cosa verifica | Dati | Esito atteso | Rileva | Oggi |
|---|---|---|---|---|---|---|---|
| T-D01 | `association_launchers_are_executable` | U | Le estensioni di D-01, anche maiuscole e con ". " finale, danno `OpenPlan::Executable`; `opened_extension` → bin | elenco di estensioni | tutte eseguibili | D-01 | fallirebbe |
| T-D02 | `extracted_entry_keeps_mark_of_the_web` | I (Windows) | Zip con `Zone.Identifier` ZoneId=3 → `extract_to_private_temp` → `Get-Content <file> -Stream Zone.Identifier` | zip con stream | contiene ZoneId=3 | D-02 | fallirebbe |
| T-D03 | `static_imports_ignore_the_exe_folder` | S (artefatto, in `dist`) | Copia dell'exe con `dwmapi.dll` e `VCRUNTIME140.dll` finte accanto; `--extract-worker --memory-limit-bytes 1073741824` con stdin chiuso | exe del pacchetto | uscita diversa da 0xC000012F | D-03, D-04 | fallirebbe |
| T-D04 | `pe_mitigations_are_present` | U (xtask) | Parser PE: CFG, CETCOMPAT, `DependentLoadFlags` 0x800, nessun `VCRUNTIME140.dll` tra gli import; fallisce se mancano | exe di release | tutte presenti | D-03, D-04, D-07 | fallirebbe (e `dumpbin` assente passa) |
| T-D05 | `window_state_file_is_inside_the_config_dir` | U | `persistence_path` = `config\window.ron`; con `None` `persist_window` è false | `native_options` | percorso a file | D-05 | fallirebbe |
| T-D06 | `refresh_with_an_unsafe_exe_removes_our_entry` | U | `FakeStore` in stato Current, cartella dell'exe scrivibile | fake | azioni = ["remove"] | D-06 | fallirebbe |
| T-D07 | `child_inherits_only_stdio` | I (Windows) | Il padre passa al figlio il valore di un handle ereditabile; nel figlio `GetHandleInformation` fallisce | handle di prova | il figlio non lo vede | regola 16 (D, punti di forza) | manca |
| T-D08 | `a_slow_client_does_not_block_the_next` | I | Client lento sulla pipe dell'istanza unica (previsto dal TM) | pipe | il client successivo è servito | TM 4.8 A | manca |
| T-D09 | `twenty_accept_errors_stop_the_server` | I | Venti errori di accept consecutivi (previsto dal TM) | pipe finta | il server si ferma e lo dichiara | TM 4.8 A | manca |
| T-D10 | `job_limits_active_process_and_ui` (derivato) | I (Windows) | Il figlio prova ad avviare `cmd.exe` e a leggere gli appunti | worker di test | avvio negato | D-09 | fallirebbe |
| T-D11 | `bare_paths_are_added_like_dash_dash_add` (derivato) | U | `lynkeus-searcher.exe C:\cartella` | argomenti | trattato come `--add` | D-10 | fallirebbe (uscita 2) |
| T-D12 | `deny_exceptions_expire` (derivato) | U (xtask) | Ogni eccezione di `deny.toml` ha una data; `cargo xtask check` fallisce se scaduta | `deny.toml` | fallimento dopo la data | D-08 | manca |
| T-D13 | `percent_in_exe_path_is_refused` (derivato) | U | Registrazione del clic destro con `%` nel percorso dell'exe | percorso finto | rifiutata | D-P2 | fallirebbe |

D-P1 (UB formale con `&OVERLAPPED`) si verifica per revisione e, dopo la correzione, con Miri sui test che non toccano il kernel.

Totale livello 2: 14 (A) + 10 (B) + 13 (C) + 13 (D) = 50 test, di cui 9 derivati dall'analisi (non proposti dai revisori).

## Livello 3: prestazioni

### Metodo comune

- Build di rilascio. Ambiente annotato in ogni riga: CPU e numero di core, tipo di disco, antivirus (attivo, con esclusione, spento), versione di Windows o Ubuntu, commit.
- Una esecuzione scartata, poi 15 ripetizioni a caldo (30 per le misure sotto carico), e una misura a freddo separata (cache di sistema svuotata o riavvio).
- Si riportano p50, p95 e massimo, mai la sola media; per la memoria il picco di memoria privata (Process Explorer, "Private Bytes", o `GetProcessMemoryInfo`); si annota il fattore limitante (CPU, disco, memoria).
- Una differenza conta solo se supera il rumore (distanza tra ripetizioni) e si ripete.
- Le soglie sono proposte e vanno confermate dopo la prima serie completa.

### Misure

| ID | Misura | Metodo | Soglia attesa | Rileva |
|---|---|---|---|---|
| P-01 | Scansione di 100.000 file di testo ("ar", cartella generata) | Esecuzione scartata, 15 a caldo, 1 a freddo; tempo fino a "Finito" | caldo p95 ≤ 1,5 s; il freddo si riporta senza soglia | regola 31 |
| P-02 | Annulla sotto carico | 4 processi "lento" più CPU satura, 30 ripetizioni; tempo da Annulla a `Finished` | massimo < 1 s | B-01 |
| P-03 | `p03_and_retained_hits_memory` | 16 file con 300.000 "il", AND con "zzz"; 5 ripetizioni; picco di memoria privata | nessun risultato; massimo sotto il budget di 256 MiB | B-02 |
| P-04 | Filtro di 32 estensioni su 100.000 nomi | `criterion` con `black_box`, prima e dopo il filtro compilato | nessuna allocazione nel ciclo; miglioramento oltre il rumore | B-11 |
| P-05 | Cache contro estrazione sui documenti demo | p50 e p95 della seconda ricerca contro la prima | la seconda più veloce, p95 riportato | B-12 |
| P-06 | `sort_200k_timing` | `cargo test --release -p lynkeus-app --lib sort_200k_timing -- --ignored --nocapture` | < 50 ms per colonna (obiettivo del codice) | regola 31 |
| P-07 | `step_target_200k` | Test `#[ignore]` da scrivere, 200.000 righe, release | < 1 ms per chiamata | C-08 |
| P-08 | `copy_200k` | Ctrl+A, Ctrl+C con 200.000 righe; durata del blocco del thread UI e picco di memoria | thread UI libero (nessun frame oltre 100 ms) | C-07 |
| P-09 | Primo frame e risposta a un tasto | Tempo dal tasto al ridisegno con 200.000 risultati | < 100 ms (regola 40) | C-07, C-08 |

### Risultati

| ID | Data | Commit | Ambiente | Ripetizioni | p50 | p95 | Massimo | Memoria di picco | Esito | Note |
|---|---|---|---|---|---|---|---|---|---|---|
| P-01 | 2026-11-05 | 9788999 | non annotato | non annotate | 0,8-1 s a caldo | non misurato | non misurato | non misurato | indicativo | verifica finale della fase 7: mancano distribuzione e ambiente |
| P-06 | 2026-11-05 | 9788999 | non annotato | non annotate | 2-52 ms | non misurato | non misurato | non misurato | indicativo | intervallo per colonna; mancano distribuzione e ambiente |
| P-02 |  |  |  |  |  |  |  |  |  | il revisore ha misurato 1,08 / 1,21 / 1,19 s con CPU satura (3 prove, 12 core) |
| P-03 |  |  |  |  |  |  |  |  |  | stima del revisore: ~1,3 GB, non misurata |
| P-04, P-05, P-07, P-08, P-09 |  |  |  |  |  |  |  |  |  |  |

## Livello 4: manuali

Piattaforme: W10, W11, UBU. Esito: OK, KO o N/A; il KO si accompagna a una nota e, secondo la regola in fondo, a un test automatico.

### Ricerca e risultati

| ID | Piattaforma | Passi | Esito atteso | Esito | Note |
|---|---|---|---|---|---|
| M-01 | W10, W11, UBU | 1. Estrarre lo zip del pacchetto (`dist/`) in una cartella nuova. 2. Avviare `lynkeus-searcher` dal pacchetto. 3. Aggiungere `fixtures/demo` (Ctrl+O). 4. Cercare "ar" (Ctrl+F, Invio). | Compaiono solo le parole elencate in `fixtures/README.md`; barra di stato con conteggi; nessun avviso inatteso; `immagine.png` saltata e contata | | |
| M-02 | W10, W11, UBU | Provare i filtri su `fixtures/demo` e `fixtures/formati`: estensioni incluse/escluse, dimensione massima, file nascosti, cartelle da saltare, "Solo queste", maiuscole, accenti, AND/OR, frase con spazio, jolly `?` e `*` | Ogni filtro cambia l'esito come descritto nella specifica; un errore nei Filtri chiusi apre il pannello (C-04) | | |
| M-03 | W10, W11, UBU | Cercare in `fixtures/formati` una parola presente in tutti i formati; avviare e premere Esc (o Annulla) a metà; ripetere con un albero grande (100.000 file) | Annulla risponde entro 1 s; la barra dice "annullata", non "nessuna parola trovata" (C-10); i processi estrattori spariscono | | |
| M-04 | W10, W11, UBU | Selezionare un risultato: anteprima con il contesto. Doppio clic o Invio: apertura nel programma associato. Ripetere per una voce in `fixtures/demo/musica/archivio.zip` (`elenco.pdf`) e per un capitolo di `fixtures/formati/libro.epub` | L'anteprima mostra le unità vicine; la voce dello zip si apre da una copia privata; l'EPUB mostra il capitolo giusto | | |
| M-05 | W10, W11, UBU | Esportare i risultati in CSV (virgola) e TXT/TSV; aprire il CSV in Excel e in LibreOffice Calc; verificare con un termine che produce celle che iniziano con `=`, `+`, `-`, `@` | Colonne corrette, accenti corretti, nessuna formula eseguita (neutralizzate); in inglese la virgola separa bene le colonne | | |
| M-06 | W10, W11 | Cercare in `fixtures/malevoli` | Avvisi attesi (bomba, annidamento, percorsi, entità, PDF finto, PDF troncato, riga lunga); nessun file scritto fuori dalla cartella privata; l'app resta reattiva | | |

### Tastiera, lettori di schermo, aspetto

| ID | Piattaforma | Passi | Esito atteso | Esito | Note |
|---|---|---|---|---|---|
| M-07 | W10, W11, UBU | Solo tastiera: Ctrl+O, Ctrl+F, termine, Invio; Esc annulla; frecce e Invio aprono; Ctrl+Maiusc+R mostra nella cartella; F3 e Maiusc+F3; Ctrl+Maiusc+E poi Esc | Tutto raggiungibile senza mouse; focus sempre visibile; nessuna trappola nella tabella | | |
| M-08 | W10, W11, UBU | Aprire il menu "Ricerca" durante una ricerca e premere Esc; aprire il menu contestuale su una riga e premere Esc | Si chiude il menu, la ricerca prosegue (C-01) | | |
| M-09 | W11 (Narrator, Win+Ctrl+Invio), UBU (Orca, Super+Alt+S) | Tab dal menu; ogni campo letto con il nome; cercare "ar" in `fixtures/demo`; freccia giù nella tabella; Ctrl+C | La riga è letta con "selezionato"; Ctrl+C annuncia la copia; i messaggi di stato sono annunciati o il limite è annotato (C-03, C-P3) | | |
| M-10 | W10, W11, UBU | Con lettore di schermo, scorrere i pulsanti dell'anteprima | I nomi dei comandi non sono ambigui (C-13) | | |
| M-11 | W10, W11, UBU | Tema chiaro, tema scuro, contrasto elevato di Windows (Alt+Maiusc+Stamp): righe selezionate, focus su Cerca e sulle intestazioni, messaggi di errore | Riga selezionata distinguibile anche in scala di grigi; focus visibile ovunque; errori leggibili (C-02, C-09, C-P4) | | |
| M-12 | W10, W11, UBU | Scorrere con Tab il pannello di ricerca | Ordine: luoghi, termini, Cerca (C-15); il Tab non passa da ogni cella (C-P1) | | |
| M-13 | W10, W11, UBU | Cambiare l'ordinamento durante la ricerca; a fine ricerca controllare la riga del cursore; "Ripristina i filtri predefiniti" | La riga del cursore resta in vista (C-17); il ripristino offre Annulla (C-14) | | |
| M-14 | W10, W11, UBU | Finestra 1024×600 al 100%, in italiano e in inglese, con risultati, anteprima, banner e messaggio | Cerca visibile, almeno 5 righe, nessun testo tagliato | | |
| M-15 | W10, W11, UBU | Lingua inglese: scorrere menu, tooltip, finestre di dialogo, messaggi di errore | Nessun testo italiano; numeri "200,000"; plurali corretti ("1 skipped") | | |
| M-16 | W10, W11, UBU | Chiudere con la finestra spostata e ridimensionata, riaprire | Posizione e dimensione ricordate (D-05); impostazioni, cartelle di apertura e di salvataggio ricordate | | |
| M-17 | W10, W11 | Con 200.000 risultati: Ctrl+A, Ctrl+C; passare il mouse sulle righe | L'interfaccia non si blocca oltre 0,1 s (C-07, C-08) | | |

### Sistema e sicurezza

Percorsi con `%LOCALAPPDATA%\Lynkeus Searcher\` (cartelle `data`, `cache`).

| ID | Piattaforma | Passi | Esito atteso | Esito | Note |
|---|---|---|---|---|---|
| M-18 | W10, W11 | 1. `icacls "$env:LOCALAPPDATA\Lynkeus Searcher\data"`. 2. `(Get-Acl "$env:LOCALAPPDATA\Lynkeus Searcher\cache").AreAccessRulesProtected`. 3. Chiudere l'app; `icacls "$env:LOCALAPPDATA\Lynkeus Searcher\cache" /grant *S-1-5-32-545:(OI)(CI)M`; riavviare. | Solo utente e SYSTEM con `(OI)(CI)(F)`, protetta; al punto 3 compare la barra "cartelle non private" e la DACL resta invariata (B-09) | | |
| M-19 | W10, W11 | Avviare una ricerca su PDF; in Process Explorer aprire i processi `--extract-worker`: schede Job, Environment, Handles; poi `taskkill /F /IM lynkeus-searcher.exe` sul padre | Job con limite di 1 GiB e kill on close; Environment con la sola `SystemRoot`; Handles solo pipe; dopo il kill i figli spariscono; il job limita i processi attivi (D-09) | | |
| M-20 | UBU | Avviare una ricerca con un file "lento"; `kill -9` sul padre; `pgrep -f extract-worker` entro 5 s | Nessun figlio rimasto (B-04) | | |
| M-21 | W10, W11 | 1. Avviare con `--add C:\Windows`. 2. `[IO.Directory]::GetFiles('\\.\pipe\') -like '*lynkeus-searcher-*'`. 3. `accesschk.exe -v \pipe\<nome>`. 4. `runas /user:<altro> powershell` e provare ad aprire la pipe. | Una sola pipe; accesso solo all'utente; per l'altro utente accesso negato | | |
| M-22 | W10 | Clic destro "diretto" su: cartella con spazi, sfondo di `C:\`, file `-r`, `a&b.txt`, `100%.txt`, 20 file selezionati; in regedit `HKCU\Software\Classes\Directory\Background\shell\LynkeusSearcher\command` = `"<exe>" --add "%V\."`; spostare l'exe e riavviare; spegnere l'opzione | Percorso esatto aggiunto; 20 file → una sola finestra; exe spostato → voce riscritta; opzione spenta → tre chiavi tolte; una cartella trascinata sull'icona dell'exe dà un avviso visibile (D-10) | | |
| M-23 | W11 | Come M-22, con "Mostra altre opzioni"; cartella con `%` nel percorso dell'exe (copia in `C:\Strumenti\50%L\`) | Funziona come su W10; il `%` nel percorso dell'exe è rifiutato con messaggio (D-P2) | | |
| M-24 | UBU | `ls -l ~/.local/share/nautilus/scripts`; usare lo script su un file chiamato `'$(touch x)'` e sullo sfondo di una cartella | Permessi `-rwx------`; percorso esatto; nessun file `x` creato | | |
| M-25 | W10, W11 | Cartella OneDrive con file "solo online": cercare con "salta solo online" attivo e spento | Con l'opzione attiva nessun download né richiamo (B-P1); con l'opzione spenta i file sono letti | | |
| M-26 | W10, W11 | `compact /c /exe:lzx` su una cartella di prova (file WOF), poi cercare | File letti; nessun download; nessun errore | | |
| M-27 | W10 | Estrarre da uno zip scaricato dal browser (con Mark-of-the-Web) un `fattura.docm`; "Apri" | Word mostra la vista protetta/blocco macro, non "Abilita contenuto" (D-02) | | |
| M-28 | W10, W11 | Cercare in uno zip che contiene `tema.theme`, `app.jnlp`, `x.pyc`; "Apri" su ciascuna voce | Nessuna viene eseguita; ripiego su "Mostra nella cartella" (D-01) | | |
| M-29 | W10, W11 | Con `settings.toml` della versione successiva (cambiare il numero di versione a mano), avviare, cambiare un'impostazione, chiudere | Messaggio visibile che le modifiche non si salvano (C-11) | | |

### Rilascio

| ID | Piattaforma | Passi | Esito atteso | Esito | Note |
|---|---|---|---|---|---|
| M-30 | W10 | `Get-FileHash -Algorithm SHA256 .\lynkeus-searcher-0.1.0-windows-x86_64.zip` e confronto con `SHA256SUMS` | Hash uguale | | |
| M-31 | UBU | `sha256sum -c SHA256SUMS` | `OK` | | |
| M-32 | W10, W11 | Scaricare lo zip con il browser; avviare: SmartScreen "Windows ha protetto il PC" → "Ulteriori informazioni" → "Esegui comunque" | Il README descrive esattamente questi passi | | |
| M-33 | W10 (VM pulita 22H2, senza Visual C++ Redistributable) | Estrarre lo zip, avviare; poi `dumpbin /headers /loadconfig lynkeus-searcher.exe \| findstr /i "Guard CET Dependent"` | L'app parte (D-04); CFG e CET presenti; `DependentLoadFlags` 0x800 (D-03) | | |
| M-34 | W10, W11 | Cercare nel pacchetto estratto: nome utente, nome del computer e percorsi locali (`Select-String` sull'exe e sui file dello zip) | Nessuna occorrenza | | |

## Livello 5: usabilità con tre persone

Tre persone diverse, senza spiegazioni preventive, su un pacchetto estratto, con `fixtures/demo` e `fixtures/formati`. Si osserva senza aiutare e si annotano tempo, esitazioni, errori e frasi dette ad alta voce.

| ID | Compito | Cosa osservare |
|---|---|---|
| U-01 | Trovare dove compare "aragosta" in `fixtures/demo` | Se capiscono come aggiungere la cartella e avviare; se leggono i risultati; se notano il contesto nell'anteprima; tempo al primo risultato |
| U-02 | Cercare due parole che compaiono nello stesso file (AND), poi una delle due (OR) | Se trovano l'opzione; se capiscono il messaggio quando non c'è nessun risultato |
| U-03 | Esportare le sole righe selezionate e aprirle in un foglio di calcolo | Se trovano "Esporta"; se capiscono la selezione; se il CSV si apre senza passaggi extra |
| U-04 | Aprire una voce di uno zip (`fixtures/demo/musica/archivio.zip`) | Se capiscono che la voce è dentro un archivio; se capiscono l'apertura da copia privata |
| U-05 | Annullare una ricerca lunga e rifarla con un'altra parola | Se trovano Annulla; se capiscono lo stato dopo l'annullamento |
| U-06 | Dopo un errore indotto (termine vuoto, filtro "Solo queste" senza elenco) correggere e ripetere | Se vedono il messaggio; se capiscono come rimediare |

Per ogni sessione: un file di note con data, commit, ambiente, persona (sigla anonima), esito per compito e problemi osservati. Ogni problema ricorrente diventa una voce di `TIDY.md` o un problema dell'analisi.

## Matrice di tracciabilità

| Problema | Test che lo rileva |
|---|---|
| A-01 | T-A08, M-06 |
| A-02 | T-A01, T-A02, T-A04 |
| A-03 | T-A03, T-A04, M-02 |
| A-04 | T-A05 |
| A-05 | T-A06 |
| A-06 | T-A07 |
| A-07 | T-A09 |
| A-08 | T-A11, M-20 |
| A-09 | T-A10 |
| A-10 | revisione dei commenti (nessun test) |
| A-P1 | T-A14 |
| A-P2 | T-A12 |
| A-P3 | T-A13 (fuzzing `ppt_records` con oracolo di tempo) |
| A-P4 | T-A08, limite di memoria del processo (M-19) |
| A-P5 | T-A13 |
| B-01 | T-B01, P-02, M-03 |
| B-02 | P-03 |
| B-03 | T-B02, T-B03, T-B04, T-B05 |
| B-04 | T-B06, M-20 |
| B-05 | controllo dei cicli in `xtask` (da aggiungere, vedi Livello 2, area B) |
| B-06 | test esistenti `the_result_limit_is_exact_across_threads`, `blocks_equal_whole` |
| B-07 | T-B07 |
| B-08 | T-B09 |
| B-09 | M-18 |
| B-10 | T-B10 |
| B-11 | P-04 |
| B-12 | P-05 |
| B-13 | T-B01 |
| B-P1 | M-25 |
| B-P2 | T-B03 (variante: figlio che non legge stdin) |
| B-P3 | T-B03 |
| B-P4 | T-B08 |
| B-P5 | revisione; T-B01 (nessun thread rimasto dopo l'arresto) |
| C-01 | T-C01, M-08 |
| C-02 | T-C07, M-11 |
| C-03 | T-C04, M-09 |
| C-04 | T-C02, T-C03, M-02 |
| C-05 | T-C05, M-15 |
| C-06 | T-C08 |
| C-07 | P-08, P-09, M-17 |
| C-08 | P-07, P-09, M-17 |
| C-09 | T-C11, M-11 |
| C-10 | T-C10, M-03 |
| C-11 | T-C13, M-29 |
| C-12 | T-C06, M-15 |
| C-13 | M-10 |
| C-14 | M-13 |
| C-15 | M-12 |
| C-16 | T-C12 |
| C-17 | M-13 |
| C-18 | M-15 (ID mancante visibile come testo grezzo) |
| C-P1 | M-12 |
| C-P2 | T-C01 |
| C-P3 | M-09 |
| C-P4 | M-11 |
| C-P5 | M-01 (finestra sempre ridisegnata durante "Aggiungi cartella…") |
| C-P6 | M-04 |
| D-01 | T-D01, M-28 |
| D-02 | T-D02, M-27 |
| D-03 | T-D03, T-D04, M-33 |
| D-04 | T-D03, T-D04, M-33 |
| D-05 | T-D05, M-16 |
| D-06 | T-D06, M-22 |
| D-07 | T-D04 |
| D-08 | T-D12 |
| D-09 | T-D10, M-19 |
| D-10 | T-D11, M-22 |
| D-P1 | revisione; Miri dopo la correzione |
| D-P2 | T-D13, M-23 |

## Registro delle esecuzioni

| Data | Commit | Ambiente | Livelli eseguiti | Esito | Note |
|---|---|---|---|---|---|
|  |  |  |  |  |  |

Regola: un test manuale che trova un problema diventa un test automatico, scritto prima della correzione (si vede fallire, poi si corregge); il problema entra in `docs/analisi-conformita.md` con un nuovo ID e nella matrice. Il registro si compila a ogni esecuzione, anche parziale, con il commit esatto.

## Esecuzione automatica

`cargo xtask test-plan` esegue con un solo comando la parte automatizzabile del piano e scrive un rapporto.

| Opzione | Effetto |
|---|---|
| nessuna | tutti i livelli, da 1 a 5 |
| `--level 1,2,3` (o `--level=1,2,3`) | solo i livelli indicati |
| `--quick` | salta `cargo xtask check`, `cargo xtask check-linux`, le misure lunghe (P-01, P-02) e il pacchetto di `cargo xtask dist` (livello 4) |
| `--clipboard` | esegue anche P-08, che sovrascrive gli appunti di sistema |
| `--registry` | esegue anche la parte automatica di M-22, che scrive e toglie chiavi in una radice di prova di `HKCU\Software` |

Le voci che modificano stato condiviso della macchina (appunti, registro, menu contestuale) non partono senza l'opzione esplicita: nel rapporto risultano "non eseguita: richiede --clipboard" (o `--registry`).

Cosa esegue:

1. `cargo xtask check` e, da Windows, `cargo xtask check-linux`; poi `cargo test --release --workspace --locked --no-fail-fast -- --list` (compilazione ed elenco dei test) e la stessa riga senza `--list` (tutte le suite in release, accettazione e interfaccia comprese), con una voce per suite.
2. L'esito di ogni T-xx dai test indicati in `xtask/src/plan_map.rs`, l'unico posto che lega gli ID del piano ai nomi dei test. La voce MAPPA fallisce se un test della mappa non compare nell'elenco di `cargo test -- --list`; il test `every_mapped_test_is_a_function_of_its_package` lo nota già in `cargo xtask check`. T-A13 (fuzzing) resta nel workflow `fuzz.yml`; T-D03 prende l'esito di `cargo xtask dist`.
3. Le misure P-01…P-09, un test `#[ignore]` per processo, in release: `crates/lynkeus-engine/tests/prestazioni.rs` (P-01, P-02, P-03, P-05) e `crates/lynkeus-app/tests/prestazioni_interfaccia.rs` (P-06…P-09) stampano `MISURA <ID> samples=… p50_ms=… p95_ms=… max_ms=… mem_mb=… mem_kind=…`; P-04 è il test di libreria `prepared_filter_timing` del motore (il filtro preparato è interno al crate), letto dalle sue righe `P-04 …: p50 …, p95 …, massimo …`. Il comando confronta i valori con le soglie di questo piano, scritte in `plan_map.rs`. La memoria è il picco della memoria privata su Windows e il picco RSS (`VmHWM`) su Linux. P-01 crea i 100.000 file nella cartella temporanea e li cancella alla fine; la misura a freddo non si fa, perché svuotare la cache del sistema richiede privilegi.
4. La parte automatica di M-18 (DACL delle cartelle private con `Get-Acl`, se esistono), M-22 (chiavi del clic destro in una radice di prova di `HKCU\Software`, lette con `reg.exe`), M-24 (Linux: permessi dello script di Nautilus), M-26 (file compresso con WOF), M-30 e M-31 (hash del pacchetto con `Get-FileHash` o `sha256sum -c`), M-33 (mitigazioni dell'eseguibile estratto dallo zip), M-34 (nessun dato locale nei file dello zip); i test di sistema sono in `crates/lynkeus-app/tests/sistema.rs`. Gli altri casi M-xx compaiono come "a mano", con ciò che resta da fare.
5. I compiti U-01…U-06, elencati come "a mano".

Esiti: superata, FALLITA, sopra soglia, non eseguita (con il motivo), non applicabile, altrove, a mano. Una soglia superata è un avviso; ogni altra voce automatica è obbligatoria. Il comando esce con 1 se fallisce una voce obbligatoria e con 2 se le opzioni sono sbagliate.

Rapporto: `target/test-plan/rapporto-<AAAA-MM-GG>T<hhmmss>Z.md` (ora UTC, non versionato, conservati gli ultimi 20), con data, commit, ambiente (sistema, CPU, core, RAM, dischi, antivirus se leggibile senza privilegi, toolchain, profilo) e, per ogni voce, comando, esito e durata; per le misure campioni, p50, p95, massimo, memoria di picco e soglia. Il riepilogo va su stderr. Le tabelle dei risultati del livello 3 e il registro delle esecuzioni si compilano dal rapporto.

In CI il workflow pianificato `test-plan.yml` (ogni lunedì, o a richiesta) esegue `cargo xtask test-plan --level 1,2,3` su Windows e Ubuntu e pubblica il rapporto come artifact; su runner condivisi i tempi sono solo indicativi.
