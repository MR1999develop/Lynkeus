// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Il piano di test (`docs/piano-di-test.md`) come dati, per `cargo xtask test-plan`: per
//! ogni voce dove la si verifica.
//!
//! - Livello 2: ogni T-xx con i test che lo coprono (pacchetto, bersaglio, nome);
//! - livello 3: ogni P-xx con il test `#[ignore]` di misura e le soglie del piano;
//! - livello 4: ogni M-xx, manuale o con la sua parte automatica;
//! - livello 5: le prove con le persone (U-xx), sempre manuali.
//!
//! È l'unico posto che lega gli ID del piano ai nomi dei test: un test rinominato si
//! corregge qui. Il controllo della mappa in `cargo xtask test-plan` verifica che ogni test
//! elencato esista tra quelli di `cargo test -- --list`; il test unitario di questo modulo
//! verifica che il nome compaia come `fn` nei sorgenti del pacchetto (così anche
//! `cargo xtask check` lo nota). Le soglie sono le proposte del piano, da confermare dopo la
//! prima serie completa: superarle è un avviso, non un fallimento.

/// Il sistema su cui una voce si verifica.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Any,
    Windows,
    Linux,
}

impl Os {
    /// Vero se la voce si verifica sul sistema dove gira xtask.
    pub fn includes_host(self) -> bool {
        match self {
            Os::Any => true,
            Os::Windows => cfg!(windows),
            Os::Linux => cfg!(target_os = "linux"),
        }
    }

    /// Il nome per il rapporto.
    pub fn label(self) -> &'static str {
        match self {
            Os::Any => "tutti",
            Os::Windows => "Windows",
            Os::Linux => "Linux",
        }
    }
}

/// Il bersaglio di cargo che contiene un test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// I test unitari della libreria del pacchetto.
    Lib,
    /// I test unitari di un binario.
    Bin(&'static str),
    /// Un file di `tests/`.
    Test(&'static str),
}

/// Un test del workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestRef {
    pub package: &'static str,
    pub target: Target,
    /// Il nome della funzione, o la sua coda `modulo::nome` se il nome da solo è ambiguo
    /// nel bersaglio.
    pub name: &'static str,
    pub os: Os,
}

impl TestRef {
    /// Il nome dell'eseguibile di test senza l'hash (`lynkeus_app`, `accettazione`), come
    /// lo stampa cargo nella riga `Running`.
    pub fn suite(&self) -> String {
        match self.target {
            Target::Lib => self.package.replace('-', "_"),
            Target::Bin(name) | Target::Test(name) => name.replace('-', "_"),
        }
    }

    /// Il comando che esegue questo test, in release. Con `--workspace` le feature delle
    /// dipendenze sono quelle delle suite, quindi nulla si ricompila (con `-p` cambierebbero e
    /// la build di rilascio ripartirebbe); il bersaglio sceglie il pacchetto. Il nome filtra
    /// come sottostringa (senza `--exact`, che vorrebbe il nome completo con i moduli): i nomi
    /// della mappa non sono parte di altri nomi dello stesso bersaglio.
    pub fn command(&self, ignored: bool) -> String {
        format!(
            "cargo test --release --workspace {} -- {}",
            self.cargo_args().join(" "),
            TestRef::harness_args(ignored).join(" ")
        )
    }

    /// Gli argomenti di cargo dopo il pacchetto: bersaglio e filtro.
    pub fn cargo_args(&self) -> Vec<&'static str> {
        let mut args = match self.target {
            Target::Lib => vec!["--lib"],
            Target::Bin(name) => vec!["--bin", name],
            Target::Test(name) => vec!["--test", name],
        };
        args.push(self.name);
        args
    }

    /// Gli argomenti di libtest dopo `--`, con `--ignored` per i test `#[ignore]`.
    pub fn harness_args(ignored: bool) -> &'static [&'static str] {
        if ignored {
            &["--ignored", "--nocapture"]
        } else {
            &["--nocapture"]
        }
    }

    /// Vero se `listed` (il nome completo stampato da libtest) è questo test.
    pub fn matches(&self, listed: &str) -> bool {
        listed == self.name
            || listed
                .strip_suffix(self.name)
                .is_some_and(|head| head.ends_with("::"))
    }
}

const fn lib(package: &'static str, name: &'static str) -> TestRef {
    TestRef {
        package,
        target: Target::Lib,
        name,
        os: Os::Any,
    }
}

const fn test(package: &'static str, file: &'static str, name: &'static str) -> TestRef {
    TestRef {
        package,
        target: Target::Test(file),
        name,
        os: Os::Any,
    }
}

