// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Le sole chiamate Win32 dirette dell'app, con un'API safe. Unico modulo con `unsafe` nel
//! workspace (il crate ha `#![deny(unsafe_code)]`; qui il permesso è mirato).
//!
//! - [`restrict_dll_search`]: `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)`, prima
//!   istruzione di `main`: nessuna DLL caricata dalla cartella dell'eseguibile, dalla cartella
//!   corrente o dal `PATH`.
//! - [`Job`]: Job Object con `KILL_ON_JOB_CLOSE`, limite di memoria impegnata per processo
//!   (`JOB_OBJECT_LIMIT_PROCESS_MEMORY`, che `win32job` non offre) e porta di completamento
//!   per sapere se un processo ha superato il limite (il codice d'uscita non lo distingue da
//!   un aborto qualsiasi: experiments/jobobject/ESITI.md).
//! - [`spawn_in_job`]: `CreateProcessW` con `PROC_THREAD_ATTRIBUTE_JOB_LIST` (il processo
//!   nasce già nel job) e `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (eredita solo stdin e stdout),
//!   percorso assoluto, ambiente con la sola `SystemRoot`, nessuna console, cartella corrente =
//!   cartella dell'eseguibile.
//!
//! Ogni blocco `unsafe` ha il suo `// SAFETY:`; gli handle sono posseduti da `OwnedHandle`
//! (chiusi una volta sola) e i puntatori passati vivono fino al ritorno della chiamata.
#![allow(
    unsafe_code,
    reason = "unico modulo FFI dell'app: API safe sopra Win32 (PLAN.md, Architettura)"
)]

use std::ffi::{OsStr, OsString, c_void};
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::IO::{
    CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED,
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    JOBOBJECT_ASSOCIATE_COMPLETION_PORT, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectAssociateCompletionPortInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::SystemServices::JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT;
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DETACHED_PROCESS, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};

/// Codice d'uscita dato ai processi terminati da qui.
const KILLED_EXIT_CODE: u32 = 1;

