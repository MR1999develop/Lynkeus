// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Il pool non si fida del processo estrattore: messaggi fuori misura o fuori sequenza,
//! processi che parlano senza richiesta, che inondano, che non leggono la richiesta; e i
//! tempi che non dipendono dal figlio (attese dell'interfaccia, anteprima già completa).

mod common;

use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use common::{documents_with, drain, fixtures, launcher, request, start_with, test_worker};
use lynkeus_core::{Combine, Location};
use lynkeus_engine::{
    CommandLauncher, DocPath, EntryEnd, ExtractEnd, ExtractorPool, Outcome, WorkerFailure,
};
use lynkeus_extract::{ExtractEvent, ExtractOptions, Stopped};

/// Tempo massimo di un'estrazione normale nei test.
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(30);

/// Margine oltre una scadenza attesa, per l'arresto e la raccolta del processo.
const STOP_SLACK: Duration = Duration::from_secs(2);

/// Scadenza dell'anteprima di `finestra.*`, che dopo cinque righe non risponde più: senza
/// l'arresto anticipato l'anteprima durerebbe tutta la scadenza.
const PREVIEW_TIMEOUT: Duration = Duration::from_secs(10);

/// Tempo massimo di un'anteprima la cui finestra è già completa, con il processo già avviato:
/// metà della scadenza, così il test distingue l'arresto anticipato dall'attesa della
/// scadenza senza dipendere dal carico della macchina.
const PREVIEW_BUDGET: Duration = Duration::from_secs(5);

/// Scadenza stretta per `molti.*` (400 documenti di una riga, pochi millisecondi di lavoro
/// con il processo già avviato): più breve della pausa dell'interfaccia.
const SHORT_TIMEOUT: Duration = Duration::from_secs(1);

/// Quanto l'interfaccia simulata non legge gli eventi: il doppio di [`SHORT_TIMEOUT`]. È lo
/// stimolo del test (un'interfaccia lenta), non un'attesa di una condizione.
const INTERFACE_PAUSE: Duration = Duration::from_secs(2);

/// Avvia il processo estrattore di `source` e aspetta che abbia servito una richiesta
/// (`note.docx` di `dir`): da qui i tempi misurati non comprendono l'avvio di un eseguibile
/// appena compilato, che su Windows l'antivirus può esaminare per secondi, ma solo il lavoro
/// sul file. Il processo torna libero e serve la richiesta seguente (un solo posto nel pool).
fn warm_up(source: &lynkeus_engine::DocumentSource, dir: &Path) {
    let never = || false;
    let mut events = Vec::new();
    let end = source.pool().extract(
        &dir.join("note.docx"),
        &ExtractOptions::default(),
        EXTRACT_TIMEOUT,
        &never,
        &mut events,
    );
    assert_eq!(end, ExtractEnd::Completed, "riscaldamento: {events:?}");
    assert_eq!(source.pool().launches(), 1);
}

/// Una cartella con i file `names` (contenuto da PDF) e una copia di `note.docx`.
fn dir_with(names: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("cartella temporanea");
    for name in names {
        std::fs::write(dir.path().join(name), b"%PDF-1.4\n").expect("file");
    }
    std::fs::copy(
        fixtures("demo/ufficio/note.docx"),
        dir.path().join("note.docx"),
    )
    .expect("copia");
    dir
}

fn pool(size: usize) -> ExtractorPool {
    ExtractorPool::new(launcher(), NonZeroUsize::new(size).expect("almeno uno"))
}

/// Estrae `path` con il pool e restituisce esito ed eventi.
fn extract(
    pool: &ExtractorPool,
    path: &Path,
    timeout: Duration,
) -> (ExtractEnd, Vec<ExtractEvent>) {
    let never = || false;
    let mut events = Vec::new();
    let end = pool.extract(
        path,
        &ExtractOptions::default(),
        timeout,
        &never,
        &mut events,
    );
    (end, events)
}

/// Vero se gli eventi contengono un'unità con `word`.
fn has_word(events: &[ExtractEvent], word: &str) -> bool {
    events
        .iter()
        .any(|e| matches!(e, ExtractEvent::Unit(unit) if unit.text.contains(word)))
}

/// Il thread del pool (avvii e chiusure dei processi) finisce con `shutdown`: dopo, il
/// lanciatore che teneva resta solo al test.
#[test]
fn shutdown_ends_the_pool_thread() {
    let dir = dir_with(&[]);
    let probe = launcher();
    let pool = ExtractorPool::new(Arc::clone(&probe), NonZeroUsize::MIN);
    let (end, _) = extract(&pool, &dir.path().join("note.docx"), EXTRACT_TIMEOUT);
    assert_eq!(end, ExtractEnd::Completed);
    assert!(Arc::strong_count(&probe) > 1);
    pool.shutdown();
    assert_eq!(Arc::strong_count(&probe), 1, "thread del pool ancora vivo");
    // Dopo la chiusura nessun processo parte.
    let (end, _) = extract(&pool, &dir.path().join("note.docx"), EXTRACT_TIMEOUT);
    assert_eq!(end, ExtractEnd::Failed(WorkerFailure::Unavailable));
}