const fn only(os: Os, test: TestRef) -> TestRef {
    TestRef { os, ..test }
}

const CORE: &str = "lynkeus-core";
const EXTRACT: &str = "lynkeus-extract";
const ENGINE: &str = "lynkeus-engine";
const APP: &str = "lynkeus-app";
const XTASK: &str = "xtask";

/// Dove si verifica una voce del livello 2.
#[derive(Debug, Clone, Copy)]
pub enum Coverage {
    /// Test della suite: la voce passa se passano tutti quelli del sistema corrente.
    Tests(&'static [TestRef]),
    /// Test della suite più la verifica sul pacchetto di `cargo xtask dist` (livello 4).
    TestsAndDist(&'static [TestRef]),
    /// Solo la verifica sul pacchetto di `cargo xtask dist`.
    Dist,
    /// Fuori da questo comando, con il motivo.
    Elsewhere(&'static str),
}

/// Una voce T-xx del livello 2.
#[derive(Debug, Clone, Copy)]
pub struct TestItem {
    pub id: &'static str,
    pub title: &'static str,
    pub coverage: Coverage,
}

const fn item(id: &'static str, title: &'static str, coverage: Coverage) -> TestItem {
    TestItem {
        id,
        title,
        coverage,
    }
}

/// Le voci del livello 2, nell'ordine del piano.
pub const TEST_ITEMS: &[TestItem] = &[
    // Area A.
    item(
        "T-A01",
        "ogni termine trova se stesso",
        Coverage::Tests(&[test(CORE, "proprieta", "term_finds_itself_in_every_mode")]),
    ),
    item(
        "T-A02",
        "NFC e NFD danno le stesse parole",
        Coverage::Tests(&[test(
            CORE,
            "proprieta",
            "nfc_and_nfd_text_give_the_same_words",
        )]),
    ),
    item(
        "T-A03",
        "termini che diventano jolly rifiutati",
        Coverage::Tests(&[lib(CORE, "terms_normalizing_to_wildcards_are_rejected")]),
    ),
    item(
        "T-A04",
        "find_words come l'oracolo ingenuo",
        Coverage::Tests(&[test(CORE, "proprieta", "find_words_matches_naive_oracle")]),
    ),
    item(
        "T-A05",
        "paragrafi nelle destinazioni RTF saltate",
        Coverage::Tests(&[lib(EXTRACT, "paragraph_marks_inside_skipped_destinations")]),
    ),
    item(
        "T-A06",
        "gz con tar ha i limiti del tar.gz",
        Coverage::Tests(&[test(
            EXTRACT,
            "archives",
            "gz_with_tar_content_has_tar_gz_limits",
        )]),
    ),
    item(
        "T-A07",
        "EPUB con profondità e voci limitate",
        Coverage::Tests(&[test(
            EXTRACT,
            "archives",
            "epub_respects_depth_and_entry_budget",
        )]),
    ),
    item(
        "T-A08",
        "xlsx sparso: limite, non crash",
        Coverage::Tests(&[test(
            EXTRACT,
            "processo",
            "sparse_xlsx_does_not_crash_the_worker",
        )]),
    ),
    item(
        "T-A09",
        "nome non UTF-8 con il suo formato",
        Coverage::Tests(&[lib(EXTRACT, "non_utf8_name_keeps_its_format")]),
    ),
    item(
        "T-A10",
        "etichette senza separatori invisibili",
        Coverage::Tests(&[lib(CORE, "labels_neutralize_separators_and_zero_width")]),
    ),
    item(
        "T-A11",
        "processo estrattore senza limite di memoria rifiutato",
        Coverage::Tests(&[lib(EXTRACT, "worker_refuses_missing_memory_limit")]),
    ),
    item(
        "T-A12",
        "pptx con diapositive ripetute letto una volta",
        Coverage::Tests(&[test(
            EXTRACT,
            "fixtures",
            "pptx_repeated_slide_ids_read_once",
        )]),
    ),
    item(
        "T-A13",
        "fuzzing con oracoli",
        Coverage::Elsewhere(
            "fuzzing su Linux con cargo +nightly fuzz (workflow fuzz.yml, fuzz/README.md)",
        ),
    ),
    item(
        "T-A14",
        "zip imbottito: limite del rapporto",
        Coverage::Tests(&[test(EXTRACT, "archives", "padded_zip_hits_ratio_limit")]),
    ),
    // Area B.
    item(
        "T-B01",
        "Annulla di un processo occupato entro 1 s",
        Coverage::Tests(&[test(
            ENGINE,
            "annulla",
            "cancel_stops_a_busy_worker_within_one_second",
        )]),
    ),
    item(
        "T-B02",
        "unità oltre 1 MiB: violazione del protocollo",
        Coverage::Tests(&[test(
            ENGINE,
            "protocollo",
            "unit_over_1_mib_is_a_protocol_violation",
        )]),
    ),
    item(
        "T-B03",
        "processo che inonda: scadenza",
        Coverage::Tests(&[test(
            ENGINE,
            "protocollo",
            "flooding_worker_times_out_within_budget",
        )]),
    ),
    item(
        "T-B04",
        "processo che parla da inattivo: sostituito",
        Coverage::Tests(&[test(
            ENGINE,
            "protocollo",
            "worker_speaking_while_idle_is_replaced",
        )]),
    ),
    item(
        "T-B05",
        "EntryData fuori sequenza rifiutato",
        Coverage::Tests(&[test(
            ENGINE,
            "protocollo",
            "entry_data_out_of_sequence_is_rejected",
        )]),
    ),
    item(
        "T-B06",
        "orfano occupato esce col padre",
        Coverage::Tests(&[
            only(
                Os::Linux,
                test(ENGINE, "processi", "busy_orphan_exits_with_its_parent"),
            ),
            only(
                Os::Linux,
                test(EXTRACT, "processo", "busy_orphan_exits_with_its_parent"),
            ),
        ]),
    ),
    item(
        "T-B07",
        "svuotare la cache durante una ricerca",
        Coverage::Tests(&[test(ENGINE, "cache", "cache_clear_during_search_is_safe")]),
    ),
    item(
        "T-B08",
        "due istanze sulla stessa cache",
        Coverage::Tests(&[test(
            ENGINE,
            "cache",
            "two_instances_share_the_cache_safely",
        )]),
    ),
    item(
        "T-B09",
        "scrittura della cache fallita contata",
        Coverage::Tests(&[lib(ENGINE, "failed_cache_put_is_counted")]),
    ),
    item(
        "T-B10",
        "anteprima senza cache ferma a finestra completa",
        Coverage::Tests(&[test(
            ENGINE,
            "protocollo",
            "preview_without_cache_stops_at_a_full_window",
        )]),
    ),
    // Area C.
    item(
        "T-C01",
        "Esc chiude il menu durante la ricerca",
        Coverage::Tests(&[test(APP, "interfaccia", "esc_closes_menu_while_searching")]),
    ),
    item(
        "T-C02",
        "errore del modulo apre i Filtri",
        Coverage::Tests(&[test(APP, "interfaccia", "form_error_opens_filters")]),
    ),
    item(
        "T-C03",
        "errore con il pannello nascosto",
        Coverage::Tests(&[test(APP, "interfaccia", "form_error_with_hidden_panel")]),
    ),
    item(
        "T-C04",
        "riga selezionata esposta all'accessibilità",
        Coverage::Tests(&[test(APP, "interfaccia", "row_selection_is_exposed")]),
    ),
    item(
        "T-C05",
        "numeri raggruppati per lingua",
        Coverage::Tests(&[lib(APP, "numbers_grouped_per_language")]),
    ),
    item(
        "T-C06",
        "plurali di saltati e binari",
        Coverage::Tests(&[lib(APP, "plural_skipped_binary")]),
    ),
    item(
        "T-C07",
        "contrasto della riga selezionata",
        Coverage::Tests(&[lib(APP, "selection_contrast")]),
    ),
    item(
        "T-C08",
        "avanzamento indeterminato durante la scansione",
        Coverage::Tests(&[lib(APP, "progress_indeterminate_while_walking")]),
    ),
    item(
        "T-C09",
        "finestra 1024×600 con tutto visibile",
        Coverage::Tests(&[test(
            APP,
            "interfaccia",
            "the_window_fits_1024_by_600_with_everything_shown",
        )]),
    ),
    item(
        "T-C10",
        "stati vuoti distinti",
        Coverage::Tests(&[lib(APP, "empty_state_partitions")]),
    ),
    item(
        "T-C11",
        "contrasto del bordo di focus",
        Coverage::Tests(&[lib(APP, "focus_ring_contrast")]),
    ),
    item(
        "T-C12",
        "riga di termine vuota ignorata",
        Coverage::Tests(&[lib(APP, "empty_term_row_is_ignored_when_another_is_valid")]),
    ),
    item(
        "T-C13",
        "impostazioni più recenti segnalate",
        Coverage::Tests(&[lib(APP, "settings_newer_version_is_reported")]),
    ),
    // Area D.
    item(
        "T-D01",
        "lanciatori per associazione eseguibili",
        Coverage::Tests(&[lib(APP, "association_launchers_are_executable")]),
    ),
    item(
        "T-D02",
        "voce estratta con il Mark-of-the-Web",
        Coverage::Tests(&[only(
            Os::Windows,
            lib(APP, "extracted_entry_keeps_mark_of_the_web"),
        )]),
    ),
    item(
        "T-D03",
        "DLL importate mai cercate accanto all'exe",
        Coverage::Dist,
    ),
    item(
        "T-D04",
        "mitigazioni del PE presenti",
        Coverage::TestsAndDist(&[
            TestRef {
                package: XTASK,
                target: Target::Bin(XTASK),
                name: "each_missing_mitigation_is_reported",
                os: Os::Any,
            },
            TestRef {
                package: XTASK,
                target: Target::Bin(XTASK),
                name: "a_complete_image_has_no_problems",
                os: Os::Any,
            },
        ]),
    ),
    item(
        "T-D05",
        "stato della finestra nella cartella di configurazione",
        Coverage::Tests(&[TestRef {
            package: APP,
            target: Target::Bin("lynkeus-searcher"),
            name: "window_state_file_is_inside_the_config_dir",
            os: Os::Any,
        }]),
    ),
    item(
        "T-D06",
        "exe non sicuro: voce del clic destro tolta",
        Coverage::Tests(&[lib(APP, "refresh_with_an_unsafe_exe_removes_our_entry")]),
    ),
    item(
        "T-D07",
        "il figlio eredita solo stdio",
        Coverage::Tests(&[only(Os::Windows, lib(APP, "child_inherits_only_stdio"))]),
    ),
    item(
        "T-D08",
        "un client lento non blocca il successivo",
        Coverage::Tests(&[lib(APP, "a_slow_client_does_not_block_the_next")]),
    ),
    item(
        "T-D09",
        "venti errori di accept fermano il server",
        Coverage::Tests(&[lib(APP, "twenty_accept_errors_stop_the_server")]),
    ),
    item(
        "T-D10",
        "il job limita processi e interfaccia",
        Coverage::Tests(&[only(
            Os::Windows,
            lib(APP, "job_limits_active_process_and_ui"),
        )]),
    ),
    item(
        "T-D11",
        "percorsi senza --add aggiunti",
        Coverage::Tests(&[lib(APP, "bare_paths_are_added_like_dash_dash_add")]),
    ),
    item(
        "T-D12",
        "eccezioni di deny.toml con scadenza",
        Coverage::Tests(&[TestRef {
            package: XTASK,
            target: Target::Bin(XTASK),
            name: "deny_exceptions_expire",
            os: Os::Any,
        }]),
    ),
    item(
        "T-D13",
        "% nel percorso dell'exe rifiutato",
        Coverage::Tests(&[lib(APP, "percent_in_exe_path_is_refused")]),
    ),
];

/// Una soglia del piano su un valore della riga `MISURA` (millisecondi o MiB).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Threshold {
    /// `key` ≤ `limit`.
    AtMost { key: &'static str, limit: f64 },
    /// `key` < `limit`.
    Below { key: &'static str, limit: f64 },
    /// `key` < il valore di `other` nella stessa riga (miglioramento oltre il rumore).
    BelowKey {
        key: &'static str,
        other: &'static str,
    },
}

/// Come si leggono i valori di una misura nell'uscita del suo test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// La riga `MISURA <id> chiave=valore …` (`crates/lynkeus-engine/tests/misura`).
    Line,
    /// Le righe `<id> <etichetta>: p50 <durata>, p95 <durata>, massimo <durata>` di un test
    /// scritto prima della riga `MISURA`, con le durate nel formato `Debug` di `Duration`:
    /// `main` dà p50, p95 e massimo, `baseline` il valore `baseline_p50_ms`. Memoria non
    /// misurata.
    Labeled {
        main: &'static str,
        baseline: &'static str,
    },
}

/// Stato condiviso della macchina che un test modifica: il comando non lo tocca senza
/// l'opzione esplicita, e il rapporto dice quale serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SharedState {
    /// Gli appunti di sistema (`--clipboard`): la copia li sovrascrive.
    Clipboard,
    /// Il registro dell'utente (`--registry`): chiavi scritte e tolte in una radice di prova.
    Registry,
}

impl SharedState {
    pub const ALL: [SharedState; 2] = [SharedState::Clipboard, SharedState::Registry];

    /// L'opzione del comando che lo permette.
    pub fn option(self) -> &'static str {
        match self {
            SharedState::Clipboard => "--clipboard",
            SharedState::Registry => "--registry",
        }
    }

    /// Che cosa tocca, per il rapporto.
    pub fn effect(self) -> &'static str {
        match self {
            SharedState::Clipboard => "sovrascrive gli appunti di sistema",
            SharedState::Registry => "scrive e toglie chiavi nel registro dell'utente",
        }
    }