/// Limita la ricerca delle DLL caricate da qui in poi alla cartella di sistema.
pub fn restrict_dll_search() -> io::Result<()> {
    // SAFETY: nessun puntatore; la funzione cambia solo un'impostazione del processo.
    let ok = unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn struct_len<T>() -> u32 {
    u32::try_from(size_of::<T>()).unwrap_or(u32::MAX)
}

/// Un handle appena restituito da Win32, o l'ultimo errore se nullo o non valido.
fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: handle valido appena creato, posseduto solo da qui: `OwnedHandle` lo chiude una
    // volta sola.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// Un Job Object con un solo processo estrattore.
///
/// Chiuderlo (drop) termina il processo (`KILL_ON_JOB_CLOSE`); il sistema lo chiude anche se
/// l'app termina in modo anomalo, quindi nessun estrattore sopravvive all'app.
#[derive(Debug)]
pub struct Job {
    handle: OwnedHandle,
    port: OwnedHandle,
    memory_limit_hit: bool,
}

impl Job {
    /// Un job con limite di `memory_limit` byte di memoria impegnata per processo.
    pub fn new(memory_limit: usize) -> io::Result<Job> {
        // SAFETY: attributi nulli (handle non ereditabile) e nessun nome.
        let handle = owned(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) })?;
        // SAFETY: struttura C di soli interi, lo zero è un valore valido.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY
            | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
        info.ProcessMemoryLimit = memory_limit;
        // SAFETY: handle del job valido; puntatore e lunghezza della struttura corretti, la
        // struttura vive fino al ritorno.
        let ok = unsafe {
            SetInformationJobObject(
                handle.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw const info).cast::<c_void>(),
                struct_len::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: nuova porta non associata a file (INVALID_HANDLE_VALUE), un thread.
        let port = owned(unsafe {
            CreateIoCompletionPort(INVALID_HANDLE_VALUE, std::ptr::null_mut(), 0, 1)
        })?;
        let association = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
            CompletionKey: std::ptr::null_mut(),
            CompletionPort: port.as_raw_handle(),
        };
        // SAFETY: handle del job e della porta validi; struttura viva fino al ritorno.
        let ok = unsafe {
            SetInformationJobObject(
                handle.as_raw_handle(),
                JobObjectAssociateCompletionPortInformation,
                (&raw const association).cast::<c_void>(),
                struct_len::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Job {
            handle,
            port,
            memory_limit_hit: false,
        })
    }

    /// Vero se un processo del job ha provato a superare il limite di memoria (messaggio
    /// `JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT` sulla porta), anche se poi è sopravvissuto.
    pub fn memory_limit_hit(&mut self) -> bool {
        loop {
            let mut message = 0u32;
            let mut key = 0usize;
            let mut overlapped: *mut OVERLAPPED = std::ptr::null_mut();
            // SAFETY: porta valida; puntatori d'uscita validi; attesa zero (non blocca).
            let got = unsafe {
                GetQueuedCompletionStatus(
                    self.port.as_raw_handle(),
                    &raw mut message,
                    &raw mut key,
                    &raw mut overlapped,
                    0,
                )
            };
            if got == 0 {
                return self.memory_limit_hit;
            }
            if message == JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT {
                self.memory_limit_hit = true;
            }
        }
    }

    /// Picco di memoria impegnata da un processo del job, in byte.
    pub fn peak_process_memory(&self) -> io::Result<usize> {
        // SAFETY: struttura C di soli interi, lo zero è un valore valido.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: handle valido; buffer scrivibile della lunghezza dichiarata.
        let ok = unsafe {
            QueryInformationJobObject(
                self.handle.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw mut info).cast::<c_void>(),
                struct_len::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>(),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(info.PeakProcessMemoryUsed)
        }
    }

    /// Termina tutti i processi del job.
    pub fn terminate(&self) -> io::Result<()> {
        // SAFETY: handle valido preso in prestito.
        if unsafe { TerminateJobObject(self.handle.as_raw_handle(), KILLED_EXIT_CODE) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Un processo avviato da [`spawn_in_job`].
#[derive(Debug)]
pub struct ChildProcess {
    handle: OwnedHandle,
}

impl ChildProcess {
    /// Aspetta l'uscita al massimo `timeout`: il codice d'uscita, o `None` se è ancora vivo.
    pub fn wait(&self, timeout: Duration) -> io::Result<Option<u32>> {
        let millis = u32::try_from(timeout.as_millis())
            .unwrap_or(INFINITE - 1)
            .min(INFINITE - 1);
        // SAFETY: handle del processo valido preso in prestito.
        let waited = unsafe { WaitForSingleObject(self.handle.as_raw_handle(), millis) };
        match waited {
            WAIT_OBJECT_0 => {
                let mut code = 0u32;
                // SAFETY: handle valido, puntatore d'uscita valido.
                let ok = unsafe { GetExitCodeProcess(self.handle.as_raw_handle(), &raw mut code) };
                if ok == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(Some(code))
                }
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }

    /// Termina il processo; nessun errore se è già uscito.
    pub fn terminate(&self) -> io::Result<()> {
        // SAFETY: handle valido preso in prestito.
        if unsafe { TerminateProcess(self.handle.as_raw_handle(), KILLED_EXIT_CODE) } != 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        // Un processo già uscito risponde "accesso negato".
        let access_denied = i32::try_from(ERROR_ACCESS_DENIED).ok();
        if error.raw_os_error() == access_denied && matches!(self.wait(Duration::ZERO), Ok(Some(_)))
        {
            Ok(())
        } else {
            Err(error)
        }
    }

    /// Vero se il processo appartiene a `job`.
    pub fn is_in(&self, job: &Job) -> io::Result<bool> {
        let mut result = 0;
        // SAFETY: handle validi presi in prestito, puntatore d'uscita valido.
        let ok = unsafe {
            IsProcessInJob(
                self.handle.as_raw_handle(),
                job.handle.as_raw_handle(),
                &raw mut result,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(result != 0)
    }
}

/// Un processo appena avviato con i capi dei suoi canali.
#[derive(Debug)]
pub struct SpawnedChild {
    /// Il processo.
    pub process: ChildProcess,
    /// Il suo stdin (scrittura).
    pub stdin: File,
    /// Il suo stdout (lettura).
    pub stdout: File,
}

/// Pipe anonima con entrambi i capi non ereditabili: (lettura, scrittura).
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let mut read: HANDLE = std::ptr::null_mut();
    let mut write: HANDLE = std::ptr::null_mut();
    // SAFETY: puntatori d'uscita validi; attributi nulli = handle non ereditabili.
    if unsafe { CreatePipe(&raw mut read, &raw mut write, std::ptr::null(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((owned(read)?, owned(write)?))
}

/// Rende ereditabile un handle (serve per la lista degli handle di `CreateProcessW`).
fn make_inheritable(handle: &OwnedHandle) -> io::Result<()> {
    // SAFETY: handle valido preso in prestito.
    let ok = unsafe {
        SetHandleInformation(
            handle.as_raw_handle(),
            HANDLE_FLAG_INHERIT,
            HANDLE_FLAG_INHERIT,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Testo UTF-16 terminato da zero.
fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

/// Aggiunge `arg` alla riga di comando con le regole di `CommandLineToArgvW` e del runtime
/// C: tra virgolette se serve, con le barre rovesciate prima delle virgolette raddoppiate.
fn push_arg(line: &mut Vec<u16>, arg: &OsStr) {
    const QUOTE: u16 = b'"' as u16;
    const BACKSLASH: u16 = b'\\' as u16;
    let units: Vec<u16> = arg.encode_wide().collect();
    let needs_quotes = units.is_empty()
        || units
            .iter()
            .any(|&u| u == u16::from(b' ') || u == u16::from(b'\t') || u == QUOTE);
    if !needs_quotes {
        line.extend_from_slice(&units);
        return;
    }
    line.push(QUOTE);
    let mut backslashes = 0usize;
    for &unit in &units {
        if unit == BACKSLASH {
            backslashes += 1;
            continue;
        }
        let repeat = if unit == QUOTE {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        line.extend(std::iter::repeat_n(BACKSLASH, repeat));
        backslashes = 0;
        line.push(unit);
    }
    line.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2));
    line.push(QUOTE);
}

/// La riga di comando: l'eseguibile tra virgolette, poi gli argomenti.
fn command_line(exe: &Path, args: &[OsString]) -> Vec<u16> {
    let mut line = Vec::new();
    line.push(u16::from(b'"'));
    line.extend(exe.as_os_str().encode_wide());
    line.push(u16::from(b'"'));
    for arg in args {
        line.push(u16::from(b' '));
        push_arg(&mut line, arg);
    }
    line.push(0);
    line
}

/// L'ambiente del figlio: solo `SystemRoot`, in formato `CREATE_UNICODE_ENVIRONMENT`.
fn minimal_environment() -> Vec<u16> {
    let mut env = Vec::new();
    if let Some(root) = std::env::var_os("SystemRoot") {
        env.extend(OsStr::new("SystemRoot=").encode_wide());
        env.extend(root.encode_wide());
        env.push(0);
    }
    env.extend([0, 0]);
    env
}

/// Serializza gli avvii: tra il momento in cui i capi delle pipe diventano ereditabili e la
/// loro chiusura nessun altro avvio di questo modulo deve vederli. (Un avvio con
/// `std::process::Command` in un altro thread li erediterebbe: l'app non ne fa.)
static SPAWN_LOCK: Mutex<()> = Mutex::new(());

/// La lista degli attributi di `CreateProcessW`, liberata nel `Drop`.
struct AttributeList {
    storage: Vec<usize>,
}

impl AttributeList {
    /// Una lista inizializzata per `count` attributi.
    fn new(count: u32) -> io::Result<AttributeList> {
        let mut size = 0usize;
        // SAFETY: prima chiamata con lista nulla per sapere la dimensione; fallisce di
        // proposito e scrive solo `size`.
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), count, 0, &raw mut size) };
        let mut storage = vec![0usize; size.div_ceil(size_of::<usize>()).max(1)];
        // SAFETY: `storage` è allineato a `usize` e lungo almeno `size` byte.
        let ok = unsafe {
            InitializeProcThreadAttributeList(
                storage.as_mut_ptr().cast::<c_void>(),
                count,
                0,
                &raw mut size,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(AttributeList { storage })
    }

    fn as_ptr(&mut self) -> *mut c_void {
        self.storage.as_mut_ptr().cast::<c_void>()
    }

    /// Imposta un attributo che punta a `values`.
    ///
    /// I valori devono vivere finché la lista è usata da `CreateProcessW`: lo garantisce il
    /// chiamante tenendoli in vita oltre la chiamata (vedi [`spawn_in_job`]).
    fn set(&mut self, attribute: usize, values: &[HANDLE]) -> io::Result<()> {
        let list = self.as_ptr();
        // SAFETY: lista inizializzata; `values` è un array valido di `size_of_val` byte che il
        // chiamante mantiene in vita fino a dopo `CreateProcessW`.
        let ok = unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                attribute,
                values.as_ptr().cast::<c_void>(),
                size_of_val(values),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        let list = self.as_ptr();
        // SAFETY: lista inizializzata in `new`, cancellata una volta sola.
        unsafe { DeleteProcThreadAttributeList(list) };
    }
}

/// Avvia `exe` (percorso assoluto) con `args` dentro `job`, con solo stdin e stdout
/// ereditati, ambiente con la sola `SystemRoot`, senza console, nella cartella
/// dell'eseguibile.
pub fn spawn_in_job(exe: &Path, args: &[OsString], job: &Job) -> io::Result<SpawnedChild> {
    if !exe.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "eseguibile senza percorso assoluto",
        ));
    }
    let _serialized = SPAWN_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let (stdin_read, stdin_write) = pipe()?;
    let (stdout_read, stdout_write) = pipe()?;
    make_inheritable(&stdin_read)?;
    make_inheritable(&stdout_write)?;

    let inherit: [HANDLE; 2] = [stdin_read.as_raw_handle(), stdout_write.as_raw_handle()];
    let jobs: [HANDLE; 1] = [job.handle.as_raw_handle()];
    let mut attributes = AttributeList::new(2)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, &inherit)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST as usize, &jobs)?;

    // SAFETY: struttura C di interi e puntatori, lo zero è un valore valido.
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = struct_len::<STARTUPINFOEXW>();
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = stdin_read.as_raw_handle();
    startup.StartupInfo.hStdOutput = stdout_write.as_raw_handle();
    startup.StartupInfo.hStdError = std::ptr::null_mut();
    startup.lpAttributeList = attributes.as_ptr();

    let application = wide(exe.as_os_str());
    let mut line = command_line(exe, args);
    let environment = minimal_environment();
    let directory = wide(exe.parent().map_or(OsStr::new("."), Path::as_os_str));

    // SAFETY: struttura d'uscita, lo zero è un valore valido.
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: stringhe terminate da zero vive fino al ritorno; `line` è modificabile come
    // richiesto; `startup` è uno STARTUPINFOEXW con `cb` corretto e una lista d'attributi
    // valida, i cui valori (`inherit`, `jobs`) vivono fino a dopo la chiamata.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | DETACHED_PROCESS,
            environment.as_ptr().cast::<c_void>(),
            directory.as_ptr(),
            (&raw const startup).cast::<STARTUPINFOW>(),
            &raw mut info,
        )
    };
    let error = io::Error::last_os_error();
    drop(attributes);
    // I capi del figlio si chiudono subito nel padre: il figlio ha i suoi.
    drop(stdin_read);
    drop(stdout_write);
    if created == 0 {
        return Err(error);
    }
    // Il thread principale non serve; il processo resta posseduto da `ChildProcess`.
    drop(owned(info.hThread));
    let process = ChildProcess {
        handle: owned(info.hProcess)?,
    };
    Ok(SpawnedChild {
        process,
        stdin: File::from(stdin_write),
        stdout: File::from(stdout_read),
    })
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    /// Argomento che segna i figli avviati da questi test: senza, i test "figlio" non fanno
    /// nulla (anche se qualcuno li esegue con `--ignored`).
    const CHILD_MARKER: &str = "lynkeus-figlio-del-test-7c1d";

    fn is_child() -> bool {
        std::env::args().any(|a| a == CHILD_MARKER)
    }

    /// Avvia questo stesso eseguibile di test nel job, eseguendo solo il test `name`.
    fn spawn_test_child(name: &str, job: &Job) -> SpawnedChild {
        let exe = std::env::current_exe().expect("eseguibile del test");
        let args: Vec<OsString> = [
            format!("platform::windows_ffi::tests::{name}"),
            CHILD_MARKER.to_owned(),
            "--exact".to_owned(),
            "--ignored".to_owned(),
            "--test-threads=1".to_owned(),
            "--nocapture".to_owned(),
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        spawn_in_job(&exe, &args, job).expect("figlio avviato")
    }

    #[test]
    #[ignore = "figlio dei test del Job Object"]
    fn child_sleeps() {
        if is_child() {
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    #[test]
    #[ignore = "figlio dei test del Job Object"]
    fn child_allocates() {
        if is_child() {
            let big = vec![1u8; 512 * 1024 * 1024];
            std::hint::black_box(&big);
        }
    }

    #[test]
    #[ignore = "figlio dei test del Job Object"]
    fn child_prints_environment() {
        if is_child() {
            let names: Vec<String> = std::env::vars_os()
                .map(|(name, _)| name.to_string_lossy().into_owned())
                .collect();
            println!("AMBIENTE:{}", names.join(","));
        }
    }

    #[test]
    fn closing_the_job_kills_the_child() {
        let job = Job::new(256 * 1024 * 1024).expect("job");
        let child = spawn_test_child("child_sleeps", &job);
        assert!(child.process.is_in(&job).expect("IsProcessInJob"));
        assert_eq!(
            child.process.wait(Duration::from_millis(200)).ok(),
            Some(None)
        );
        drop(job);
        let code = child.process.wait(Duration::from_secs(5)).expect("attesa");
        assert!(code.is_some(), "figlio vivo dopo la chiusura del job");
    }

    #[test]
    fn exceeding_the_memory_limit_kills_and_is_recognized() {
        let mut job = Job::new(128 * 1024 * 1024).expect("job");
        let child = spawn_test_child("child_allocates", &job);
        let code = child.process.wait(Duration::from_secs(30)).expect("attesa");
        assert!(matches!(code, Some(c) if c != 0), "{code:?}");
        assert!(job.memory_limit_hit(), "limite non riconosciuto");
        assert!(job.peak_process_memory().expect("picco") <= 128 * 1024 * 1024);
    }

    #[test]
    fn the_child_sees_only_system_root() {
        let job = Job::new(256 * 1024 * 1024).expect("job");
        let mut child = spawn_test_child("child_prints_environment", &job);
        drop(child.stdin);
        let mut output = String::new();
        child
            .stdout
            .read_to_string(&mut output)
            .expect("lettura dello stdout");
        // libtest scrive il nome del test sulla stessa riga, prima della stampa del figlio.
        let line = output
            .split_once("AMBIENTE:")
            .and_then(|(_, rest)| rest.lines().next())
            .unwrap_or_else(|| panic!("nessuna riga dell'ambiente in {output:?}"));
        let names: Vec<&str> = line.split(',').filter(|n| !n.is_empty()).collect();
        assert!(
            !names.is_empty() && names.iter().all(|n| n.eq_ignore_ascii_case("SystemRoot")),
            "{names:?}"
        );
    }

    #[test]
    fn terminate_kills_and_is_idempotent() {
        let job = Job::new(256 * 1024 * 1024).expect("job");
        let child = spawn_test_child("child_sleeps", &job);
        child.process.terminate().expect("terminato");
        assert_eq!(
            child.process.wait(Duration::from_secs(5)).ok(),
            Some(Some(KILLED_EXIT_CODE))
        );
        assert!(child.process.terminate().is_ok());
        assert!(job.terminate().is_ok());
    }

    #[test]
    fn relative_paths_are_refused() {
        let job = Job::new(256 * 1024 * 1024).expect("job");
        let refused = spawn_in_job(Path::new("lynkeus-searcher.exe"), &[], &job);
        assert_eq!(
            refused.map(|_| ()).map_err(|e| e.kind()),
            Err(io::ErrorKind::InvalidInput)
        );
    }

    #[test]
    fn command_line_quoting() {
        let line = |args: &[&str]| {
            let args: Vec<OsString> = args.iter().map(OsString::from).collect();
            let wide = command_line(Path::new(r"C:\Program Files\L\l.exe"), &args);
            String::from_utf16_lossy(&wide[..wide.len() - 1])
        };
        assert_eq!(line(&[]), r#""C:\Program Files\L\l.exe""#);
        assert_eq!(
            line(&["--extract-worker", "--memory-limit-bytes", "1024"]),
            r#""C:\Program Files\L\l.exe" --extract-worker --memory-limit-bytes 1024"#
        );
        assert_eq!(
            line(&["a b", "", r#"x"y"#, r"c:\dir\ "]),
            r#""C:\Program Files\L\l.exe" "a b" "" "x\"y" "c:\dir\ ""#
        );
        assert_eq!(line(&[r"fine\"]), r#""C:\Program Files\L\l.exe" fine\"#);
        assert_eq!(line(&[r"a b\"]), r#""C:\Program Files\L\l.exe" "a b\\""#);
    }
}