/// Un'unità oltre 1 MiB, o un avviso con più voci annidate della profondità massima, è una
/// violazione del protocollo: il processo si sostituisce e il documento dopo si legge.
#[test]
fn unit_over_1_mib_is_a_protocol_violation() {
    for name in ["unita-grande.pdf", "troppo-profondo.pdf"] {
        let dir = dir_with(&[name]);
        let pool = pool(1);
        let (end, _) = extract(&pool, &dir.path().join(name), EXTRACT_TIMEOUT);
        assert_eq!(
            end,
            ExtractEnd::Failed(WorkerFailure::ProtocolViolation),
            "{name}"
        );
        let (end, events) = extract(&pool, &dir.path().join("note.docx"), EXTRACT_TIMEOUT);
        assert_eq!(end, ExtractEnd::Completed, "{name}");
        assert!(has_word(&events, "carta"), "{name}: {events:?}");
        assert_eq!(pool.launches(), 2, "{name}: processo non sostituito");
    }
}

/// Un frame che dichiara più del massimo del protocollo (qui 16 MiB) è una violazione appena
/// se ne legge la lunghezza: il thread che legge non lo alloca e non aspetta i suoi byte
/// (che non arrivano: se li aspettasse, l'esito sarebbe la scadenza).
#[test]
fn frame_over_the_maximum_is_a_protocol_violation() {
    let name = "frame-enorme.pdf";
    let dir = dir_with(&[name]);
    let pool = pool(1);
    let (end, _) = extract(&pool, &dir.path().join(name), EXTRACT_TIMEOUT);
    assert_eq!(end, ExtractEnd::Failed(WorkerFailure::ProtocolViolation));
}

/// Un processo che manda unità valide senza fine finisce per scadenza, entro la scadenza.
#[test]
fn flooding_worker_times_out_within_budget() {
    struct Discard(u64);
    impl lynkeus_extract::EventSink for Discard {
        fn emit(&mut self, _event: ExtractEvent) -> Result<(), Stopped> {
            self.0 += 1;
            Ok(())
        }
    }
    let dir = dir_with(&["inonda.pdf"]);
    let pool = pool(1);
    let timeout = Duration::from_secs(2);
    let never = || false;
    let mut sink = Discard(0);
    let started = Instant::now();
    let end = pool.extract(
        &dir.path().join("inonda.pdf"),
        &ExtractOptions::default(),
        timeout,
        &never,
        &mut sink,
    );
    let took = started.elapsed();
    assert_eq!(end, ExtractEnd::Failed(WorkerFailure::Timeout));
    assert!(sink.0 > 1, "nessuna unità ricevuta");
    assert!(took < timeout + STOP_SLACK, "scadenza dopo {took:?}");
}

/// Un processo che parla da libero (un secondo `Done` dopo la fine) non si riusa, e il
/// documento successivo si legge comunque, senza avvisi.
#[test]
fn worker_speaking_while_idle_is_replaced() {
    let dir = dir_with(&["chiacchierone.pdf"]);
    let pool = pool(1);
    let (end, _) = extract(
        &pool,
        &dir.path().join("chiacchierone.pdf"),
        EXTRACT_TIMEOUT,
    );
    assert_eq!(end, ExtractEnd::Completed);
    let (end, events) = extract(&pool, &dir.path().join("note.docx"), EXTRACT_TIMEOUT);
    assert_eq!(end, ExtractEnd::Completed, "{events:?}");
    assert!(has_word(&events, "carta"), "{events:?}");
    assert_eq!(pool.launches(), 2, "processo chiacchierone riusato");
}

/// I byte di una voce in risposta a un'estrazione sono fuori sequenza: violazione, processo
/// non riusato.
#[test]
fn entry_data_out_of_sequence_is_rejected() {
    let dir = dir_with(&["dati-fuori-sequenza.pdf"]);
    let pool = pool(1);
    let (end, _) = extract(
        &pool,
        &dir.path().join("dati-fuori-sequenza.pdf"),
        EXTRACT_TIMEOUT,
    );
    assert_eq!(end, ExtractEnd::Failed(WorkerFailure::ProtocolViolation));
    let (end, _) = extract(&pool, &dir.path().join("note.docx"), EXTRACT_TIMEOUT);
    assert_eq!(end, ExtractEnd::Completed);
    assert_eq!(pool.launches(), 2);
}