    /// Il motivo per cui una voce non è stata eseguita senza l'opzione.
    pub fn requirement(self) -> String {
        format!("richiede {} ({})", self.option(), self.effect())
    }
}

/// Quanto dura una voce: le lunghe si saltano con `--quick`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Length {
    Short,
    Long,
}

/// Una misura P-xx del livello 3.
#[derive(Debug, Clone, Copy)]
pub struct Measure {
    pub id: &'static str,
    pub title: &'static str,
    /// Il test `#[ignore]` della misura.
    pub test: TestRef,
    /// Come si leggono i valori nell'uscita del test.
    pub output: Output,
    pub thresholds: &'static [Threshold],
    pub length: Length,
    /// Lo stato condiviso che il test modifica, se c'è.
    pub touches: Option<SharedState>,
    /// Cosa dicono i valori in più della riga, per il rapporto.
    pub note: &'static str,
}

const fn engine_measure(name: &'static str) -> TestRef {
    test(ENGINE, "prestazioni", name)
}

const fn app_measure(name: &'static str) -> TestRef {
    test(APP, "prestazioni_interfaccia", name)
}

/// Le misure del livello 3, nell'ordine del piano.
pub const MEASURES: &[Measure] = &[
    Measure {
        id: "P-01",
        title: "scansione di 100.000 file di testo",
        test: engine_measure("p01_scan_100k_text_files"),
        output: Output::Line,
        thresholds: &[Threshold::AtMost {
            key: "p95_ms",
            limit: 1_500.0,
        }],
        length: Length::Long,
        touches: None,
        note: "a caldo; misura a freddo non fatta (svuotare la cache di sistema richiede \
               privilegi); prepare_ms = creazione dei file",
    },
    Measure {
        id: "P-02",
        title: "Annulla sotto carico (4 processi, CPU satura)",
        test: engine_measure("p02_cancel_under_load"),
        output: Output::Line,
        thresholds: &[Threshold::Below {
            key: "max_ms",
            limit: 1_000.0,
        }],
        length: Length::Long,
        touches: None,
        note: "30 ripetizioni; tempo da Annulla a Finished",
    },
    Measure {
        id: "P-03",
        title: "AND con 16 file da 300.000 \"il\": memoria",
        test: engine_measure("p03_and_retained_hits_memory"),
        output: Output::Line,
        thresholds: &[Threshold::Below {
            key: "mem_mb",
            limit: 256.0,
        }],
        length: Length::Short,
        touches: None,
        note: "5 ripetizioni; budget di memoria proposto 256 MiB",
    },
    Measure {
        id: "P-04",
        title: "filtro di 32 estensioni su 100.000 nomi",
        // Il filtro preparato della scansione è interno al crate: la misura è il suo test di
        // libreria, che stampa le righe prima del formato `MISURA`.
        test: lib(ENGINE, "prepared_filter_timing"),
        output: Output::Labeled {
            main: "filtro preparato",
            baseline: "confronto con to_lowercase",
        },
        thresholds: &[Threshold::BelowKey {
            key: "p95_ms",
            other: "baseline_p50_ms",
        }],
        length: Length::Short,
        touches: None,
        note: "filtro preparato della scansione contro to_lowercase per voce (baseline); \
               memoria e allocazioni nel ciclo non misurate",
    },
    Measure {
        id: "P-05",
        title: "cache contro estrazione (documenti demo)",
        test: engine_measure("p05_cache_against_extraction"),
        output: Output::Line,
        thresholds: &[Threshold::BelowKey {
            key: "p50_ms",
            other: "baseline_p50_ms",
        }],
        length: Length::Short,
        touches: None,
        note: "tempi della cache; baseline = processo estrattore",
    },
    Measure {
        id: "P-06",
        title: "ordinamento di 200.000 righe per colonna",
        test: app_measure("p06_sort_200k"),
        output: Output::Line,
        thresholds: &[Threshold::Below {
            key: "p95_ms",
            limit: 50.0,
        }],
        length: Length::Short,
        touches: None,
        note: "5 colonne × 15 ripetizioni; batch_max_ms = lotto più lento durante la ricerca",
    },
    Measure {
        id: "P-07",
        title: "step_target con 200.000 righe",
        test: app_measure("p07_step_target_200k"),
        output: Output::Line,
        thresholds: &[Threshold::Below {
            key: "max_ms",
            limit: 1.0,
        }],
        length: Length::Short,
        touches: None,
        note: "tempo per chiamata",
    },
    Measure {
        id: "P-08",
        title: "copia negli appunti al limite di 8 MiB e rifiuto con 200.000 righe",
        test: app_measure("p08_copy_200k"),
        output: Output::Line,
        thresholds: &[
            Threshold::AtMost {
                key: "max_ms",
                limit: 100.0,
            },
            Threshold::AtMost {
                key: "reject_block_max_ms",
                limit: 100.0,
            },
            Threshold::Below {
                key: "reject_max_ms",
                limit: 1_000.0,
            },
        ],
        length: Length::Short,
        touches: Some(SharedState::Clipboard),
        note: "blocco più lungo del thread dell'interfaccia nella copia al limite, consegna \
               agli appunti compresa; copy_max_ms = copia completa al limite; reject_* = \
               rifiuto delle 200.000 righe (blocco e tempo fino al messaggio), appunti non \
               toccati; il testo di prima torna negli appunti alla fine",
    },
    Measure {
        id: "P-09",
        title: "risposta a un tasto con 200.000 risultati",
        test: app_measure("p09_key_response_200k"),
        output: Output::Line,
        thresholds: &[Threshold::Below {
            key: "max_ms",
            limit: 100.0,
        }],
        length: Length::Short,
        touches: None,
        note: "frame che riceve la freccia giù (kittest, senza GPU); first_frame_ms = apertura \
               della finestra fino a stabile",
    },
];

