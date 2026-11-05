// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Le sole chiamate Win32 dirette dell'app, con un'API safe. Unico modulo con `unsafe` nel
//! workspace (il crate ha `#![deny(unsafe_code)]`; qui il permesso è mirato).
//!
//! - [`restrict_dll_search`]: `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)`, prima
//!   istruzione di `main`: nessuna DLL caricata da lì in poi dalla cartella dell'eseguibile,
//!   dalla cartella corrente o dal `PATH` (per le DLL importate, `/DEPENDENTLOADFLAG`).
//! - [`Job`]: Job Object con `KILL_ON_JOB_CLOSE`, limite di memoria impegnata per processo
//!   (`JOB_OBJECT_LIMIT_PROCESS_MEMORY`, che `win32job` non offre), un solo processo attivo,
//!   restrizioni dell'interfaccia e porta di completamento per sapere se un processo ha
//!   superato il limite (il codice d'uscita non lo distingue da un aborto qualsiasi:
//!   experiments/jobobject/ESITI.md).
//! - [`spawn_in_job`]: `CreateProcessW` con `PROC_THREAD_ATTRIBUTE_JOB_LIST` (il processo
//!   nasce già nel job) e `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (eredita solo stdin e stdout),
//!   percorso assoluto, ambiente con la sola `SystemRoot`, nessuna console, cartella corrente =
//!   cartella dell'eseguibile.
//!
//! - Identità e descrittori: SID dell'utente dal token ([`current_user`], [`process_user`]),
//!   cartelle e file privati con descrittore esplicito e protetto (solo utente e SYSTEM:
//!   [`create_private_dir`], [`create_new_private_file`], [`WindowsPrivateFs`]), lettura di
//!   proprietario e DACL ([`read_security`], [`exe_writable_by_others`]).
//! - Istanza unica: named pipe con `FILE_FLAG_FIRST_PIPE_INSTANCE`, solo client locali, DACL
//!   con il solo utente e I/O sovrapposto con tempi limite ([`PipeServer`], [`PipeClient`],
//!   aperto con SQoS Identification); [`allow_set_foreground`].
//! - Shell e accessibilità: [`assoc_is_dangerous`] (`AssocIsDangerous`) e
//!   [`high_contrast_colors`] (`SPI_GETHIGHCONTRAST`, poi `GetSysColor`).
//!
//! Ogni blocco `unsafe` ha il suo `// SAFETY:`; gli handle sono posseduti da `OwnedHandle`
//! (chiusi una volta sola) e i puntatori passati vivono fino al ritorno della chiamata. Le
//! funzioni private con precondizioni che il compilatore non verifica sono `unsafe fn` con
//! la sezione `# Safety` (`owned`, `cancel_and_wait`, `overlapped_io`); la lista degli
//! attributi di `CreateProcessW` prende in prestito i suoi valori (`AttributeList<'a>`).
#![allow(
    unsafe_code,
    reason = "unico modulo FFI dell'app: API safe sopra Win32 (PLAN.md, Architettura)"
)]

use std::ffi::{OsStr, OsString, c_void};
use std::fs::File;
use std::io;
use std::marker::PhantomData;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_NO_DATA,
    ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, GENERIC_ALL, GENERIC_WRITE,
    HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, LocalFree, SetHandleInformation,
    WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT,
    GetSysColor, SYS_COLOR_INDEX,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    ConvertStringSidToSidW, GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    CreateWellKnownSid, DACL_SECURITY_INFORMATION, GetAce, GetAclInformation, GetLengthSid,
    GetSecurityDescriptorControl, GetTokenInformation, INHERIT_ONLY_ACE, IsValidSid,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES,
    SECURITY_MAX_SID_SIZE, TOKEN_QUERY, TOKEN_USER, TokenUser, WinBuiltinAdministratorsSid,
    WinCreatorOwnerRightsSid, WinCreatorOwnerSid, WinLocalSystemSid,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateDirectoryW, CreateFileW,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_FLAG_OVERLAPPED, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GetFileInformationByHandle, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, READ_CONTROL, ReadFile,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows_sys::Win32::System::IO::{
    CancelIoEx, CreateIoCompletionPort, GetOverlappedResult, GetOverlappedResultEx,
    GetQueuedCompletionStatus, OVERLAPPED,
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOB_OBJECT_UILIMIT_DESKTOP,
    JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    JOBOBJECT_ASSOCIATE_COMPLETION_PORT, JOBOBJECT_BASIC_UI_RESTRICTIONS,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectAssociateCompletionPortInformation,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeServerProcessId,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT, WaitNamedPipeW,
};
use windows_sys::Win32::System::SystemServices::JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT;
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateEventW, CreateProcessW, DETACHED_PROCESS,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
    GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList, OpenProcess, OpenProcessToken,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW,
    SetEvent, TerminateProcess, UpdateProcThreadAttribute, WaitForMultipleObjects,
    WaitForSingleObject,
};
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::Shell::AssocIsDangerous;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, SPI_GETHIGHCONTRAST, SystemParametersInfoW,
};
use windows_sys::core::PWSTR;

/// Codice d'uscita dato ai processi terminati da qui.
const KILLED_EXIT_CODE: u32 = 1;

/// Limita la ricerca delle DLL caricate da qui in poi alla cartella di sistema. Le DLL
/// importate dall'eseguibile sono già caricate: per loro vale `DependentLoadFlags`
/// (`/DEPENDENTLOADFLAG:0x800` in `.cargo/config.toml`).
pub fn restrict_dll_search() -> io::Result<()> {
    // SAFETY: nessun puntatore; la funzione cambia solo un'impostazione del processo.
    let ok = unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Caratteri massimi di un'estensione chiesta ad [`assoc_is_dangerous`]: oltre, nessun tipo
/// registrato la usa e si risponde "pericolosa" senza chiedere.
const MAX_ASSOC_EXTENSION_CHARS: usize = 64;

/// Vero se Windows considera pericoloso il tipo con estensione `extension` (senza punto):
/// `AssocIsDangerous`, che unisce l'elenco del sistema e i tipi registrati come sempre
/// pericolosi. Un'estensione vuota non è un tipo (falso); con NUL o troppo lunga conta come
/// pericolosa.
pub fn assoc_is_dangerous(extension: &str) -> bool {
    if extension.is_empty() {
        return false;
    }
    if extension.contains('\0') || extension.chars().count() > MAX_ASSOC_EXTENSION_CHARS {
        return true;
    }
    let dotted = wide(OsStr::new(&format!(".{extension}")));
    // SAFETY: stringa UTF-16 terminata da zero viva fino al ritorno; la funzione la legge
    // soltanto.
    unsafe { AssocIsDangerous(dotted.as_ptr()) != 0 }
}

/// I colori di sistema di un tema a contrasto elevato, come `COLORREF` (`0x00BBGGRR`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighContrastColors {
    /// `COLOR_WINDOW`: fondo delle finestre.
    pub window: u32,
    /// `COLOR_WINDOWTEXT`: testo delle finestre.
    pub window_text: u32,
    /// `COLOR_HIGHLIGHT`: fondo degli elementi selezionati.
    pub highlight: u32,
    /// `COLOR_HIGHLIGHTTEXT`: testo degli elementi selezionati.
    pub highlight_text: u32,
    /// `COLOR_BTNFACE`: fondo dei bottoni.
    pub button_face: u32,
}

/// I colori del tema a contrasto elevato, se è attivo (`SPI_GETHIGHCONTRAST`, poi
/// `GetSysColor`); `None` se non è attivo o se la domanda non riesce.
pub fn high_contrast_colors() -> Option<HighContrastColors> {
    if !high_contrast_active() {
        return None;
    }
    Some(HighContrastColors {
        window: system_color(COLOR_WINDOW),
        window_text: system_color(COLOR_WINDOWTEXT),
        highlight: system_color(COLOR_HIGHLIGHT),
        highlight_text: system_color(COLOR_HIGHLIGHTTEXT),
        button_face: system_color(COLOR_BTNFACE),
    })
}

/// Il colore di sistema `index` (`GetSysColor`), come `COLORREF`.
fn system_color(index: SYS_COLOR_INDEX) -> u32 {
    // SAFETY: nessun puntatore; per un indice valido (le costanti `COLOR_*` di Windows) la
    // funzione restituisce il colore, per uno non valido zero.
    unsafe { GetSysColor(index) }
}

/// Vero se è attivo un tema a contrasto elevato (`SPI_GETHIGHCONTRAST`, flag
/// `HCF_HIGHCONTRASTON`); falso se la domanda non riesce.
fn high_contrast_active() -> bool {
    // SAFETY: struttura C di interi e un puntatore, lo zero è un valore valido.
    let mut info: HIGHCONTRASTW = unsafe { std::mem::zeroed() };
    info.cbSize = struct_len::<HIGHCONTRASTW>();
    // SAFETY: `info` è una HIGHCONTRASTW con `cbSize` corretto, scrivibile per la sua
    // lunghezza e viva fino al ritorno; nessuna impostazione cambiata (`fWinIni` = 0).
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            struct_len::<HIGHCONTRASTW>(),
            (&raw mut info).cast::<c_void>(),
            0,
        )
    };
    ok != 0 && info.dwFlags & HCF_HIGHCONTRASTON != 0
}

fn struct_len<T>() -> u32 {
    u32::try_from(size_of::<T>()).unwrap_or(u32::MAX)
}

