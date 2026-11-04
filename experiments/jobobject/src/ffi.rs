// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Chiamate Win32 che `win32job` 2.x non offre, con API safe. Prototipo: nel prodotto andrebbero in
//! `platform/windows_ffi.rs`.
#![expect(unsafe_code, reason = "modulo FFI del prototipo")]

use std::ffi::{OsStr, c_void};
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::process::Child;

use windows_sys::Win32::System::JobObjects::IsProcessInJob;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DETACHED_PROCESS, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, UpdateProcThreadAttribute,
};

use win32job::Job;
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetHandleInformation, GetLastError, HANDLE, HANDLE_FLAG_INHERIT,
    SetHandleInformation, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_TYPE_CHAR, FILE_TYPE_DISK, FILE_TYPE_PIPE, FILE_TYPE_UNKNOWN, GetFileType,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, OpenThread, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, ResumeThread,
    THREAD_SUSPEND_RESUME, WaitForSingleObject,
};

fn job_handle(job: &Job) -> HANDLE {
    // `win32job` espone l'handle come intero; il valore è un HANDLE valido finché vive `job`.
    job.handle() as HANDLE
}

fn struct_len<T>() -> u32 {
    u32::try_from(size_of::<T>()).unwrap_or(u32::MAX)
}

/// Imposta `KILL_ON_JOB_CLOSE` e il limite di memoria impegnata per processo (`ProcessMemoryLimit`).
/// `win32job` 2.0 offre solo il limite del working set, che non fa fallire le allocazioni.
pub fn set_kill_and_process_memory_limit(job: &Job, limit_bytes: usize) -> io::Result<()> {
    // SAFETY: struttura C di soli interi, lo zero è un valore valido.
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
    info.ProcessMemoryLimit = limit_bytes;
    // SAFETY: handle del job valido (preso in prestito da `job`), puntatore e lunghezza della struttura corretti.
    let ok = unsafe {
        SetInformationJobObject(
            job_handle(job),
            JobObjectExtendedLimitInformation,
            (&raw const info).cast::<c_void>(),
            struct_len::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Picco di memoria impegnata da un processo del job (`PeakProcessMemoryUsed`), in byte.
pub fn peak_process_memory(job: &Job) -> io::Result<usize> {
    // SAFETY: come sopra.
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: handle valido, buffer scrivibile della lunghezza dichiarata.
    let ok = unsafe {
        QueryInformationJobObject(
            job_handle(job),
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

/// Riprende i thread di un figlio avviato con `CREATE_SUSPENDED`.
///
/// `ChildExt::main_thread_handle` di std è instabile in Rust 1.99, quindi i thread del processo si
/// trovano con un'istantanea Toolhelp (un processo appena creato e sospeso ne ha uno solo).
pub fn resume_main_thread(child: &Child) -> io::Result<()> {
    let pid = child.id();
    // SAFETY: nessun puntatore; l'handle restituito si chiude sotto.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: struttura C di soli interi; `dwSize` va impostato prima della chiamata.
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = struct_len::<THREADENTRY32>();
    let mut resumed = 0usize;
    let mut result = Ok(());
    // SAFETY: istantanea valida e struttura con `dwSize` corretto.
    let mut more = unsafe { Thread32First(snapshot, &raw mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: id di thread dall'istantanea; handle chiuso subito dopo l'uso.
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if thread.is_null() {
                result = Err(io::Error::last_os_error());
            } else {
                // SAFETY: handle di thread valido con il diritto di ripresa.
                if unsafe { ResumeThread(thread) } == u32::MAX {
                    result = Err(io::Error::last_os_error());
                } else {
                    resumed += 1;
                }
                // SAFETY: handle nostro.
                unsafe { CloseHandle(thread) };
            }
        }
        // SAFETY: come sopra.
        more = unsafe { Thread32Next(snapshot, &raw mut entry) } != 0;
    }
    // SAFETY: handle nostro.
    unsafe { CloseHandle(snapshot) };
    result?;
    if resumed == 0 {
        Err(io::Error::other("nessun thread del figlio trovato"))
    } else {
        Ok(())
    }
}

/// Rende ereditabile un handle del processo corrente (simula una fuga d'handle per il test).
pub fn make_inheritable(handle: &impl AsRawHandle) -> io::Result<()> {
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

/// Vero se `value` è un handle aperto nel processo corrente.
pub fn handle_is_open(value: usize) -> bool {
    let mut flags = 0u32;
    // SAFETY: GetHandleInformation accetta qualsiasi valore e fallisce se non è un handle.
    unsafe { GetHandleInformation(value as HANDLE, &raw mut flags) != 0 }
}

/// Vero se `value` è un handle aperto ed ereditabile nel processo corrente.
pub fn handle_is_inheritable(value: usize) -> bool {
    let mut flags = 0u32;
    // SAFETY: come in `handle_is_open`.
    let ok = unsafe { GetHandleInformation(value as HANDLE, &raw mut flags) != 0 };
    ok && flags & HANDLE_FLAG_INHERIT != 0
}

/// Handle di tipo file, pipe o dispositivo a caratteri aperti nel processo (valori 4..=65536).
pub fn file_like_handles(only_inheritable: bool) -> Vec<(usize, &'static str)> {
    let mut found = Vec::new();
    for value in (4..=65_536usize).step_by(4) {
        if !handle_is_open(value) || (only_inheritable && !handle_is_inheritable(value)) {
            continue;
        }
        // SAFETY: handle aperto; GetFileType non blocca e non modifica l'handle.
        let kind = unsafe { GetFileType(value as HANDLE) };
        let name = match kind {
            FILE_TYPE_DISK => "file",
            FILE_TYPE_PIPE => "pipe",
            FILE_TYPE_CHAR => "char",
            FILE_TYPE_UNKNOWN => continue,
            _ => "altro",
        };
        found.push((value, name));
    }
    found
}

/// Figlio avviato con [`spawn_raw`]: handle del processo e capi della pipe del padre.
pub struct RawChild {
    pub process: OwnedHandle,
    #[expect(dead_code, reason = "il PID serve solo nei messaggi del prodotto")]
    pub pid: u32,
    pub stdin: File,
    pub stdout: File,
}

/// Pipe anonima con entrambi i capi non ereditabili.
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let mut read: HANDLE = std::ptr::null_mut();
    let mut write: HANDLE = std::ptr::null_mut();
    // SAFETY: puntatori di uscita validi; attributi nulli = handle non ereditabili.
    if unsafe { CreatePipe(&raw mut read, &raw mut write, std::ptr::null(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: handle appena creati e posseduti solo da qui.
    Ok(unsafe {
        (
            OwnedHandle::from_raw_handle(read),
            OwnedHandle::from_raw_handle(write),
        )
    })
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Avvia `exe arg` con `CreateProcessW` come proposto per il prodotto:
/// - `PROC_THREAD_ATTRIBUTE_JOB_LIST`: il processo nasce già nel job (niente finestra né sospensione);
/// - `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`: eredita solo il capo di lettura di stdin e quello di
///   scrittura di stdout, anche se il padre ha altri handle ereditabili; stderr assente;
/// - ambiente con la sola `SystemRoot`, nessuna console (`DETACHED_PROCESS`), cartella corrente = cartella dell'eseguibile.
pub fn spawn_raw(exe: &Path, arg: &str, job: &Job) -> io::Result<RawChild> {
    let (stdin_read, stdin_write) = pipe()?;
    let (stdout_read, stdout_write) = pipe()?;
    // La lista degli handle richiede che siano ereditabili; restano fuori dagli altri avvii solo se
    // nessun altro thread avvia processi con l'ereditarietà piena nel frattempo.
    make_inheritable(&stdin_read)?;
    make_inheritable(&stdout_write)?;

    let mut size = 0usize;
    // SAFETY: prima chiamata con lista nulla per sapere la dimensione; fallisce di proposito.
    unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 2, 0, &raw mut size) };
    let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
    let list = storage.as_mut_ptr().cast::<c_void>();
    // SAFETY: `storage` è allineato a `usize` e lungo almeno `size` byte.
    if unsafe { InitializeProcThreadAttributeList(list, 2, 0, &raw mut size) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let inherit: [HANDLE; 2] = [stdin_read.as_raw_handle(), stdout_write.as_raw_handle()];
    let jobs: [HANDLE; 1] = [job_handle(job)];
    // SAFETY: lista inizializzata; i valori puntati (`inherit`, `jobs`) vivono fino a dopo CreateProcessW.
    let updated = unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            inherit.as_ptr().cast::<c_void>(),
            size_of_val(&inherit),
            std::ptr::null_mut(),
            std::ptr::null(),
        ) != 0
            && UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                jobs.as_ptr().cast::<c_void>(),
                size_of_val(&jobs),
                std::ptr::null_mut(),
                std::ptr::null(),
            ) != 0
    };
    if !updated {
        let err = io::Error::last_os_error();
        // SAFETY: lista inizializzata sopra.
        unsafe { DeleteProcThreadAttributeList(list) };
        return Err(err);
    }

    // SAFETY: struttura C di interi e puntatori, lo zero è valido.
    let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    si.StartupInfo.cb = struct_len::<STARTUPINFOEXW>();
    si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    si.StartupInfo.hStdInput = stdin_read.as_raw_handle();
    si.StartupInfo.hStdOutput = stdout_write.as_raw_handle();
    si.StartupInfo.hStdError = std::ptr::null_mut();
    si.lpAttributeList = list;

    let app = wide(exe.as_os_str());
    let mut cmdline: Vec<u16> = Vec::new();
    cmdline.push(u16::from(b'"'));
    cmdline.extend(exe.as_os_str().encode_wide());
    cmdline.push(u16::from(b'"'));
    cmdline.extend(OsStr::new(&format!(" {arg}")).encode_wide());
    cmdline.push(0);
    let mut env: Vec<u16> = Vec::new();
    if let Some(root) = std::env::var_os("SystemRoot") {
        env.extend(OsStr::new("SystemRoot=").encode_wide());
        env.extend(root.encode_wide());
        env.push(0);
    }
    env.extend([0, 0]);
    let cwd = wide(exe.parent().map_or(OsStr::new("."), Path::as_os_str));

    // SAFETY: struttura di uscita valida, lo zero è valido.
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: stringhe terminate da zero e vive fino al ritorno; `cmdline` è modificabile come
    // richiesto; `si` è uno STARTUPINFOEXW con `cb` corretto e lista di attributi valida.
    let ok = unsafe {
        CreateProcessW(
            app.as_ptr(),
            cmdline.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | DETACHED_PROCESS,
            env.as_ptr().cast::<c_void>(),
            cwd.as_ptr(),
            (&raw const si).cast::<STARTUPINFOW>(),
            &raw mut pi,
        )
    };
    let err = io::Error::last_os_error();
    // SAFETY: lista inizializzata, non più usata dopo CreateProcessW.
    unsafe { DeleteProcThreadAttributeList(list) };
    if ok == 0 {
        return Err(err);
    }
    // SAFETY: handle restituiti da CreateProcessW, nostri; il thread non serve.
    unsafe { CloseHandle(pi.hThread) };
    drop((stdin_read, stdout_write));
    Ok(RawChild {
        // SAFETY: handle del processo nostro, chiuso una sola volta da `OwnedHandle`.
        process: unsafe { OwnedHandle::from_raw_handle(pi.hProcess) },
        pid: pi.dwProcessId,
        stdin: File::from(stdin_write),
        stdout: File::from(stdout_read),
    })
}

/// Vero se il processo appartiene al job.
pub fn process_in_job(process: &OwnedHandle, job: &Job) -> io::Result<bool> {
    let mut result = 0;
    // SAFETY: handle validi presi in prestito, puntatore di uscita valido.
    if unsafe { IsProcessInJob(process.as_raw_handle(), job_handle(job), &raw mut result) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(result != 0)
}

/// Aspetta la fine del processo e ne restituisce il codice d'uscita.
pub fn wait_exit(process: &OwnedHandle) -> io::Result<u32> {
    // SAFETY: handle valido preso in prestito.
    unsafe { WaitForSingleObject(process.as_raw_handle(), INFINITE) };
    let mut code = 0u32;
    // SAFETY: come sopra, puntatore di uscita valido.
    if unsafe { GetExitCodeProcess(process.as_raw_handle(), &raw mut code) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(code)
}

/// Vero se il processo `pid` esiste ancora e non è terminato.
pub fn process_alive(pid: u32) -> bool {
    // SAFETY: OpenProcess restituisce null se il processo non esiste più.
    let h = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    if h.is_null() {
        // SAFETY: lettura dell'ultimo errore del thread.
        let _ = unsafe { GetLastError() };
        return false;
    }
    // SAFETY: handle appena aperto, chiuso subito dopo.
    let exited = unsafe { WaitForSingleObject(h, 0) } == WAIT_OBJECT_0;
    // SAFETY: handle nostro.
    unsafe { CloseHandle(h) };
    !exited
}