/// Un processo che non legge stdin: la richiesta (più grande del buffer del canale) non si
/// scrive mai. L'estrazione finisce comunque per scadenza, e prima con Annulla.
#[test]
fn a_worker_that_never_reads_its_request_is_timed_out_or_stopped() {
    let deaf: Arc<dyn lynkeus_engine::WorkerLauncher> = Arc::new(
        CommandLauncher::with_args(
            test_worker(),
            vec!["--extract-worker".into(), "--deaf".into()],
        )
        .expect("assoluto"),
    );
    // 1 MiB di nomi di voci: più del buffer dei canali anonimi (64 KiB su Linux, pochi KiB
    // su Windows), dentro il massimo di un frame.
    let inner: Vec<String> = (0..16)
        .map(|n| format!("{n}{}", "x".repeat(64 * 1024)))
        .collect();
    let timeout = Duration::from_secs(2);
    let cases: [(&str, Option<Duration>, EntryEnd); 2] = [
        ("scadenza", None, EntryEnd::Failed(WorkerFailure::Timeout)),
        (
            "annulla",
            Some(Duration::from_millis(200)),
            EntryEnd::Stopped,
        ),
    ];
    for (case, stop_after, expected) in cases {
        let launcher = Arc::clone(&deaf);
        let inner = inner.clone();
        let (done_tx, done_rx) = mpsc::channel();
        let started = Instant::now();
        // In un thread: se la scrittura resta bloccata il test fallisce invece di fermarsi.
        std::thread::spawn(move || {
            let pool = ExtractorPool::new(launcher, NonZeroUsize::MIN);
            let stop = || stop_after.is_some_and(|after| started.elapsed() > after);
            let mut bytes = Vec::new();
            let end = pool.extract_entry(
                Path::new("archivio.zip"),
                &inner,
                u64::MAX,
                &ExtractOptions::default(),
                timeout,
                &stop,
                &mut bytes,
            );
            let took = started.elapsed();
            // Il processo che non legge si chiude prima della fine del test.
            pool.shutdown();
            if done_tx.send((end, took)).is_err() {}
        });
        let (end, took) = done_rx
            .recv_timeout(timeout + Duration::from_secs(10))
            .unwrap_or_else(|_| panic!("{case}: scrittura della richiesta senza scadenza"));
        assert_eq!(end, expected, "{case}");
        assert!(took < timeout + STOP_SLACK, "{case}: {took:?}");
    }
}

/// Il tempo in cui l'interfaccia non legge gli eventi (coda piena) non conta nella scadenza
/// del processo estrattore: nessun "tempo scaduto" spurio.
#[test]
fn waiting_for_the_interface_does_not_count_toward_the_timeout() {
    let dir = dir_with(&["molti.pdf"]);
    let mut req = request(vec![dir.path().join("molti.pdf")], &["carta"], Combine::Any);
    req.limits.extract_timeout = SHORT_TIMEOUT;
    let source = documents_with(launcher(), 1, None);
    // L'avvio del processo resterebbe dentro la scadenza stretta: prima lo si avvia.
    warm_up(&source, dir.path());
    let (mut handle, wake) = start_with(req, Some(source));
    // Abbastanza eventi perché la coda verso l'interfaccia si avvicini al pieno; poi
    // l'interfaccia resta occupata più a lungo della scadenza.
    for _ in 0..200 {
        wake.recv_timeout(EXTRACT_TIMEOUT)
            .expect("eventi della ricerca");
    }
    std::thread::sleep(INTERFACE_PAUSE);
    let found = drain(&mut handle, &wake);
    assert_eq!(found.summary.outcome, Outcome::Completed);
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    assert_eq!(found.hits.len(), 400);
}

/// Anteprima senza cache: appena arrivate l'unità e le sue vicine l'estrazione si ferma,
/// invece di occupare il processo fino alla scadenza.
#[test]
fn preview_without_cache_stops_at_a_full_window() {
    let dir = dir_with(&["finestra.pdf"]);
    let source = documents_with(launcher(), 1, None);
    // Il tempo misurato è quello dell'anteprima, non dell'avvio del processo.
    warm_up(&source, dir.path());
    let doc = DocPath::file(dir.path().join("finestra.pdf"));
    let never = || false;
    let started = Instant::now();
    let units = source
        .units_around(
            &doc,
            &Location::PageLine { page: 1, line: 2 },
            1,
            &ExtractOptions::default(),
            PREVIEW_TIMEOUT,
            &never,
        )
        .expect("documento leggibile")
        .expect("unità trovata");
    let took = started.elapsed();
    let texts: Vec<&str> = units.iter().map(|u| u.text.as_str()).collect();
    assert_eq!(texts, ["riga 1 carta", "riga 2 carta", "riga 3 carta"]);
    assert!(took < PREVIEW_BUDGET, "anteprima in {took:?}");
    // Il processo si libera: il documento dopo si legge.
    let mut events = Vec::new();
    let end = source.pool().extract(
        &dir.path().join("note.docx"),
        &ExtractOptions::default(),
        EXTRACT_TIMEOUT,
        &never,
        &mut events,
    );
    assert_eq!(end, ExtractEnd::Completed);
}