/// La parte automatica di un test manuale del livello 4.
#[derive(Debug, Clone, Copy)]
pub enum Automation {
    /// Solo a mano.
    Manual,
    /// Un test `#[ignore]` di sistema (`crates/lynkeus-app/tests/sistema.rs`).
    IgnoredTest(TestRef),
    /// Descrittore di sicurezza delle cartelle private dell'utente, letto con `Get-Acl`.
    PrivateDirAcl,
    /// Hash dello zip contro `SHA256SUMS` con lo strumento del sistema.
    Checksums,
    /// `cargo xtask mitigations` sull'eseguibile estratto dallo zip.
    Mitigations,
    /// Nome utente, nome del computer e percorsi locali nei file dello zip.
    NoLocalData,
}

/// Un caso M-xx del livello 4.
#[derive(Debug, Clone, Copy)]
pub struct ManualCase {
    pub id: &'static str,
    pub title: &'static str,
    /// Il sistema della parte automatica (quella manuale segue la colonna del piano).
    pub os: Os,
    pub automation: Automation,
    /// Ciò che resta da fare a mano dopo la parte automatica.
    pub rest: &'static str,
    /// Lo stato condiviso che la parte automatica modifica, se c'è.
    pub touches: Option<SharedState>,
}

const fn manual(id: &'static str, title: &'static str) -> ManualCase {
    ManualCase {
        id,
        title,
        os: Os::Any,
        automation: Automation::Manual,
        rest: "",
        touches: None,
    }
}

