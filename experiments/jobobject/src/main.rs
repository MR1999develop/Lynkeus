// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Prototipo usa e getta: processo figlio (lo stesso eseguibile) in un Job Object.
//!
//! Verifica, con l'eseguibile avviato da una console:
//! - `test-mem`: un figlio che alloca oltre il limite di memoria per processo fallisce
//!   (`try_reserve` restituisce errore, l'allocazione normale termina il processo);
//! - `test-kill`: chiudendo il padre (uscita normale o `taskkill /F`) il figlio muore;
//! - `test-env`: il figlio parte con `env_clear` più `SystemRoot` e con i soli handle standard;
//! - `bench <n>`: costo di avvio di un figlio fino alla prima risposta, con e senza job;
//! - `wrap <MiB> <programma> [argomenti]`: esegue un programma qualsiasi nel job con il limite
//!   (per provare un estrattore PDF su una bomba di decompressione);
//! - `all`: i primi quattro in fila.
//!
//! Modi interni: `child` (il lavoratore) e `hold <job|nojob>` (padre intermedio da terminare).

#[cfg(windows)]
mod ffi;

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    imp::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("prototipo solo per Windows");
}

#[cfg(windows)]
mod imp {
    use std::io::{BufRead, BufReader, Write};
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::{CommandExt, ExitStatusExt};
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdin, ChildStdout, Command, ExitCode, ExitStatus, Stdio};
    use std::time::{Duration, Instant};

    use win32job::{ExtendedLimitInfo, Job};

    use crate::ffi;

    const MIB: usize = 1 << 20;
    /// `CREATE_SUSPENDED`: il figlio non esegue nulla finché non è nel job.
    const CREATE_SUSPENDED: u32 = 0x0000_0004;
    /// Limite di memoria per processo usato nelle prove.
    const LIMIT_MIB: usize = 256;

    pub fn main() -> ExitCode {
        let args: Vec<String> = std::env::args().collect();
        let result = match args.get(1).map(String::as_str) {
            Some("child") => child_main(),
            Some("hold") => hold(args.get(2).map(String::as_str) == Some("job")),
            Some("test-mem") => test_mem(),
            Some("test-kill") => test_kill(),
            Some("test-env") => test_env(),
            Some("test-raw") => test_raw(),
            Some("bench") => bench(args.get(2).and_then(|n| n.parse().ok()).unwrap_or(50)),
            Some("wrap") if args.len() >= 4 => wrap(&args[2], &args[3], &args[4..]),
            Some("all") => test_mem()
                .and_then(|()| test_kill())
                .and_then(|()| test_env())
                .and_then(|()| test_raw())
                .and_then(|()| bench(50)),
            _ => Err(
                "uso: jobobject-experiment test-mem | test-kill | test-env | test-raw | bench <n> | wrap <MiB> <programma> [arg] | all"
                    .into(),
            ),
        };
        match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("errore: {e}");
                ExitCode::FAILURE
            }
        }
    }

    type Res = Result<(), Box<dyn std::error::Error>>;

    // ---------------------------------------------------------------- figlio

    /// Lavoratore: non fa nulla finché non riceve un comando su stdin; esce alla chiusura di stdin.
    fn child_main() -> Res {
        let stdin = std::io::stdin();
        let mut out = std::io::stdout().lock();
        for line in stdin.lock().lines() {
            let line = line?;
            let mut parts = line.split_whitespace();
            let reply = match (
                parts.next(),
                parts.next().and_then(|n| n.parse::<usize>().ok()),
            ) {
                (Some("ping"), _) => "pong".to_owned(),
                (Some("hang"), _) => {
                    // Simula un decoder bloccato: non legge più stdin, quindi non vede la chiusura.
                    writeln!(out, "bloccato")?;
                    out.flush()?;
                    std::thread::sleep(Duration::from_secs(300));
                    "sveglio".to_owned()
                }
                (Some("env"), _) => {
                    let vars: Vec<String> = std::env::vars_os()
                        .map(|(k, v)| format!("{}={}", k.to_string_lossy(), v.to_string_lossy()))
                        .collect();
                    format!("env [{}]", vars.join(", "))
                }
                (Some("handles"), _) => {
                    format!("handles [{}]", handle_list(false))
                }
                (Some("check"), Some(value)) => format!(
                    "check {value:#x} {}",
                    if ffi::handle_is_open(value) {
                        "APERTO"
                    } else {
                        "chiuso"
                    }
                ),
                (Some("try"), Some(mib)) => {
                    let mut v: Vec<u8> = Vec::new();
                    match v.try_reserve_exact(mib.saturating_mul(MIB)) {
                        Ok(()) => {
                            v.resize(mib * MIB, 1);
                            format!("try {mib} MiB ok")
                        }
                        Err(e) => format!("try {mib} MiB errore: {e}"),
                    }
                }
                (Some("alloc"), Some(mib)) => {
                    let v = vec![1u8; mib * MIB];
                    format!(
                        "alloc {mib} MiB ok ({})",
                        v.iter()
                            .step_by(4096)
                            .map(|&b| usize::from(b))
                            .sum::<usize>()
                    )
                }
                _ => format!("sconosciuto: {line}"),
            };
            writeln!(out, "{reply}")?;
            out.flush()?;
        }
        Ok(())
    }

    fn handle_list(only_inheritable: bool) -> String {
        let hs: Vec<String> = ffi::file_like_handles(only_inheritable)
            .iter()
            .map(|(v, k)| format!("{v:#x}:{k}"))
            .collect();
        hs.join(", ")
    }

    /// Elementi di una lista `nome [a, b, c]` stampata dal figlio.
    fn bracket_items(s: &str) -> Vec<String> {
        let body = s
            .split_once('[')
            .map_or("", |(_, b)| b.trim_end_matches(']'));
        body.split(", ")
            .filter(|x| !x.is_empty())
            .map(str::to_owned)
            .collect()
    }

    // ---------------------------------------------------------------- avvio

    /// Lavoratore avviato e collegato: stdin e stdout in pipe, stderr nullo.
    struct Worker {
        child: Child,
        stdin: Option<ChildStdin>,
        stdout: BufReader<ChildStdout>,
    }

    impl Worker {
        fn ask(&mut self, line: &str) -> Result<String, Box<dyn std::error::Error>> {
            let stdin = self.stdin.as_mut().ok_or("stdin chiuso")?;
            writeln!(stdin, "{line}")?;
            stdin.flush()?;
            let mut reply = String::new();
            if self.stdout.read_line(&mut reply)? == 0 {
                return Err("il figlio ha chiuso stdout".into());
            }
            Ok(reply.trim_end().to_owned())
        }

        /// Chiude stdin (richiesta di stop) e aspetta l'uscita.
        fn finish(mut self) -> std::io::Result<ExitStatus> {
            drop(self.stdin.take());
            self.child.wait()
        }
    }

    /// Avvia `program` come nel prodotto: percorso assoluto, ambiente svuotato tranne `SystemRoot`,
    /// stdin/stdout in pipe e stderr nullo. Con un job il figlio parte sospeso, entra nel job e solo
    /// dopo riprende: nessuna istruzione gira fuori dal job.
    fn spawn_in(
        program: &Path,
        args: &[&str],
        job: Option<&Job>,
        stdout: Stdio,
    ) -> std::io::Result<Child> {
        spawn_with(program, args, job, job.is_some(), stdout)
    }

    /// Come [`spawn_in`], con la sospensione scelta a parte (solo per misurarne il costo).
    fn spawn_with(
        program: &Path,
        args: &[&str],
        job: Option<&Job>,
        suspend: bool,
        stdout: Stdio,
    ) -> std::io::Result<Child> {
        let mut cmd = Command::new(program);
        cmd.args(args).env_clear();
        if let Some(root) = std::env::var_os("SystemRoot") {
            cmd.env("SystemRoot", root);
        }
        cmd.stdin(Stdio::piped())
            .stdout(stdout)
            .stderr(Stdio::null());
        if suspend {
            cmd.creation_flags(CREATE_SUSPENDED);
        }
        let mut child = cmd.spawn()?;
        let mut setup = Ok(());
        if let Some(job) = job {
            setup = job
                .assign_process(child.as_raw_handle() as isize)
                .map_err(std::io::Error::other);
        }
        if suspend && setup.is_ok() {
            setup = ffi::resume_main_thread(&child);
        }
        if let Err(e) = setup {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
        Ok(child)
    }

    fn spawn_worker(job: Option<&Job>) -> Result<Worker, Box<dyn std::error::Error>> {
        spawn_worker_with(job, job.is_some())
    }

    fn spawn_worker_with(
        job: Option<&Job>,
        suspend: bool,
    ) -> Result<Worker, Box<dyn std::error::Error>> {
        let exe = std::env::current_exe()?;
        let mut child = spawn_with(&exe, &["child"], job, suspend, Stdio::piped())?;
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().ok_or("stdout assente")?);
        Ok(Worker {
            child,
            stdin,
            stdout,
        })
    }

    /// Job con `KILL_ON_JOB_CLOSE` e limite di memoria impegnata per processo.
    fn limited_job(limit_mib: usize) -> Result<Job, Box<dyn std::error::Error>> {
        let job = Job::create()?;
        ffi::set_kill_and_process_memory_limit(&job, limit_mib * MIB)?;
        Ok(job)
    }

    fn describe(status: ExitStatus) -> String {
        match status.code() {
            Some(code) => format!("codice {code} ({:#010x})", code.cast_unsigned()),
            None => format!("{status}"),
        }
    }

    // ---------------------------------------------------------------- prove

    fn test_mem() -> Res {
        println!("## test-mem (limite per processo {LIMIT_MIB} MiB)");
        let job = limited_job(LIMIT_MIB)?;
        let mut w = spawn_worker(Some(&job))?;
        println!("- {}", w.ask("try 128")?);
        println!("- {}", w.ask("try 512")?);
        println!(
            "- {} (il processo resta vivo dopo l'errore)",
            w.ask("ping")?
        );
        let t = Instant::now();
        let reply = w
            .ask("alloc 512")
            .unwrap_or_else(|e| format!("nessuna risposta ({e})"));
        let status = w.finish()?;
        println!(
            "- alloc 512 MiB: {reply}; uscita {} dopo {:.0} ms",
            describe(status),
            ms(t.elapsed())
        );
        println!(
            "- picco di memoria impegnata di un processo nel job: {:.1} MiB",
            mib(ffi::peak_process_memory(&job)?)
        );

        // Controllo: il solo limite del working set (l'unico offerto da win32job) non fa fallire nulla.
        let mut info = ExtendedLimitInfo::new();
        info.limit_working_memory(MIB, 64 * MIB)
            .limit_kill_on_job_close();
        match Job::create_with_limit_info(&info) {
            Ok(ws_job) => {
                let mut w = spawn_worker(Some(&ws_job))?;
                println!(
                    "- solo limite del working set a 64 MiB: {}",
                    w.ask("try 512")?
                );
                w.finish()?;
            }
            Err(e) => println!("- limite del working set non impostabile: {e}"),
        }
        Ok(())
    }

    fn test_kill() -> Res {
        println!("## test-kill");
        // Uscita normale del proprietario del job: si chiude l'ultimo handle del job.
        let job = limited_job(LIMIT_MIB)?;
        let mut w = spawn_worker(Some(&job))?;
        w.ask("ping")?;
        let pid = w.child.id();
        let t = Instant::now();
        drop(job);
        let died = wait_dead(pid, Duration::from_secs(5));
        println!(
            "- handle del job chiuso, stdin del figlio ancora aperto: figlio morto = {died} dopo {:.1} ms",
            ms(t.elapsed())
        );
        let _ = w.finish();

        // Senza job un figlio che legge stdin esce da solo quando il padre muore (EOF).
        let mut w = spawn_worker(None)?;
        w.ask("ping")?;
        let pid = w.child.id();
        let t = Instant::now();
        drop(w.stdin.take());
        let died = wait_dead(pid, Duration::from_secs(5));
        println!(
            "- senza job, stdin chiuso a un figlio in attesa: uscito = {died} dopo {:.1} ms",
            ms(t.elapsed())
        );
        let _ = w.finish();

        for with_job in [true, false] {
            let exe = std::env::current_exe()?;
            let mut hold = Command::new(&exe)
                .arg("hold")
                .arg(if with_job { "job" } else { "nojob" })
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()?;
            let mut line = String::new();
            BufReader::new(hold.stdout.take().ok_or("stdout assente")?).read_line(&mut line)?;
            let child_pid: u32 = line
                .trim()
                .strip_prefix("child ")
                .ok_or("risposta inattesa")?
                .parse()?;
            let t = Instant::now();
            let status = taskkill(hold.id())?;
            hold.wait()?;
            let died = wait_dead(child_pid, Duration::from_secs(5));
            println!(
                "- padre {} terminato con taskkill /F ({}): figlio morto = {died} dopo {:.1} ms",
                if with_job {
                    "con job"
                } else {
                    "senza job (controllo)"
                },
                describe(status),
                ms(t.elapsed())
            );
            if !died {
                taskkill(child_pid)?;
            }
        }
        Ok(())
    }

    /// Padre intermedio: avvia un figlio, lo blocca (`hang`), ne stampa il PID e aspetta di essere
    /// terminato. Il figlio bloccato non legge stdin: senza job sopravvive al padre.
    fn hold(with_job: bool) -> Res {
        let job = if with_job {
            Some(limited_job(LIMIT_MIB)?)
        } else {
            None
        };
        let mut w = spawn_worker(job.as_ref())?;
        w.ask("hang")?;
        println!("child {}", w.child.id());
        std::io::stdout().flush()?;
        std::thread::sleep(Duration::from_secs(120));
        Ok(())
    }

    fn test_env() -> Res {
        println!("## test-env");
        // Un file aperto da std non è ereditabile; un secondo reso ereditabile simula una fuga.
        let dir = std::env::temp_dir();
        let normal = std::fs::File::create(dir.join("jobobject-exp-normale.tmp"))?;
        let leaked = std::fs::File::create(dir.join("jobobject-exp-ereditabile.tmp"))?;
        ffi::make_inheritable(&leaked)?;
        let job = limited_job(LIMIT_MIB)?;
        let parent_inheritable = handle_list(true);
        println!("- padre, handle ereditabili prima dell'avvio: [{parent_inheritable}]");
        let mut w = spawn_worker(Some(&job))?;
        println!("- figlio, {}", w.ask("env")?);
        let child_handles = w.ask("handles")?;
        println!("- figlio, {child_handles}");
        let parent_set = bracket_items(&format!("[{parent_inheritable}]"));
        let child_set = bracket_items(&child_handles);
        let inherited = parent_set.iter().filter(|h| child_set.contains(h)).count();
        let own: Vec<&String> = child_set
            .iter()
            .filter(|h| !parent_set.contains(h))
            .collect();
        println!(
            "- ereditabili del padre presenti nel figlio con lo stesso valore e tipo: {inherited} su {}",
            parent_set.len()
        );
        println!("- handle file/pipe/char del figlio non ereditati dal padre: {own:?}");
        let normal_value = format!("{:#x}:", normal.as_raw_handle() as usize);
        println!(
            "- file aperto da std nel padre ({normal_value} non ereditabile) tra gli handle file del figlio: {}",
            child_set.iter().any(|h| h.starts_with(&normal_value))
        );
        println!(
            "- file reso ereditabile nel padre: {}",
            w.ask(&format!("check {}", leaked.as_raw_handle() as usize))?
        );
        w.finish()?;
        drop((normal, leaked));
        let _ = std::fs::remove_file(dir.join("jobobject-exp-normale.tmp"));
        let _ = std::fs::remove_file(dir.join("jobobject-exp-ereditabile.tmp"));
        Ok(())
    }

    fn bench(n: usize) -> Res {
        println!("## bench ({n} avvii per variante, tempo fino alla prima risposta)");
        let variants = [
            ("senza job", false),
            ("senza job, sospeso e ripreso (Toolhelp)", true),
            (
                "job condiviso, assegnato dopo l'avvio senza sospensione",
                false,
            ),
            ("job condiviso, sospeso, assegnato, ripreso", true),
            ("job nuovo per figlio, sospeso, assegnato, ripreso", true),
        ];
        for (variant, suspend) in variants {
            let shared = limited_job(LIMIT_MIB)?;
            let mut times = Vec::with_capacity(n);
            for _ in 0..n {
                let t = Instant::now();
                let own;
                let job = if variant.starts_with("senza job") {
                    None
                } else if variant.starts_with("job condiviso") {
                    Some(&shared)
                } else {
                    own = limited_job(LIMIT_MIB)?;
                    Some(&own)
                };
                let mut w = spawn_worker_with(job, suspend)?;
                w.ask("ping")?;
                times.push(ms(t.elapsed()));
                w.finish()?;
            }
            times.sort_by(f64::total_cmp);
            let pct = |p: usize| times[(times.len() - 1) * p / 100];
            println!(
                "- {variant}: mediana {:.1} ms, p90 {:.1} ms, min {:.1} ms, max {:.1} ms",
                pct(50),
                pct(90),
                times[0],
                times[times.len() - 1]
            );
        }
        Ok(())
    }

    /// Lavoratore avviato con `CreateProcessW` e liste di attributi (handle e job).
    struct RawWorker {
        child: ffi::RawChild,
        stdout: BufReader<std::fs::File>,
    }

    impl RawWorker {
        fn spawn(job: &Job) -> Result<Self, Box<dyn std::error::Error>> {
            let exe = std::env::current_exe()?;
            let child = ffi::spawn_raw(&exe, "child", job)?;
            let stdout = BufReader::new(child.stdout.try_clone()?);
            Ok(Self { child, stdout })
        }

        fn ask(&mut self, line: &str) -> Result<String, Box<dyn std::error::Error>> {
            writeln!(self.child.stdin, "{line}")?;
            let mut reply = String::new();
            if self.stdout.read_line(&mut reply)? == 0 {
                return Err("il figlio ha chiuso stdout".into());
            }
            Ok(reply.trim_end().to_owned())
        }

        /// Chiude stdin e aspetta il codice d'uscita.
        fn finish(self) -> std::io::Result<u32> {
            let RawWorker { child, stdout } = self;
            drop(stdout);
            let ffi::RawChild {
                process,
                stdin,
                stdout,
                ..
            } = child;
            drop((stdin, stdout));
            ffi::wait_exit(&process)
        }
    }

    fn test_raw() -> Res {
        println!("## test-raw (CreateProcessW con JOB_LIST e HANDLE_LIST)");
        let dir = std::env::temp_dir();
        let leaked = std::fs::File::create(dir.join("jobobject-exp-ereditabile.tmp"))?;
        ffi::make_inheritable(&leaked)?;
        let parent_inheritable = handle_list(true);
        println!("- padre, handle ereditabili: [{parent_inheritable}]");
        let job = limited_job(LIMIT_MIB)?;
        let mut w = RawWorker::spawn(&job)?;
        println!(
            "- figlio nel job fin dalla creazione: {}",
            ffi::process_in_job(&w.child.process, &job)?
        );
        println!("- figlio, {}", w.ask("env")?);
        let child_handles = w.ask("handles")?;
        println!("- figlio, {child_handles}");
        let parent_set = bracket_items(&format!("[{parent_inheritable}]"));
        let child_set = bracket_items(&child_handles);
        let inherited = parent_set.iter().filter(|h| child_set.contains(h)).count();
        println!(
            "- ereditabili del padre presenti nel figlio con lo stesso valore e tipo: {inherited} su {}",
            parent_set.len()
        );
        let leaked_value = format!("{:#x}:", leaked.as_raw_handle() as usize);
        println!(
            "- file reso ereditabile nel padre ({leaked_value}) tra gli handle file del figlio: {}",
            child_set.iter().any(|h| h.starts_with(&leaked_value))
        );
        println!("- {}", w.ask("try 512")?);
        let code = w.finish()?;
        println!("- uscita dopo la chiusura di stdin: {code}");
        let mut w = RawWorker::spawn(&job)?;
        let reply = w
            .ask("alloc 512")
            .unwrap_or_else(|e| format!("nessuna risposta ({e})"));
        println!("- alloc 512 MiB: {reply}; codice {:#010x}", w.finish()?);
        drop(leaked);
        let _ = std::fs::remove_file(dir.join("jobobject-exp-ereditabile.tmp"));

        let n = 50;
        let mut times = Vec::with_capacity(n);
        for _ in 0..n {
            let t = Instant::now();
            let mut w = RawWorker::spawn(&job)?;
            w.ask("ping")?;
            times.push(ms(t.elapsed()));
            w.finish()?;
        }
        times.sort_by(f64::total_cmp);
        println!(
            "- avvio fino alla prima risposta ({n} avvii, job condiviso): mediana {:.1} ms, p90 {:.1} ms, min {:.1} ms, max {:.1} ms",
            times[n / 2],
            times[(n - 1) * 9 / 10],
            times[0],
            times[n - 1]
        );
        Ok(())
    }

    fn wrap(limit: &str, program: &str, args: &[String]) -> Res {
        let limit_mib: usize = limit.parse()?;
        let job = limited_job(limit_mib)?;
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let t = Instant::now();
        let mut child = spawn_in(&PathBuf::from(program), &args, Some(&job), Stdio::piped())?;
        drop(child.stdin.take());
        let mut out = String::new();
        if let Some(mut so) = child.stdout.take() {
            std::io::Read::read_to_string(&mut so, &mut out)?;
        }
        let status = child.wait()?;
        let tail: Vec<&str> = out.lines().filter(|l| !l.starts_with("  |")).collect();
        println!(
            "uscita {} dopo {:.0} ms; picco {:.1} MiB",
            describe(status),
            ms(t.elapsed()),
            mib(ffi::peak_process_memory(&job)?)
        );
        println!("{}", tail.join("\n"));
        Ok(())
    }

    // ---------------------------------------------------------------- utilità

    /// `taskkill /F /PID` dal percorso assoluto di sistema.
    fn taskkill(pid: u32) -> std::io::Result<ExitStatus> {
        let root = std::env::var_os("SystemRoot")
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        Command::new(root.join(r"System32\taskkill.exe"))
            .args(["/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| ExitStatus::from_raw(s.code().unwrap_or(-1).cast_unsigned()))
    }

    /// Aspetta che `pid` termini; vero se è morto entro `limit`.
    fn wait_dead(pid: u32, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if !ffi::process_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        !ffi::process_alive(pid)
    }

    fn ms(d: Duration) -> f64 {
        d.as_secs_f64() * 1000.0
    }

    #[expect(clippy::cast_precision_loss, reason = "MiB indicativi")]
    fn mib(bytes: usize) -> f64 {
        bytes as f64 / MIB as f64
    }
}