/// Un handle appena restituito da Win32, o l'ultimo errore se nullo o non valido.
///
/// # Safety
///
/// `handle` deve essere nullo, `INVALID_HANDLE_VALUE` o un handle aperto appena restituito
/// da una funzione Win32 e non posseduto da nessun altro: `OwnedHandle` lo chiuderà.
unsafe fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: handle valido appena creato, posseduto solo da qui (garanzia del chiamante):
    // `OwnedHandle` lo chiude una volta sola.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// Restrizioni dell'interfaccia dei processi estrattori: niente appunti, atomi globali,
/// handle di finestre di altri processi, impostazioni di sistema e dello schermo, desktop,
/// uscita da Windows. Un estrattore non ha finestre: nulla di questo gli serve.
const WORKER_UI_RESTRICTIONS: u32 = JOB_OBJECT_UILIMIT_DESKTOP
    | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
    | JOB_OBJECT_UILIMIT_EXITWINDOWS
    | JOB_OBJECT_UILIMIT_GLOBALATOMS
    | JOB_OBJECT_UILIMIT_HANDLES
    | JOB_OBJECT_UILIMIT_READCLIPBOARD
    | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
    | JOB_OBJECT_UILIMIT_WRITECLIPBOARD;

/// Un Job Object con un solo processo estrattore.
///
/// Chiuderlo (drop) termina il processo (`KILL_ON_JOB_CLOSE`); il sistema lo chiude anche se
/// l'app termina in modo anomalo, quindi nessun estrattore sopravvive all'app. Il processo
/// non può avviarne altri (`ActiveProcessLimit` = 1) né usare l'interfaccia del sistema
/// ([`WORKER_UI_RESTRICTIONS`]): un decoder compromesso resta senza programmi da lanciare,
/// appunti e finestre altrui.
#[derive(Debug)]
pub struct Job {
    handle: OwnedHandle,
    port: OwnedHandle,
    memory_limit_hit: bool,
}

impl Job {
    /// Un job con limite di `memory_limit` byte di memoria impegnata per processo, un solo
    /// processo attivo e le restrizioni dell'interfaccia.
    pub fn new(memory_limit: usize) -> io::Result<Job> {
        // SAFETY: attributi nulli (handle non ereditabile) e nessun nome; l'handle
        // restituito è posseduto solo da qui.
        let handle = unsafe { owned(CreateJobObjectW(std::ptr::null(), std::ptr::null())) }?;
        // SAFETY: struttura C di soli interi, lo zero è un valore valido.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY
            | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        info.BasicLimitInformation.ActiveProcessLimit = 1;
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
        let restrictions = JOBOBJECT_BASIC_UI_RESTRICTIONS {
            UIRestrictionsClass: WORKER_UI_RESTRICTIONS,
        };
        // SAFETY: come sopra, con la struttura delle restrizioni dell'interfaccia.
        let ok = unsafe {
            SetInformationJobObject(
                handle.as_raw_handle(),
                JobObjectBasicUIRestrictions,
                (&raw const restrictions).cast::<c_void>(),
                struct_len::<JOBOBJECT_BASIC_UI_RESTRICTIONS>(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: nuova porta non associata a file (INVALID_HANDLE_VALUE), un thread;
        // l'handle restituito è posseduto solo da qui.
        let port = unsafe {
            owned(CreateIoCompletionPort(
                INVALID_HANDLE_VALUE,
                std::ptr::null_mut(),
                0,
                1,
            ))
        }?;
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
    // SAFETY: due handle appena creati da `CreatePipe`, posseduti solo da qui.
    let (read, write) = unsafe { (owned(read), owned(write)) };
    Ok((read?, write?))
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
///
/// La lista tiene puntatori ai valori passati a [`AttributeList::set`]: il prestito `'a`
/// li tiene vivi finché la lista esiste (e la lista va usata prima del suo `Drop`).
struct AttributeList<'a> {
    storage: Vec<usize>,
    values: PhantomData<&'a [HANDLE]>,
}

impl<'a> AttributeList<'a> {
    /// Una lista inizializzata per `count` attributi.
    fn new(count: u32) -> io::Result<AttributeList<'a>> {
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
        Ok(AttributeList {
            storage,
            values: PhantomData,
        })
    }

    fn as_ptr(&mut self) -> *mut c_void {
        self.storage.as_mut_ptr().cast::<c_void>()
    }

    /// Imposta un attributo che punta a `values`, prestati finché vive la lista.
    fn set(&mut self, attribute: usize, values: &'a [HANDLE]) -> io::Result<()> {
        let list = self.as_ptr();
        // SAFETY: lista inizializzata; `values` è un array valido di `size_of_val` byte, vivo
        // per tutta la vita della lista (prestito `'a`).
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

impl Drop for AttributeList<'_> {
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
    // SAFETY: handle del thread e del processo appena creati da `CreateProcessW` riuscita,
    // posseduti solo da qui.
    let (thread, process) = unsafe { (owned(info.hThread), owned(info.hProcess)) };
    // Il thread principale non serve; il processo resta posseduto da `ChildProcess`.
    drop(thread);
    let process = ChildProcess { handle: process? };
    Ok(SpawnedChild {
        process,
        stdin: File::from(stdin_write),
        stdout: File::from(stdout_read),
    })
}

// --- Identità e descrittori di sicurezza ---------------------------------------------------

/// Un SID copiato in memoria propria (byte del formato binario di Windows).
#[derive(Clone, PartialEq, Eq)]
pub struct Sid(Vec<u8>);

impl std::fmt::Debug for Sid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.to_sddl() {
            Ok(text) => f.write_str(&text),
            Err(_) => f.write_str("<SID>"),
        }
    }
}

/// SID ben noti che le cartelle dell'app e dei programmi possono contenere senza aprirle ad
/// altri utenti.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KnownSid {
    LocalSystem,
    Administrators,
    CreatorOwner,
    OwnerRights,
}

/// SID del servizio TrustedInstaller, proprietario delle cartelle di Windows e dei programmi
/// installati (non è tra i SID ben noti di `CreateWellKnownSid`).
const TRUSTED_INSTALLER_SDDL: &str =
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

impl Sid {
    /// Copia il SID puntato da `psid`.
    ///
    /// # Safety
    ///
    /// `psid` deve essere nullo o puntare a memoria leggibile che contiene un SID per tutta
    /// la sua lunghezza.
    unsafe fn copy_from(psid: PSID) -> io::Result<Sid> {
        if psid.is_null() {
            return Err(io::Error::other("SID nullo"));
        }
        // SAFETY: il chiamante garantisce un SID leggibile; `IsValidSid` controlla la
        // struttura prima di leggerne la lunghezza.
        if unsafe { IsValidSid(psid) } == 0 {
            return Err(io::Error::other("SID non valido"));
        }
        // SAFETY: SID valido (controllato sopra).
        let len = unsafe { GetLengthSid(psid) } as usize;
        // SAFETY: `len` byte leggibili a partire da `psid` (lunghezza dichiarata dal SID
        // valido); la copia non tiene il puntatore.
        let bytes = unsafe { std::slice::from_raw_parts(psid.cast::<u8>(), len) }.to_vec();
        Ok(Sid(bytes))
    }

    /// Il puntatore per le API che leggono un SID (che non lo modificano).
    fn as_psid(&self) -> PSID {
        self.0.as_ptr().cast_mut().cast::<c_void>()
    }