const fn automated(
    id: &'static str,
    title: &'static str,
    os: Os,
    automation: Automation,
    rest: &'static str,
) -> ManualCase {
    ManualCase {
        id,
        title,
        os,
        automation,
        rest,
        touches: None,
    }
}

/// `case`, la cui parte automatica modifica `state`.
const fn touching(state: SharedState, case: ManualCase) -> ManualCase {
    ManualCase {
        touches: Some(state),
        ..case
    }
}

/// I casi del livello 4, nell'ordine del piano.
pub const MANUAL_CASES: &[ManualCase] = &[
    manual("M-01", "pacchetto estratto, ricerca di \"ar\" nella demo"),
    manual("M-02", "filtri"),
    manual("M-03", "Annulla a metà, anche su 100.000 file"),
    manual("M-04", "anteprima e apertura, voci di zip ed EPUB"),
    manual("M-05", "esportazione CSV e TSV in Excel e Calc"),
    manual("M-06", "ricerca in fixtures/malevoli"),
    manual("M-07", "solo tastiera"),
    manual("M-08", "Esc sui menu durante una ricerca"),
    manual("M-09", "Narrator e Orca"),
    manual("M-10", "nomi dei pulsanti dell'anteprima"),
    manual("M-11", "temi chiaro, scuro e contrasto elevato"),
    manual("M-12", "ordine del Tab"),
    manual(
        "M-13",
        "ordinamento durante la ricerca, ripristino dei filtri",
    ),
    manual("M-14", "finestra 1024×600"),
    manual("M-15", "interfaccia in inglese"),
    manual("M-16", "finestra e impostazioni ricordate"),
    manual(
        "M-17",
        "200.000 risultati: Ctrl+A, Ctrl+C, mouse sulle righe",
    ),
    automated(
        "M-18",
        "cartelle private: DACL protetta, solo utente e SYSTEM",
        Os::Windows,
        Automation::PrivateDirAcl,
        "punto 3 (permesso concesso a Users, riavvio, barra \"cartelle non private\")",
    ),
    manual(
        "M-19",
        "Process Explorer: job, ambiente, handle, kill del padre",
    ),
    manual("M-20", "Linux: kill -9 del padre, nessun figlio rimasto"),
    manual(
        "M-21",
        "pipe dell'istanza unica e accesso di un altro utente",
    ),
    touching(
        SharedState::Registry,
        automated(
            "M-22",
            "clic destro: chiavi e riga di comando (radice di prova)",
            Os::Windows,
            Automation::IgnoredTest(only(
                Os::Windows,
                test(APP, "sistema", "m22_context_menu_keys_in_a_test_root"),
            )),
            "clic destro vero in Explorer sui percorsi del piano, exe spostato, trascinamento",
        ),
    ),
    manual(
        "M-23",
        "Windows 11: \"Mostra altre opzioni\", % nel percorso",
    ),
    automated(
        "M-24",
        "script di Nautilus privato",
        Os::Linux,
        Automation::IgnoredTest(only(
            Os::Linux,
            test(APP, "sistema", "m24_nautilus_script_is_private"),
        )),
        "script usato da Nautilus su '$(touch x)' e sullo sfondo di una cartella",
    ),
    manual("M-25", "OneDrive: file solo online"),
    automated(
        "M-26",
        "file compressi con WOF (compact /exe:lzx) letti",
        Os::Windows,
        Automation::IgnoredTest(only(
            Os::Windows,
            test(APP, "sistema", "m26_wof_compressed_files_are_read"),
        )),
        "",
    ),
    manual("M-27", "Mark-of-the-Web e vista protetta di Word"),
    manual("M-28", "voci pericolose in uno zip non eseguite"),
    manual("M-29", "settings.toml di una versione successiva"),
    automated(
        "M-30",
        "SHA256SUMS con Get-FileHash",
        Os::Windows,
        Automation::Checksums,
        "",
    ),
    automated(
        "M-31",
        "sha256sum -c SHA256SUMS",
        Os::Linux,
        Automation::Checksums,
        "",
    ),
    manual("M-32", "SmartScreen e README"),
    automated(
        "M-33",
        "mitigazioni dell'exe del pacchetto, nessun VCRUNTIME",
        Os::Windows,
        Automation::Mitigations,
        "avvio su una VM pulita senza Visual C++ Redistributable",
    ),
    automated(
        "M-34",
        "nessun dato locale nei file del pacchetto",
        Os::Any,
        Automation::NoLocalData,
        "",
    ),
];