    /// Un SID ben noto.
    fn well_known(kind: KnownSid) -> io::Result<Sid> {
        let kind = match kind {
            KnownSid::LocalSystem => WinLocalSystemSid,
            KnownSid::Administrators => WinBuiltinAdministratorsSid,
            KnownSid::CreatorOwner => WinCreatorOwnerSid,
            KnownSid::OwnerRights => WinCreatorOwnerRightsSid,
        };
        let mut buffer = vec![0u8; SECURITY_MAX_SID_SIZE as usize];
        let mut len = SECURITY_MAX_SID_SIZE;
        // SAFETY: buffer scrivibile di `len` byte; nessun SID di dominio.
        let ok = unsafe {
            CreateWellKnownSid(
                kind,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast::<c_void>(),
                &raw mut len,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        buffer.truncate(len as usize);
        Ok(Sid(buffer))
    }

    /// Il SID dalla sua forma testuale ("S-1-5-…").
    fn from_sddl(text: &str) -> io::Result<Sid> {
        let wide_text = wide(OsStr::new(text));
        let mut psid: PSID = std::ptr::null_mut();
        // SAFETY: stringa terminata da zero; il SID restituito va liberato con `LocalFree`.
        let ok = unsafe { ConvertStringSidToSidW(wide_text.as_ptr(), &raw mut psid) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: SID appena allocato dal sistema, valido fino al `LocalFree`.
        let sid = unsafe { Sid::copy_from(psid) };
        // SAFETY: memoria allocata da `ConvertStringSidToSidW`, liberata una volta.
        unsafe { LocalFree(psid) };
        sid
    }

    /// La forma testuale ("S-1-5-21-…").
    pub fn to_sddl(&self) -> io::Result<String> {
        let mut text: PWSTR = std::ptr::null_mut();
        // SAFETY: SID valido posseduto da `self`; la stringa restituita va liberata con
        // `LocalFree`.
        let ok = unsafe { ConvertSidToStringSidW(self.as_psid(), &raw mut text) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: stringa terminata da zero allocata dal sistema.
        let result = unsafe { wide_ptr_to_string(text) };
        // SAFETY: memoria allocata da `ConvertSidToStringSidW`, liberata una volta.
        unsafe { LocalFree(text.cast::<c_void>()) };
        Ok(result)
    }
}

/// Il testo di una stringa UTF-16 terminata da zero.
///
/// # Safety
///
/// `text` deve puntare a una stringa UTF-16 terminata da zero.
unsafe fn wide_ptr_to_string(text: *const u16) -> String {
    let mut len = 0usize;
    // SAFETY: stringa terminata da zero (garanzia del chiamante): si legge fino allo zero.
    while unsafe { *text.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` caratteri leggibili prima dello zero.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) })
}

/// L'utente del token del processo `process` (handle con `PROCESS_QUERY_LIMITED_INFORMATION`).
fn token_user(process: HANDLE) -> io::Result<Sid> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: handle del processo valido preso in prestito; handle d'uscita valido.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: handle del token appena aperto, posseduto solo da qui.
    let token = unsafe { owned(token) }?;
    let mut needed = 0u32;
    // SAFETY: prima chiamata senza buffer per sapere la dimensione; scrive solo `needed`.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            std::ptr::null_mut(),
            0,
            &raw mut needed,
        )
    };
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    // `u64` per l'allineamento della struttura TOKEN_USER (puntatori).
    let mut buffer = vec![0u64; (needed as usize).div_ceil(size_of::<u64>())];
    // SAFETY: buffer allineato e scrivibile di almeno `needed` byte.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast::<c_void>(),
            needed,
            &raw mut needed,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: il sistema ha scritto una TOKEN_USER all'inizio del buffer allineato.
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    // SAFETY: il SID punta dentro `buffer`, vivo per tutta la copia.
    unsafe { Sid::copy_from(user.User.Sid) }
}

/// Il SID dell'utente di questo processo (dal token, `TokenUser`).
pub fn current_user() -> io::Result<Sid> {
    // SAFETY: pseudo-handle del processo corrente, non va chiuso.
    token_user(unsafe { GetCurrentProcess() })
}

/// Il SID dell'utente del processo `pid`.
pub fn process_user(pid: u32) -> io::Result<Sid> {
    // SAFETY: nessun puntatore; l'handle restituito è posseduto solo da qui.
    let process = unsafe { owned(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid)) }?;
    token_user(process.as_raw_handle())
}

/// Un descrittore di sicurezza costruito da SDDL, liberato nel `Drop`.
struct SecurityDescriptor {
    pointer: PSECURITY_DESCRIPTOR,
}

impl SecurityDescriptor {
    fn from_sddl(sddl: &str) -> io::Result<SecurityDescriptor> {
        let text = wide(OsStr::new(sddl));
        let mut pointer: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: stringa terminata da zero; il descrittore restituito va liberato con
        // `LocalFree` (nel `Drop`).
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                SDDL_REVISION_1,
                &raw mut pointer,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || pointer.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(SecurityDescriptor { pointer })
    }

    /// Gli attributi da passare alle funzioni `Create*` (handle non ereditabile); validi
    /// finché vive `self`.
    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: struct_len::<SECURITY_ATTRIBUTES>(),
            lpSecurityDescriptor: self.pointer,
            bInheritHandle: 0,
        }
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: descrittore allocato da `ConvertStringSecurityDescriptorToSecurityDescriptorW`,
        // liberato una volta sola.
        unsafe { LocalFree(self.pointer) };
    }
}

/// Che cosa protegge un descrittore privato.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivateObject {
    /// Cartella: ACE ereditate da sottocartelle e file.
    Directory,
    /// File.
    File,
    /// Named pipe: solo l'utente.
    Pipe,
}

/// Il descrittore privato in SDDL: proprietario l'utente, DACL protetta (nessuna
/// ereditarietà dal padre) con accesso completo per l'utente e SYSTEM (solo l'utente per
/// la pipe). Mai la DACL di default, che contiene il SID effimero della logon session.
fn private_sddl(user: &Sid, object: PrivateObject) -> io::Result<String> {
    let user = user.to_sddl()?;
    Ok(match object {
        PrivateObject::Directory => format!("O:{user}D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)"),
        PrivateObject::File => format!("O:{user}D:P(A;;FA;;;{user})(A;;FA;;;SY)"),
        PrivateObject::Pipe => format!("O:{user}D:P(A;;GA;;;{user})"),
    })
}

fn private_descriptor(object: PrivateObject) -> io::Result<SecurityDescriptor> {
    SecurityDescriptor::from_sddl(&private_sddl(&current_user()?, object)?)
}

/// Una voce di una DACL, copiata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ace {
    /// Concede (vero) o nega.
    pub allows: bool,
    /// Vale solo per gli oggetti creati dentro (`INHERIT_ONLY_ACE`), non per questo.
    pub inherit_only: bool,
    /// Flag d'ereditarietà (`OBJECT_INHERIT_ACE`, `CONTAINER_INHERIT_ACE`, …).
    pub flags: u8,
    /// Diritti.
    pub mask: u32,
    /// A chi.
    pub sid: Sid,
}

/// Proprietario e DACL di un file o di una cartella.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityInfo {
    /// Il proprietario.
    pub owner: Sid,
    /// La DACL; `None` = DACL nulla (tutto a tutti).
    pub dacl: Option<Vec<Ace>>,
    /// DACL protetta: nessuna voce ereditata dal padre.
    pub protected: bool,
}

/// Tipi di ACE che concedono diritti (normali, con oggetto, con callback).
const ALLOWED_ACE_TYPES: [u8; 4] = [0x0, 0x5, 0x9, 0xB];
/// Tipi di ACE che negano diritti.
const DENIED_ACE_TYPES: [u8; 4] = [0x1, 0x6, 0xA, 0xC];

/// Copia le voci di `acl`.
///
/// # Safety
///
/// `acl` deve puntare a una ACL valida, leggibile per tutta la sua dimensione.
unsafe fn read_acl(acl: *const ACL) -> io::Result<Vec<Ace>> {
    // SAFETY: struttura C di soli interi, lo zero è un valore valido.
    let mut size: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: ACL valida (garanzia del chiamante); buffer d'uscita della lunghezza dichiarata.
    let ok = unsafe {
        GetAclInformation(
            acl,
            (&raw mut size).cast::<c_void>(),
            struct_len::<ACL_SIZE_INFORMATION>(),
            AclSizeInformation,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut aces = Vec::with_capacity(size.AceCount as usize);
    for index in 0..size.AceCount {
        let mut pointer: *mut c_void = std::ptr::null_mut();
        // SAFETY: ACL valida, indice entro il numero di voci.
        if unsafe { GetAce(acl, index, &raw mut pointer) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: ogni ACE inizia con un ACE_HEADER.
        let header = unsafe { &*pointer.cast::<ACE_HEADER>() };
        let allows = ALLOWED_ACE_TYPES.contains(&header.AceType);
        let denies = DENIED_ACE_TYPES.contains(&header.AceType);
        if !allows && !denies {
            // Audit, etichette d'integrità e simili: non concedono né negano accesso.
            continue;
        }
        let sid = if matches!(header.AceType, 0x0 | 0x1 | 0x9 | 0xA) {
            // SAFETY: ACE di accesso semplice: dopo l'intestazione la maschera e il SID.
            let ace = unsafe { &*pointer.cast::<ACCESS_ALLOWED_ACE>() };
            // SAFETY: il SID inizia a `SidStart`, dentro la voce valida.
            unsafe { Sid::copy_from((&raw const ace.SidStart).cast_mut().cast::<c_void>()) }?
        } else {
            // Voci con oggetto: il SID è dopo campi di lunghezza variabile. Non servono
            // alle cartelle dei file; per prudenza contano come voce di uno sconosciuto.
            Sid::from_sddl("S-1-0-0")?
        };
        // SAFETY: come sopra, la maschera segue l'intestazione in ogni tipo di ACE.
        let mask = unsafe { &*pointer.cast::<ACCESS_ALLOWED_ACE>() }.Mask;
        aces.push(Ace {
            allows,
            inherit_only: u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0,
            flags: header.AceFlags,
            mask,
            sid,
        });
    }
    Ok(aces)
}

/// Apre `path` (file o cartella, senza seguire collegamenti) per leggerne la sicurezza.
fn open_for_security(path: &Path) -> io::Result<OwnedHandle> {
    let name = wide(path.as_os_str());
    // SAFETY: stringa terminata da zero; attributi nulli; l'handle restituito è posseduto
    // solo da qui.
    unsafe {
        owned(CreateFileW(
            name.as_ptr(),
            READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        ))
    }
}

/// Attributi del file aperto.
fn file_attributes(handle: &OwnedHandle) -> io::Result<u32> {
    // SAFETY: struttura C di soli interi, lo zero è un valore valido.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: handle valido; struttura d'uscita valida.
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &raw mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info.dwFileAttributes)
}

/// Proprietario e DACL dell'oggetto aperto.
fn security_of(handle: &OwnedHandle) -> io::Result<SecurityInfo> {
    let mut owner: PSID = std::ptr::null_mut();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: handle valido aperto con READ_CONTROL; puntatori d'uscita validi; il
    // descrittore restituito (in cui puntano `owner` e `dacl`) va liberato con `LocalFree`.
    let status = unsafe {
        GetSecurityInfo(
            handle.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &raw mut owner,
            std::ptr::null_mut(),
            &raw mut dacl,
            std::ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(
            i32::try_from(status).unwrap_or(i32::MAX),
        ));
    }
    let guard = SecurityDescriptor {
        pointer: descriptor,
    };
    let mut control = 0u16;
    let mut revision = 0u32;
    // SAFETY: descrittore valido posseduto da `guard`.
    if unsafe { GetSecurityDescriptorControl(guard.pointer, &raw mut control, &raw mut revision) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `owner` punta dentro il descrittore vivo.
    let owner = unsafe { Sid::copy_from(owner) }?;
    let dacl = if dacl.is_null() {
        None
    } else {
        // SAFETY: `dacl` punta a una ACL dentro il descrittore vivo.
        Some(unsafe { read_acl(dacl) }?)
    };
    Ok(SecurityInfo {
        owner,
        dacl,
        protected: control & SE_DACL_PROTECTED != 0,
    })
}

/// Proprietario e DACL di `path` (file o cartella; un collegamento si legge come tale).
pub fn read_security(path: &Path) -> io::Result<SecurityInfo> {
    security_of(&open_for_security(path)?)
}

/// I SID che possono avere accesso a una cartella privata dell'utente: l'utente, SYSTEM, gli
/// amministratori (che hanno comunque il privilegio di prenderne possesso) e i segnaposto
/// del proprietario.
fn trusted_sids(user: &Sid) -> io::Result<Vec<Sid>> {
    Ok(vec![
        user.clone(),
        Sid::well_known(KnownSid::LocalSystem)?,
        Sid::well_known(KnownSid::Administrators)?,
        Sid::well_known(KnownSid::CreatorOwner)?,
        Sid::well_known(KnownSid::OwnerRights)?,
    ])
}

/// Vero se `info` lascia accedere solo a `trusted`: proprietario fidato, DACL presente e ogni
/// voce che concede qualcosa (anche solo agli oggetti creati dentro) per un SID fidato.
fn is_private(info: &SecurityInfo, trusted: &[Sid]) -> bool {
    let Some(dacl) = &info.dacl else {
        return false;
    };
    trusted.contains(&info.owner)
        && dacl
            .iter()
            .all(|ace| !ace.allows || ace.mask == 0 || trusted.contains(&ace.sid))
}

/// Diritti che permettono di cambiare il contenuto di una cartella o i suoi permessi.
const WRITE_RIGHTS: u32 = 0x2 // FILE_ADD_FILE
    | 0x4 // FILE_ADD_SUBDIRECTORY
    | 0x40 // FILE_DELETE_CHILD
    | 0x1_0000 // DELETE
    | 0x4_0000 // WRITE_DAC
    | 0x8_0000 // WRITE_OWNER
    | GENERIC_WRITE
    | GENERIC_ALL;

/// Vero se `info` lascia modificare l'oggetto stesso (file o cartella) a chi non è in
/// `trusted`: proprietario fuori da `trusted` (il proprietario può sempre cambiare la DACL),
/// DACL nulla, o una voce non solo ereditabile che concede scrittura a un SID fuori da
/// `trusted`.
fn writable_by_others(info: &SecurityInfo, trusted: &[Sid]) -> bool {
    let Some(dacl) = &info.dacl else {
        return true;
    };
    !trusted.contains(&info.owner)
        || dacl.iter().any(|ace| {
            ace.allows
                && !ace.inherit_only
                && ace.mask & WRITE_RIGHTS != 0
                && !trusted.contains(&ace.sid)
        })
}

/// Chi può modificare il programma senza che sia un problema: l'utente, SYSTEM, gli
/// amministratori e TrustedInstaller. Non i segnaposto CREATOR OWNER e OWNER RIGHTS, che
/// valgono per chi crea o possiede un oggetto (anche un altro utente).
fn program_trusted_sids(user: &Sid) -> io::Result<Vec<Sid>> {
    Ok(vec![
        user.clone(),
        Sid::well_known(KnownSid::LocalSystem)?,
        Sid::well_known(KnownSid::Administrators)?,
        Sid::from_sddl(TRUSTED_INSTALLER_SDDL)?,
    ])
}

/// Vero se l'eseguibile `exe` o la sua cartella sono modificabili da utenti diversi da
/// questo, dagli amministratori, da SYSTEM e da TrustedInstaller (prima di registrare il
/// clic destro): proprietario estraneo o scrittura concessa ad altri.
pub fn exe_writable_by_others(exe: &Path) -> io::Result<bool> {
    let dir = exe
        .parent()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let trusted = program_trusted_sids(&current_user()?)?;
    for object in [exe, dir] {
        if writable_by_others(&read_security(object)?, &trusted) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Errore di "cartella non privata" (lo stesso tipo dell'engine).
fn not_private() -> io::Error {
    lynkeus_engine::NotPrivate::error()
}

/// Controlla una cartella esistente: cartella vera (non un collegamento), proprietario e
/// DACL che non lasciano accedere altri utenti.
fn check_private_dir(dir: &Path) -> io::Result<()> {
    let handle = open_for_security(dir)?;
    let attributes = file_attributes(&handle)?;
    if attributes & FILE_ATTRIBUTE_DIRECTORY == 0 || attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(not_private());
    }
    let info = security_of(&handle)?;
    if is_private(&info, &trusted_sids(&current_user()?)?) {
        Ok(())
    } else {
        Err(not_private())
    }
}

/// Crea `dir` e le cartelle mancanti sopra con il descrittore privato; una cartella `dir`
/// già esistente si controlla ([`check_private_dir`]) e non si modifica.
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    if !dir.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cartella privata senza percorso assoluto",
        ));
    }
    let descriptor = private_descriptor(PrivateObject::Directory)?;
    let attributes = descriptor.attributes();
    // Le cartelle da creare, dalla più esterna; quelle che esistono già non si toccano.
    let missing: Vec<&Path> = dir
        .ancestors()
        .take_while(|path| std::fs::symlink_metadata(path).is_err())
        .collect();
    for path in missing.iter().rev() {
        let name = wide(path.as_os_str());
        // SAFETY: stringa terminata da zero; attributi con un descrittore vivo fino al
        // ritorno.
        if unsafe { CreateDirectoryW(name.as_ptr(), &raw const attributes) } == 0 {
            let error = io::Error::last_os_error();
            // Creata nel frattempo da un altro processo: per `dir` si controlla sotto.
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
    }
    // Anche se creata qui adesso si controlla: una cartella creata da altri nello stesso
    // istante (gara sul nome) non passa.
    check_private_dir(dir)
}

/// Crea un file nuovo con il descrittore privato, aperto in scrittura; errore
/// `AlreadyExists` se il nome esiste (anche come collegamento, che non si segue).
pub fn create_new_private_file(path: &Path) -> io::Result<File> {
    let descriptor = private_descriptor(PrivateObject::File)?;
    let attributes = descriptor.attributes();
    let name = wide(path.as_os_str());
    // SAFETY: stringa terminata da zero; attributi con un descrittore vivo fino al ritorno;
    // l'handle restituito è posseduto solo da qui.
    let handle = unsafe {
        owned(CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &raw const attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        ))
    }?;
    Ok(File::from(handle))
}

/// Cartelle e file privati con il descrittore esplicito, iniettati nell'engine.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsPrivateFs;

impl lynkeus_engine::PrivateFs for WindowsPrivateFs {
    fn create_dir(&self, dir: &Path) -> io::Result<()> {
        create_private_dir(dir)
    }

    fn create_new_file(&self, path: &Path) -> io::Result<File> {
        create_new_private_file(path)
    }
}

// --- Primo piano --------------------------------------------------------------------------

/// Permette al processo `pid` (la prima istanza) di portare in primo piano la sua finestra.
pub fn allow_set_foreground(pid: u32) -> io::Result<()> {
    // SAFETY: nessun puntatore.
    if unsafe { AllowSetForegroundWindow(pid) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

// --- Named pipe dell'istanza unica ---------------------------------------------------------

/// Flag con cui il client apre la pipe: senza `SECURITY_SQOS_PRESENT` il livello
/// d'impersonazione sarebbe ignorato e un server finto otterrebbe `Impersonation`.
pub const PIPE_CLIENT_SQOS: u32 = SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION;

/// Un evento manuale con `Drop` (per l'I/O sovrapposto e per lo stop).
#[derive(Debug)]
struct Event {
    handle: OwnedHandle,
}

impl Event {
    fn new() -> io::Result<Event> {
        // SAFETY: attributi nulli, nessun nome; evento manuale non segnalato; l'handle
        // restituito è posseduto solo da qui.
        let handle = unsafe { owned(CreateEventW(std::ptr::null(), 1, 0, std::ptr::null())) }?;
        Ok(Event { handle })
    }
}

/// Un segnale di stop per il server della pipe, da qualunque thread.
#[derive(Debug)]
pub struct StopSignal {
    event: Event,
}

impl StopSignal {
    /// Un segnale non ancora dato.
    pub fn new() -> io::Result<StopSignal> {
        Ok(StopSignal {
            event: Event::new()?,
        })
    }

    /// Dà il segnale: l'attesa di una connessione finisce.
    pub fn set(&self) {
        // SAFETY: handle dell'evento valido preso in prestito.
        unsafe { SetEvent(self.event.handle.as_raw_handle()) };
    }

    /// Vero se il segnale è stato dato.
    fn is_set(&self) -> bool {
        // SAFETY: handle dell'evento valido preso in prestito; attesa nulla.
        unsafe { WaitForSingleObject(self.event.handle.as_raw_handle(), 0) == WAIT_OBJECT_0 }
    }
}

/// Esito di un tentativo di connessione del server.
enum Connection {
    /// Un client è connesso.
    Connected,
    /// Lo stop è arrivato.
    Stopped,
    /// Un client si era connesso e ha già chiuso: la connessione va chiusa.
    ClientGone,
}

/// Vero se `error` è l'errore di sistema `code`.
fn is_os_error(error: &io::Error, code: u32) -> bool {
    error.raw_os_error() == i32::try_from(code).ok()
}

/// Annulla l'operazione sovrapposta di `overlapped` (se è ancora in corso) e ne attende la
/// fine; restituisce l'esito finale (byte trasferiti o errore). Dopo il ritorno il sistema non
/// tocca più `overlapped` né il buffer dell'operazione: solo allora si possono liberare.
///
/// Regola del modulo: ogni operazione sovrapposta avviata (esito `ERROR_IO_PENDING` o
/// completata) che non si è vista finire con un'attesa riuscita passa da qui prima che
/// struttura, evento o buffer escano di scope, qualunque sia l'errore visto.
///
/// # Safety
///
/// `overlapped` punta alla struttura (viva, mai spostata) di un'operazione avviata su
/// `handle`, anche già finita, e resta valido fino al ritorno. Il puntatore è grezzo di
/// proposito: il sistema può scrivere nella struttura fino alla fine dell'operazione, quindi
/// nessun riferimento Rust deve esistere su di essa in quel tempo.
unsafe fn cancel_and_wait(
    handle: BorrowedHandle<'_>,
    overlapped: *mut OVERLAPPED,
) -> io::Result<u32> {
    let mut transferred = 0u32;
    // SAFETY: `handle` è l'handle su cui l'operazione è stata avviata e `overlapped` la sua
    // struttura, viva fino al ritorno (garanzia del chiamante). `CancelIoEx` su
    // un'operazione già finita fallisce con `ERROR_NOT_FOUND` senza effetti;
    // `GetOverlappedResult` con attesa (`bWait` = 1) ritorna solo a operazione finita
    // (completata o annullata).
    let done = unsafe {
        CancelIoEx(handle.as_raw_handle(), overlapped);
        GetOverlappedResult(handle.as_raw_handle(), overlapped, &raw mut transferred, 1)
    };
    if done == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(transferred)
    }
}

/// Vero se `error` dice che l'attesa di un'operazione sovrapposta è scaduta (`WAIT_TIMEOUT`,
/// o `ERROR_IO_INCOMPLETE` quando il tempo rimasto era zero).
fn wait_expired(error: &io::Error) -> bool {
    [WAIT_TIMEOUT, ERROR_IO_INCOMPLETE]
        .iter()
        .any(|code| error.raw_os_error() == i32::try_from(*code).ok())
}

/// Esegue un'operazione sovrapposta su `handle` e ne attende la fine al massimo `timeout`;
/// se non la si vede finire (tempo scaduto, anche con `timeout` zero, o qualunque errore
/// dell'attesa) la annulla e ne aspetta la chiusura ([`cancel_and_wait`]): la struttura
/// OVERLAPPED, l'evento e il buffer non spariscono mai con l'operazione in corso.
/// Restituisce i byte trasferiti; `TimedOut` se il tempo è scaduto senza trasferimento.
///
/// `start` riceve l'handle e la struttura, avvia l'operazione e restituisce l'esito della
/// funzione Win32 (zero = errore, con `ERROR_IO_PENDING` per un'operazione in corso).
///
/// # Safety
///
/// `start` avvia al più un'operazione sovrapposta, sull'handle e con la struttura che
/// riceve; i buffer che le passa restano validi (e non spostati) fino al ritorno di questa
/// funzione.
unsafe fn overlapped_io(
    handle: BorrowedHandle<'_>,
    timeout: Duration,
    start: impl FnOnce(HANDLE, *mut OVERLAPPED) -> i32,
) -> io::Result<u32> {
    let event = Event::new()?;
    // SAFETY: struttura C di interi e puntatori, lo zero è un valore valido.
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    overlapped.hEvent = event.handle.as_raw_handle();
    // Da qui in poi la struttura si tocca solo tramite questo puntatore.
    let pointer = &raw mut overlapped;
    if start(handle.as_raw_handle(), pointer) == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != i32::try_from(ERROR_IO_PENDING).ok() {
            // L'operazione non è partita: nulla da attendere.
            return Err(error);
        }
    }
    let millis = u32::try_from(timeout.as_millis())
        .unwrap_or(INFINITE - 1)
        .min(INFINITE - 1);
    let mut transferred = 0u32;
    // SAFETY: handle valido; la struttura vive fino alla fine dell'operazione (sotto, in ogni
    // caso diverso dal completamento visto qui, `cancel_and_wait`).
    let done = unsafe {
        GetOverlappedResultEx(
            handle.as_raw_handle(),
            pointer,
            &raw mut transferred,
            millis,
            0,
        )
    };
    if done != 0 {
        return Ok(transferred);
    }
    let error = io::Error::last_os_error();
    let expired = wait_expired(&error);
    // SAFETY: `pointer` è la struttura dell'operazione avviata da `start` su `handle`, viva
    // fino alla fine di questa funzione.
    let finished = unsafe { cancel_and_wait(handle, pointer) };
    match finished {
        // Finita proprio mentre la si annullava: il trasferimento è avvenuto e conta.
        Ok(transferred) if transferred > 0 => Ok(transferred),
        _ if expired => Err(io::Error::from(io::ErrorKind::TimedOut)),
        Ok(transferred) => Ok(transferred),
        Err(final_error) => Err(final_error),
    }
}

/// Legge esattamente `buffer.len()` byte da una pipe sovrapposta entro `deadline`.
fn pipe_read_exact(
    handle: BorrowedHandle<'_>,
    buffer: &mut [u8],
    deadline: Instant,
) -> io::Result<()> {
    let mut filled = 0usize;
    while filled < buffer.len() {
        let rest = buffer.get_mut(filled..).unwrap_or_default();
        let len = u32::try_from(rest.len()).unwrap_or(u32::MAX);
        let remaining = deadline.saturating_duration_since(Instant::now());
        let pointer = rest.as_mut_ptr();
        let start = |raw: HANDLE, overlapped: *mut OVERLAPPED| {
            // SAFETY: `rest` scrivibile per `len` byte; handle e struttura validi.
            unsafe { ReadFile(raw, pointer, len, std::ptr::null_mut(), overlapped) }
        };
        // SAFETY: `start` avvia una sola lettura, sull'handle e con la struttura ricevuti,
        // in `rest`, che resta in prestito fino al ritorno.
        let read = unsafe { overlapped_io(handle, remaining, start) }.map_err(pipe_error)?;
        if read == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        filled += read as usize;
    }
    Ok(())
}

/// Scrive tutto `bytes` su una pipe sovrapposta entro `deadline`.
fn pipe_write_all(handle: BorrowedHandle<'_>, bytes: &[u8], deadline: Instant) -> io::Result<()> {
    let mut sent = 0usize;
    while sent < bytes.len() {
        let rest = bytes.get(sent..).unwrap_or_default();
        let len = u32::try_from(rest.len()).unwrap_or(u32::MAX);
        let remaining = deadline.saturating_duration_since(Instant::now());
        let pointer = rest.as_ptr();
        let start = |raw: HANDLE, overlapped: *mut OVERLAPPED| {
            // SAFETY: `rest` leggibile per `len` byte; handle e struttura validi.
            unsafe { WriteFile(raw, pointer, len, std::ptr::null_mut(), overlapped) }
        };
        // SAFETY: `start` avvia una sola scrittura, sull'handle e con la struttura ricevuti,
        // da `rest`, che resta in prestito fino al ritorno.
        let written = unsafe { overlapped_io(handle, remaining, start) }.map_err(pipe_error)?;
        if written == 0 {
            return Err(io::Error::from(io::ErrorKind::WriteZero));
        }
        sent += written as usize;
    }
    Ok(())
}

/// Una pipe chiusa dall'altro capo diventa una fine del flusso.
fn pipe_error(error: io::Error) -> io::Error {
    let closed = [ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_PIPE_NOT_CONNECTED]
        .iter()
        .any(|code| error.raw_os_error() == i32::try_from(*code).ok());
    if closed {
        io::Error::from(io::ErrorKind::UnexpectedEof)
    } else {
        error
    }
}

/// Il capo server della named pipe dell'istanza unica: una sola istanza, solo client
/// locali, DACL con il solo utente, I/O sovrapposto con tempi limite.
#[derive(Debug)]
pub struct PipeServer {
    handle: OwnedHandle,
}

impl PipeServer {
    /// Crea la pipe `name` (`\\.\pipe\…`). Errore se il nome esiste già
    /// (`FILE_FLAG_FIRST_PIPE_INSTANCE`): non si usa mai un oggetto creato da altri.
    pub fn create(name: &str) -> io::Result<PipeServer> {
        let descriptor = private_descriptor(PrivateObject::Pipe)?;
        let attributes = descriptor.attributes();
        let wide_name = wide(OsStr::new(name));
        // SAFETY: stringa terminata da zero; attributi con un descrittore vivo fino al
        // ritorno; l'handle restituito è posseduto solo da qui.
        let handle = unsafe {
            owned(CreateNamedPipeW(
                wide_name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                PIPE_BUFFER_BYTES,
                PIPE_BUFFER_BYTES,
                0,
                &raw const attributes,
            ))
        }?;
        Ok(PipeServer { handle })
    }

    /// Aspetta un client finché non arriva (vero) o finché `stop` non è dato (falso).
    ///
    /// Un client che si è connesso e ha già chiuso (`ERROR_NO_DATA`) non ferma l'attesa: la
    /// connessione morta si chiude e si aspetta il prossimo. Gli altri errori (anche
    /// dell'attesa, `WAIT_FAILED`) chiudono la connessione e tornano al chiamante, che decide
    /// se riprovare; la pipe resta pronta.
    pub fn accept(&self, stop: &StopSignal) -> io::Result<bool> {
        loop {
            if stop.is_set() {
                return Ok(false);
            }
            match self.connect_once(stop) {
                Ok(Connection::Connected) => return Ok(true),
                Ok(Connection::Stopped) => return Ok(false),
                Ok(Connection::ClientGone) => self.disconnect(),
                Err(error) => {
                    self.disconnect();
                    return Err(error);
                }
            }
        }
    }

    /// Un tentativo di `ConnectNamedPipe` sovrapposto, atteso insieme a `stop`.
    fn connect_once(&self, stop: &StopSignal) -> io::Result<Connection> {
        let handle = self.handle.as_handle();
        let event = Event::new()?;
        // SAFETY: struttura C di interi e puntatori, lo zero è un valore valido.
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        overlapped.hEvent = event.handle.as_raw_handle();
        // Da qui in poi la struttura si tocca solo tramite questo puntatore.
        let pointer = &raw mut overlapped;
        // SAFETY: handle della pipe valido; la struttura vive fino alla fine dell'operazione
        // (sotto: completamento atteso o `cancel_and_wait`).
        if unsafe { ConnectNamedPipe(handle.as_raw_handle(), pointer) } != 0 {
            return Ok(Connection::Connected);
        }
        let error = io::Error::last_os_error();
        if is_os_error(&error, ERROR_PIPE_CONNECTED) {
            return Ok(Connection::Connected);
        }
        if is_os_error(&error, ERROR_NO_DATA) {
            return Ok(Connection::ClientGone);
        }
        if !is_os_error(&error, ERROR_IO_PENDING) {
            // Nessuna operazione partita.
            return Err(error);
        }
        let waited: [HANDLE; 2] = [
            event.handle.as_raw_handle(),
            stop.event.handle.as_raw_handle(),
        ];
        // SAFETY: due handle di eventi validi; attesa senza limite, ma lo stop la interrompe.
        let which = unsafe { WaitForMultipleObjects(2, waited.as_ptr(), 0, INFINITE) };
        if which == WAIT_OBJECT_0 {
            // Operazione finita: si legge l'esito (attesa immediata).
            // SAFETY: la struttura dell'operazione avviata qui sopra su `handle`, viva.
            return match unsafe { cancel_and_wait(handle, pointer) } {
                Ok(_) => Ok(Connection::Connected),
                Err(error) if is_os_error(&error, ERROR_NO_DATA) => Ok(Connection::ClientGone),
                Err(error) => Err(error),
            };
        }
        // Stop, o attesa fallita: in ogni caso l'operazione si annulla e se ne aspetta la
        // fine prima di liberare struttura ed evento.
        let wait_error = (which == WAIT_FAILED).then(io::Error::last_os_error);
        // SAFETY: come sopra.
        let _finished = unsafe { cancel_and_wait(handle, pointer) };
        match wait_error {
            Some(error) => Err(error),
            None if which == WAIT_OBJECT_0 + 1 => Ok(Connection::Stopped),
            None => Err(io::Error::other(format!(
                "attesa della pipe: esito {which}"
            ))),
        }
    }

    /// Legge esattamente `buffer.len()` byte dal client connesso entro `deadline`.
    pub fn read_exact(&self, buffer: &mut [u8], deadline: Instant) -> io::Result<()> {
        pipe_read_exact(self.handle.as_handle(), buffer, deadline)
    }

    /// Scrive `bytes` al client connesso entro `deadline`.
    pub fn write_all(&self, bytes: &[u8], deadline: Instant) -> io::Result<()> {
        pipe_write_all(self.handle.as_handle(), bytes, deadline)
    }

    /// Chiude la connessione corrente; la pipe resta pronta per il client successivo.
    pub fn disconnect(&self) {
        // SAFETY: handle della pipe valido preso in prestito.
        unsafe { DisconnectNamedPipe(self.handle.as_raw_handle()) };
    }
}

/// Byte del buffer della pipe in ogni direzione.
const PIPE_BUFFER_BYTES: u32 = 64 * 1024;

/// Il capo client della pipe, aperto con SQoS Identification e I/O sovrapposto.
#[derive(Debug)]
pub struct PipeClient {
    file: File,
}

impl PipeClient {
    /// Si connette alla pipe `name` entro `deadline` (se è occupata da un altro client,
    /// aspetta che si liberi). Il server non può impersonare questo processo oltre il livello
    /// Identification.
    pub fn connect(name: &str, deadline: Instant) -> io::Result<PipeClient> {
        use std::os::windows::fs::OpenOptionsExt;
        loop {
            let opened = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .security_qos_flags(PIPE_CLIENT_SQOS)
                .custom_flags(FILE_FLAG_OVERLAPPED)
                .open(name);
            match opened {
                Ok(file) => return Ok(PipeClient { file }),
                Err(error) if error.raw_os_error() == i32::try_from(ERROR_PIPE_BUSY).ok() => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    let millis = u32::try_from(remaining.as_millis()).unwrap_or(u32::MAX);
                    if millis == 0 {
                        return Err(io::Error::from(io::ErrorKind::TimedOut));
                    }
                    let wide_name = wide(OsStr::new(name));
                    // SAFETY: stringa terminata da zero; attesa limitata a `millis`.
                    if unsafe { WaitNamedPipeW(wide_name.as_ptr(), millis) } == 0 {
                        return Err(io::Error::from(io::ErrorKind::TimedOut));
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Il processo che ha creato la pipe.
    pub fn server_pid(&self) -> io::Result<u32> {
        let mut pid = 0u32;
        // SAFETY: handle della pipe valido; puntatore d'uscita valido.
        if unsafe { GetNamedPipeServerProcessId(self.file.as_raw_handle(), &raw mut pid) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(pid)
    }

    /// Legge esattamente `buffer.len()` byte entro `deadline`.
    pub fn read_exact(&self, buffer: &mut [u8], deadline: Instant) -> io::Result<()> {
        pipe_read_exact(self.file.as_handle(), buffer, deadline)
    }

    /// Scrive `bytes` entro `deadline`.
    pub fn write_all(&self, bytes: &[u8], deadline: Instant) -> io::Result<()> {
        pipe_write_all(self.file.as_handle(), bytes, deadline)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use windows_sys::Win32::Foundation::GetHandleInformation;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETBEEP, SPI_SETBEEP};

    use super::*;

    /// Argomento che segna i figli avviati da questi test: senza, i test "figlio" non fanno
    /// nulla (anche se qualcuno li esegue con `--ignored`).
    const CHILD_MARKER: &str = "lynkeus-figlio-del-test-7c1d";

    fn is_child() -> bool {
        std::env::args().any(|a| a == CHILD_MARKER)
    }

    /// Avvia questo stesso eseguibile di test nel job, eseguendo solo il test `name`.
    fn spawn_test_child(name: &str, job: &Job) -> SpawnedChild {
        spawn_test_child_with(name, job, &[])
    }

    /// Come [`spawn_test_child`], con argomenti in più per il figlio (per libtest sono
    /// filtri che non corrispondono a nessun test).
    fn spawn_test_child_with(name: &str, job: &Job, extra: &[String]) -> SpawnedChild {
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
        .chain(extra.iter().cloned())
        .map(OsString::from)
        .collect();
        spawn_in_job(&exe, &args, job).expect("figlio avviato")
    }

    /// Lo stdout del figlio dopo la sua uscita (stdin chiuso subito).
    fn child_output(mut child: SpawnedChild) -> String {
        drop(child.stdin);
        let mut output = String::new();
        child
            .stdout
            .read_to_string(&mut output)
            .expect("lettura dello stdout");
        output
    }

    /// Il valore dopo `key` nella stampa del figlio (libtest può scrivere il nome del test
    /// sulla stessa riga, prima).
    fn reported<'a>(output: &'a str, key: &str) -> &'a str {
        output
            .split_once(key)
            .and_then(|(_, rest)| rest.lines().next())
            .unwrap_or_else(|| panic!("nessuna riga {key} in {output:?}"))
    }

    /// Prefisso dell'argomento con il valore dell'handle da cercare nel figlio.
    const HANDLE_ARG: &str = "lynkeus-handle=";

    #[test]
    #[ignore = "figlio dei test del Job Object"]
    fn child_looks_for_a_handle() {
        if is_child() {
            let value = std::env::args()
                .find_map(|arg| arg.strip_prefix(HANDLE_ARG).and_then(|v| v.parse().ok()));
            let visible = value.is_some_and(|value: usize| {
                let mut flags = 0u32;
                let handle: HANDLE = std::ptr::without_provenance_mut(value);
                // SAFETY: si chiede solo se il valore è un handle valido di questo processo;
                // un valore che non lo è dà un errore, nessun accesso alla memoria.
                let ok = unsafe { GetHandleInformation(handle, &raw mut flags) };
                // Un handle ereditato è ereditabile anche nel figlio; quelli del runtime no.
                ok != 0 && flags & HANDLE_FLAG_INHERIT != 0
            });
            println!("MANIGLIA:{}", if visible { "ereditata" } else { "assente" });
        }
    }

    /// T-D07: un handle ereditabile del padre, non in elenco, non arriva al figlio: il figlio
    /// eredita solo stdin e stdout (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`).
    #[test]
    fn child_inherits_only_stdio() {
        let attributes = SECURITY_ATTRIBUTES {
            nLength: struct_len::<SECURITY_ATTRIBUTES>(),
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        // SAFETY: attributi validi per la durata della chiamata, nessun nome; l'handle
        // restituito è posseduto solo da qui.
        let event = unsafe { owned(CreateEventW(&raw const attributes, 1, 0, std::ptr::null())) }
            .expect("evento ereditabile");
        let value = event.as_raw_handle().addr();
        let job = Job::new(256 * 1024 * 1024).expect("job");
        let child = spawn_test_child_with(
            "child_looks_for_a_handle",
            &job,
            &[format!("{HANDLE_ARG}{value}")],
        );
        let output = child_output(child);
        assert_eq!(reported(&output, "MANIGLIA:"), "assente", "{output}");
        drop(event);
    }

    #[test]
    #[ignore = "figlio dei test del Job Object"]
    fn child_tries_processes_and_system_settings() {
        if is_child() {
            let cmd = Path::new(&std::env::var_os("SystemRoot").unwrap_or_default())
                .join("System32")
                .join("cmd.exe");
            let started = std::process::Command::new(cmd)
                .args(["/d", "/c", "exit 0"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            println!(
                "AVVIO:{}",
                if started.is_ok() {
                    "riuscito"
                } else {
                    "negato"
                }
            );
            // Rimette il valore che c'è già, senza salvarlo né avvisare le finestre: nessun
            // cambiamento, ma un processo con le restrizioni dell'interfaccia non può.
            let mut beep = 0i32;
            // SAFETY: `beep` è un BOOL scrivibile vivo fino al ritorno.
            let read = unsafe {
                SystemParametersInfoW(SPI_GETBEEP, 0, (&raw mut beep).cast::<c_void>(), 0)
            };
            let same = u32::from(beep != 0);
            // SAFETY: SPI_SETBEEP vuole il valore in `uiParam`; nessun puntatore.
            let written =
                unsafe { SystemParametersInfoW(SPI_SETBEEP, same, std::ptr::null_mut(), 0) };
            let outcome = match (read != 0, written != 0) {
                (false, _) => "illeggibili",
                (true, false) => "negati",
                (true, true) => "modificabili",
            };
            println!("PARAMETRI:{outcome}");
        }
    }

    /// T-D10 (D-09): un processo estrattore compromesso non avvia altri programmi
    /// (`ActiveProcessLimit` = 1) e non cambia le impostazioni di sistema (restrizioni
    /// dell'interfaccia del job; appunti, atomi e finestre altrui hanno le stesse, senza una
    /// prova che non tocchi gli appunti dell'utente).
    #[test]
    fn job_limits_active_process_and_ui() {
        let job = Job::new(256 * 1024 * 1024).expect("job");
        let output = child_output(spawn_test_child(
            "child_tries_processes_and_system_settings",
            &job,
        ));
        assert_eq!(reported(&output, "AVVIO:"), "negato", "{output}");
        assert_eq!(reported(&output, "PARAMETRI:"), "negati", "{output}");
    }

    #[test]
    fn dangerous_file_types_by_the_system() {
        for dangerous in ["exe", "EXE", "bat", "ins", "x\0y", &"a".repeat(65)] {
            assert!(assoc_is_dangerous(dangerous), "{dangerous:?}");
        }
        for document in ["", "txt", "pdf", "docx", "odt"] {
            assert!(!assoc_is_dangerous(document), "{document:?}");
        }
    }

    /// La domanda sul contrasto elevato risponde senza errori (il valore dipende dal
    /// sistema): i colori ci sono se e solo se il tema è attivo, e due domande di fila danno
    /// la stessa risposta.
    #[test]
    fn the_high_contrast_query_answers() {
        let colors = high_contrast_colors();
        assert_eq!(colors.is_some(), high_contrast_active());
        assert_eq!(colors, high_contrast_colors());
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

    // --- Cartelle e file privati, identità, pipe ------------------------------------------

    fn system() -> Sid {
        Sid::well_known(KnownSid::LocalSystem).expect("SID di SYSTEM")
    }

    /// Una cartella di prova sotto il profilo dell'utente (`%LOCALAPPDATA%`), come le
    /// cartelle vere dell'app; la cartella temporanea va bene uguale.
    fn scratch() -> tempfile::TempDir {
        tempfile::tempdir().expect("cartella temporanea")
    }

    /// La DACL letta dopo la creazione: protetta, proprietario l'utente, solo utente e
    /// SYSTEM con accesso completo ereditato da sottocartelle e file.
    #[test]
    fn private_dirs_have_an_explicit_protected_dacl() {
        let base = scratch();
        let dir = base.path().join("dati").join("cache");
        create_private_dir(&dir).expect("creata");
        let user = current_user().expect("utente");
        for created in [base.path().join("dati"), dir.clone()] {
            let info = read_security(&created).expect("sicurezza letta");
            assert!(info.protected, "ereditarietà dal padre attiva");
            assert_eq!(info.owner, user);
            let dacl = info.dacl.expect("DACL presente");
            let sids: Vec<&Sid> = dacl.iter().map(|ace| &ace.sid).collect();
            assert_eq!(sids, [&user, &system()], "{dacl:?}");
            for ace in &dacl {
                assert!(ace.allows && !ace.inherit_only);
                // OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE.
                assert_eq!(ace.flags & 0x3, 0x3);
                assert_eq!(ace.mask & 0x1F_01FF, 0x1F_01FF, "FILE_ALL_ACCESS");
            }
        }
        // Una seconda volta: esiste ed è privata.
        create_private_dir(&dir).expect("già privata");
    }

    #[test]
    fn private_files_have_an_explicit_dacl_and_are_new_only() {
        let base = scratch();
        let dir = base.path().join("p");
        create_private_dir(&dir).expect("cartella");
        let path = dir.join("token");
        let mut file = create_new_private_file(&path).expect("file");
        std::io::Write::write_all(&mut file, b"x").expect("scrittura");
        drop(file);
        let info = read_security(&path).expect("sicurezza letta");
        assert!(info.protected);
        let sids: Vec<Sid> = info
            .dacl
            .expect("DACL")
            .into_iter()
            .map(|ace| ace.sid)
            .collect();
        assert_eq!(sids, [current_user().expect("utente"), system()]);
        let again = create_new_private_file(&path).map(|_| ());
        assert_eq!(
            again.map_err(|e| e.kind()),
            Err(io::ErrorKind::AlreadyExists)
        );
    }

    /// Esegue `icacls` (percorso assoluto nella cartella di sistema) su `path`.
    fn icacls(path: &Path, args: &[&str]) {
        let root = std::env::var_os("SystemRoot").expect("SystemRoot");
        let exe = Path::new(&root).join("System32").join("icacls.exe");
        let status = std::process::Command::new(exe)
            .arg(path)
            .args(args)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("icacls avviato");
        assert!(status.success(), "icacls {args:?}");
    }

    /// Una cartella preesistente con una voce per `Users` (S-1-5-32-545): rifiutata, e non
    /// modificata.
    #[test]
    fn an_existing_dir_open_to_others_is_refused() {
        let base = scratch();
        let dir = base.path().join("aperta");
        std::fs::create_dir(&dir).expect("cartella");
        create_private_dir(&dir).expect("ereditata dal profilo: privata");
        icacls(&dir, &["/grant", "*S-1-5-32-545:(OI)(CI)M"]);
        let refused = create_private_dir(&dir).expect_err("rifiutata");
        assert!(lynkeus_engine::NotPrivate::is(&refused), "{refused:?}");
        assert!(writable_by_others(
            &read_security(&dir).expect("DACL letta"),
            &program_trusted_sids(&current_user().expect("utente")).expect("SID")
        ));
        let users = Sid::from_sddl("S-1-5-32-545").expect("SID");
        let info = read_security(&dir).expect("sicurezza");
        assert!(
            info.dacl.expect("DACL").iter().any(|ace| ace.sid == users),
            "la voce estranea è stata tolta"
        );
    }

    #[test]
    fn privacy_rules_on_dacls() {
        let user = current_user().expect("utente");
        let trusted = trusted_sids(&user).expect("SID fidati");
        let everyone = Sid::from_sddl("S-1-1-0").expect("Everyone");
        let ace = |sid: &Sid, allows: bool, inherit_only: bool, mask: u32| Ace {
            allows,
            inherit_only,
            flags: if inherit_only { 0xB } else { 0x3 },
            mask,
            sid: sid.clone(),
        };
        let info = |owner: &Sid, dacl: Option<Vec<Ace>>| SecurityInfo {
            owner: owner.clone(),
            dacl,
            protected: true,
        };
        let mine = info(&user, Some(vec![ace(&user, true, false, 0x1F_01FF)]));
        assert!(is_private(&mine, &trusted));
        assert!(!writable_by_others(&mine, &trusted));
        // DACL nulla: tutto a tutti.
        assert!(!is_private(&info(&user, None), &trusted));
        assert!(writable_by_others(&info(&user, None), &trusted));
        // Proprietario estraneo.
        assert!(!is_private(&info(&everyone, Some(vec![])), &trusted));
        // Lettura per Everyone: non privata, ma non scrivibile.
        let readable = info(&user, Some(vec![ace(&everyone, true, false, 0x1200A9)]));
        assert!(!is_private(&readable, &trusted));
        assert!(!writable_by_others(&readable, &trusted));
        // Una negazione per Everyone non apre nulla.
        let denied = info(&user, Some(vec![ace(&everyone, false, false, 0x1F_01FF)]));
        assert!(is_private(&denied, &trusted));
        // Scrittura solo per gli oggetti creati dentro: non conta per la cartella stessa,
        // ma la cartella non è privata.
        let inherited = info(&user, Some(vec![ace(&everyone, true, true, 0x2)]));
        assert!(!writable_by_others(&inherited, &trusted));
        assert!(!is_private(&inherited, &trusted));
        let writable = info(&user, Some(vec![ace(&everyone, true, false, 0x2)]));
        assert!(writable_by_others(&writable, &trusted));
    }

    /// Le regole per l'eseguibile e la sua cartella: proprietario fidato, segnaposti del
    /// proprietario non fidati.
    #[test]
    fn program_rules_on_owners_and_placeholders() {
        let user = current_user().expect("utente");
        let trusted = program_trusted_sids(&user).expect("SID fidati");
        let full = |sid: &Sid| Ace {
            allows: true,
            inherit_only: false,
            flags: 0x3,
            mask: 0x1F_01FF,
            sid: sid.clone(),
        };
        let info = |owner: &Sid, aces: Vec<Ace>| SecurityInfo {
            owner: owner.clone(),
            dacl: Some(aces),
            protected: true,
        };
        let installer = Sid::from_sddl(TRUSTED_INSTALLER_SDDL).expect("TrustedInstaller");
        let admins = Sid::well_known(KnownSid::Administrators).expect("Administrators");
        for owner in [&user, &system(), &admins, &installer] {
            let ok = info(owner, vec![full(&user), full(&installer)]);
            assert!(!writable_by_others(&ok, &trusted), "{owner:?}");
        }
        // Proprietario estraneo con una DACL innocua: può cambiarla, quindi modificabile.
        let users = Sid::from_sddl("S-1-5-32-545").expect("Users");
        assert!(writable_by_others(
            &info(&users, vec![full(&user)]),
            &trusted
        ));
        // OWNER RIGHTS e CREATOR OWNER applicati all'oggetto stesso non sono fidati.
        for placeholder in [KnownSid::OwnerRights, KnownSid::CreatorOwner] {
            let sid = Sid::well_known(placeholder).expect("segnaposto");
            assert!(
                writable_by_others(&info(&user, vec![full(&sid)]), &trusted),
                "{placeholder:?}"
            );
        }
    }

    /// Un eseguibile in una cartella privata non è modificabile da altri; con una voce di
    /// scrittura per `Users` sul solo file sì.
    #[test]
    fn an_exe_writable_by_users_is_detected() {
        let base = scratch();
        let dir = base.path().join("programma");
        create_private_dir(&dir).expect("cartella");
        let exe = dir.join("lynkeus-searcher.exe");
        drop(create_new_private_file(&exe).expect("file"));
        assert!(!exe_writable_by_others(&exe).expect("controllo"));
        icacls(&exe, &["/grant", "*S-1-5-32-545:(M)"]);
        assert!(exe_writable_by_others(&exe).expect("controllo"));
    }

    #[test]
    fn the_current_process_belongs_to_the_current_user() {
        let user = current_user().expect("utente");
        assert!(user.to_sddl().expect("testo").starts_with("S-1-5-"));
        assert_eq!(process_user(std::process::id()).expect("token"), user);
        assert_ne!(user, system());
    }

    #[test]
    fn the_pipe_client_asks_for_identification_only() {
        assert_eq!(
            PIPE_CLIENT_SQOS & SECURITY_SQOS_PRESENT,
            SECURITY_SQOS_PRESENT
        );
        assert_eq!(
            PIPE_CLIENT_SQOS & SECURITY_IDENTIFICATION,
            SECURITY_IDENTIFICATION
        );
        // Identification è il livello 1 (bit 16-17): non Impersonation né Delegation.
        assert_eq!((PIPE_CLIENT_SQOS >> 16) & 0x3, 1);
    }

    fn pipe_name() -> String {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).expect("casuale");
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        format!(r"\\.\pipe\lynkeus-test-{hex}")
    }

    #[test]
    fn a_pipe_name_already_in_use_is_refused() {
        let name = pipe_name();
        let first = PipeServer::create(&name).expect("prima pipe");
        assert!(PipeServer::create(&name).is_err(), "seconda istanza creata");
        drop(first);
    }

    #[test]
    fn pipe_round_trip_with_timeouts() {
        let name = pipe_name();
        let server = PipeServer::create(&name).expect("pipe");
        let stop = StopSignal::new().expect("stop");
        let client_name = name.clone();
        let client = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let client = PipeClient::connect(&client_name, deadline).expect("connesso");
            assert_eq!(client.server_pid().expect("pid"), std::process::id());
            client.write_all(b"ciao", deadline).expect("scritto");
            let mut reply = [0u8; 1];
            client.read_exact(&mut reply, deadline).expect("risposta");
            reply[0]
        });
        assert!(server.accept(&stop).expect("accettato"));
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut message = [0u8; 4];
        server.read_exact(&mut message, deadline).expect("letto");
        assert_eq!(&message, b"ciao");
        server.write_all(&[7], deadline).expect("risposta");
        assert_eq!(client.join().expect("client"), 7);
        // Il client ha chiuso: fine del flusso, poi il server si libera.
        let mut more = [0u8; 1];
        let end = server.read_exact(&mut more, deadline).map_err(|e| e.kind());
        assert_eq!(end, Err(io::ErrorKind::UnexpectedEof));
        server.disconnect();

        // Un client che non scrive: il server riprende dopo il tempo limite. (Dopo
        // `disconnect` il client aspetta il prossimo `accept`: si connette da un altro
        // thread.)
        let (connected_tx, connected_rx) = std::sync::mpsc::channel();
        let silent = std::thread::spawn(move || {
            let client = PipeClient::connect(&name, Instant::now() + Duration::from_secs(5));
            let _ = connected_rx.recv_timeout(Duration::from_secs(5));
            client.map(|_| ())
        });
        assert!(server.accept(&stop).expect("accettato"));
        let started = Instant::now();
        let waited = server
            .read_exact(&mut message, started + Duration::from_millis(200))
            .map_err(|e| e.kind());
        assert_eq!(waited, Err(io::ErrorKind::TimedOut));
        assert!(started.elapsed() < Duration::from_secs(2));
        connected_tx.send(()).expect("client vivo");
        assert!(silent.join().expect("client").is_ok());
        server.disconnect();

        // La DACL della pipe: protetta, solo l'utente.
        let info = security_of(&server.handle).expect("sicurezza della pipe");
        assert!(info.protected);
        let sids: Vec<Sid> = info
            .dacl
            .expect("DACL")
            .into_iter()
            .map(|ace| ace.sid)
            .collect();
        assert_eq!(sids, [current_user().expect("utente")]);

        // Lo stop interrompe l'attesa di un client.
        stop.set();
        assert!(!server.accept(&stop).expect("attesa interrotta"));
    }

    /// Una lettura con la scadenza già passata finisce con `TimedOut` e annulla davvero
    /// l'operazione: i byte scritti dopo dal client arrivano alla lettura successiva (con
    /// l'operazione rimasta in corso li avrebbe presi il buffer già liberato).
    #[test]
    fn an_expired_deadline_cancels_the_pending_read() {
        let name = pipe_name();
        let server = PipeServer::create(&name).expect("pipe");
        let stop = StopSignal::new().expect("stop");
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let client = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let client = PipeClient::connect(&name, deadline).expect("connesso");
            go_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("via libera");
            client.write_all(b"dopo", deadline).expect("scritto");
            let mut reply = [0u8; 1];
            client.read_exact(&mut reply, deadline).expect("risposta");
        });
        assert!(server.accept(&stop).expect("accettato"));
        let mut early = [0u8; 4];
        for _ in 0..3 {
            let expired = server
                .read_exact(&mut early, Instant::now())
                .map_err(|e| e.kind());
            assert_eq!(expired, Err(io::ErrorKind::TimedOut));
        }
        go_tx.send(()).expect("client vivo");
        let mut message = [0u8; 4];
        let deadline = Instant::now() + Duration::from_secs(5);
        server.read_exact(&mut message, deadline).expect("letto");
        assert_eq!(&message, b"dopo");
        server.write_all(&[1], deadline).expect("risposta");
        client.join().expect("client");
        server.disconnect();
    }

    /// Un client che si connette e chiude prima che il server lo accetti
    /// (`ERROR_NO_DATA`) non ferma il server: il client successivo viene accettato.
    #[test]
    fn a_client_that_closes_at_once_does_not_stop_the_server() {
        let name = pipe_name();
        let server = PipeServer::create(&name).expect("pipe");
        let stop = StopSignal::new().expect("stop");
        let deadline = Instant::now() + Duration::from_secs(5);
        drop(PipeClient::connect(&name, deadline).expect("primo client"));
        let second = std::thread::spawn(move || {
            let client = PipeClient::connect(&name, deadline).expect("secondo client");
            client.write_all(b"x", deadline).expect("scritto");
            let mut reply = [0u8; 1];
            client.read_exact(&mut reply, deadline).expect("risposta");
        });
        assert!(server.accept(&stop).expect("il server continua"));
        let mut byte = [0u8; 1];
        server.read_exact(&mut byte, deadline).expect("letto");
        assert_eq!(&byte, b"x");
        server.write_all(&[1], deadline).expect("risposta");
        second.join().expect("client");
        server.disconnect();
    }
}