/// I compiti del livello 5 (usabilità con tre persone), sempre manuali.
pub const USABILITY_TASKS: &[(&str, &str)] = &[
    ("U-01", "trovare \"aragosta\" nella demo"),
    ("U-02", "AND e OR tra due parole"),
    ("U-03", "esportare le righe selezionate"),
    ("U-04", "aprire una voce di uno zip"),
    ("U-05", "annullare e rifare una ricerca"),
    ("U-06", "correggere un errore indotto"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    /// Tutti i test citati dalla mappa (livelli 2, 3 e 4).
    fn all_refs() -> Vec<TestRef> {
        let mut refs: Vec<TestRef> = Vec::new();
        for item in TEST_ITEMS {
            if let Coverage::Tests(list) | Coverage::TestsAndDist(list) = item.coverage {
                refs.extend_from_slice(list);
            }
        }
        refs.extend(MEASURES.iter().map(|m| m.test));
        refs.extend(MANUAL_CASES.iter().filter_map(|c| match c.automation {
            Automation::IgnoredTest(test) => Some(test),
            _ => None,
        }));
        refs
    }

    fn package_dir(package: &str) -> PathBuf {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("cartella del workspace")
            .to_path_buf();
        if package == XTASK {
            workspace.join(XTASK)
        } else {
            workspace.join("crates").join(package)
        }
    }

    fn rust_sources(dir: &Path, found: &mut Vec<PathBuf>) {
        for item in std::fs::read_dir(dir)
            .expect("cartella leggibile")
            .flatten()
        {
            let path = item.path();
            if path.is_dir() {
                rust_sources(&path, found);
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }

    /// Ogni test della mappa è una funzione nei sorgenti del suo pacchetto: un test
    /// rinominato si nota già in `cargo xtask check`, non solo nel piano completo.
    #[test]
    fn every_mapped_test_is_a_function_of_its_package() {
        let mut missing = Vec::new();
        for test in all_refs() {
            let dir = package_dir(test.package);
            let mut files = Vec::new();
            rust_sources(&dir.join("src"), &mut files);
            if dir.join("tests").is_dir() {
                rust_sources(&dir.join("tests"), &mut files);
            }
            let function = test.name.rsplit("::").next().unwrap_or(test.name);
            let needle = format!("fn {function}(");
            let found = files
                .iter()
                .any(|file| std::fs::read_to_string(file).is_ok_and(|text| text.contains(&needle)));
            if !found {
                missing.push(format!("{} {}", test.package, test.name));
            }
        }
        assert!(
            missing.is_empty(),
            "test della mappa non trovati: {missing:?}"
        );
    }

    #[test]
    fn ids_are_unique_and_follow_the_plan_format() {
        let ids: Vec<&str> = TEST_ITEMS
            .iter()
            .map(|i| i.id)
            .chain(MEASURES.iter().map(|m| m.id))
            .chain(MANUAL_CASES.iter().map(|c| c.id))
            .chain(USABILITY_TASKS.iter().map(|(id, _)| *id))
            .collect();
        let unique: BTreeSet<&str> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len(), "ID ripetuti");
        assert_eq!(TEST_ITEMS.len(), 50, "50 voci nel livello 2 del piano");
        assert_eq!(MEASURES.len(), 9);
        assert_eq!(MANUAL_CASES.len(), 34);
    }

    #[test]
    fn only_the_declared_items_touch_shared_state() {
        let measures: Vec<(&str, SharedState)> = MEASURES
            .iter()
            .filter_map(|m| Some((m.id, m.touches?)))
            .collect();
        assert_eq!(measures, [("P-08", SharedState::Clipboard)]);
        let cases: Vec<(&str, SharedState)> = MANUAL_CASES
            .iter()
            .filter_map(|c| Some((c.id, c.touches?)))
            .collect();
        assert_eq!(cases, [("M-22", SharedState::Registry)]);
        assert_eq!(
            SharedState::Clipboard.requirement(),
            "richiede --clipboard (sovrascrive gli appunti di sistema)"
        );
    }

    #[test]
    fn measures_are_ignored_tests_in_their_files() {
        for measure in MEASURES {
            let own_file = matches!(measure.test.target, Target::Test(_));
            assert_eq!(own_file, measure.output == Output::Line, "{}", measure.id);
            assert!(!measure.thresholds.is_empty(), "{}", measure.id);
        }
    }

    #[test]
    fn listed_names_match_by_full_name_or_module_tail() {
        let reference = lib(APP, "selection_contrast");
        assert!(reference.matches("selection_contrast"));
        assert!(reference.matches("theme::tests::selection_contrast"));
        assert!(!reference.matches("theme::tests::no_selection_contrast"));
        assert!(!reference.matches("selection_contrast_dark"));
    }

    #[test]
    fn suite_and_command_partitions() {
        assert_eq!(lib(APP, "x").suite(), "lynkeus_app");
        assert_eq!(test(APP, "interfaccia", "x").suite(), "interfaccia");
        let bin = TestRef {
            package: APP,
            target: Target::Bin("lynkeus-searcher"),
            name: "x",
            os: Os::Any,
        };
        assert_eq!(bin.suite(), "lynkeus_searcher");
        assert_eq!(
            test(ENGINE, "prestazioni", "p01").command(true),
            "cargo test --release --workspace --test prestazioni p01 -- --ignored --nocapture"
        );
        assert_eq!(
            lib(CORE, "t").command(false),
            "cargo test --release --workspace --lib t -- --nocapture"
        );
        assert_eq!(
            bin.command(false),
            "cargo test --release --workspace --bin lynkeus-searcher x -- --nocapture"
        );
    }

    /// Nessun nome della mappa è parte di un altro nome dello stesso bersaglio: il filtro
    /// per sottostringa dei comandi sceglie un solo test.
    #[test]
    fn mapped_names_are_not_substrings_of_each_other() {
        let refs = all_refs();
        for a in &refs {
            for b in &refs {
                if a.package == b.package && a.target == b.target && a.name != b.name {
                    assert!(!b.name.contains(a.name), "{} dentro {}", a.name, b.name);
                }
            }
        }
    }
}
