//! Windows-only FFI: Job Objects, console modes, and console query probing.
#![allow(unsafe_code)]

use std::io;
use std::ptr;
use std::sync::OnceLock;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, WriteFile,
};
use windows_sys::Win32::System::Console::{
    CONSOLE_MODE, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
    ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleCP,
    GetConsoleMode, GetConsoleOutputCP, GetNumberOfConsoleInputEvents, INPUT_RECORD, KEY_EVENT,
    ReadConsoleInputW, SetConsoleCP, SetConsoleMode, SetConsoleOutputCP,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, WaitForSingleObject};

const CP_UTF8: u32 = 65001;

/// An owned Win32 handle, closed on drop.
struct Owned(HANDLE);

// SAFETY: a console/job HANDLE is a kernel object reference usable from any thread.
unsafe impl Send for Owned {}
// SAFETY: as above; we never mutate the handle value itself.
unsafe impl Sync for Owned {}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: we own this handle and close it exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn open_console(name: &str) -> io::Result<Owned> {
    let name = wide(name);
    // SAFETY: `name` is a valid NUL-terminated UTF-16 string that outlives the call;
    // all pointer arguments are either valid or null where the API allows null.
    let h = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE || h.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(Owned(h))
    }
}

fn get_mode(h: HANDLE) -> io::Result<CONSOLE_MODE> {
    let mut mode: CONSOLE_MODE = 0;
    // SAFETY: `h` is a console handle and `mode` is a valid out-pointer.
    if unsafe { GetConsoleMode(h, &mut mode) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(mode)
    }
}

fn set_mode(h: HANDLE, mode: CONSOLE_MODE) -> io::Result<()> {
    // SAFETY: `h` is a console handle.
    if unsafe { SetConsoleMode(h, mode) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Put the current process in a Job Object that kills every member when the
/// last handle closes (i.e. when this process dies, however it dies).
/// `BREAKAWAY_OK` lets intentional daemons (session servers) opt out.
pub fn ensure_kill_on_close_job() -> io::Result<()> {
    static JOB: OnceLock<Result<Owned, i32>> = OnceLock::new();
    let res = JOB.get_or_init(|| {
        // SAFETY: null attributes and name are allowed; the returned handle is checked.
        let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error().raw_os_error().unwrap_or(-1));
        }
        let job = Owned(job);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = Default::default();
        info.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
        // SAFETY: `info` is a properly initialised struct of the size we pass.
        let ok = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error().raw_os_error().unwrap_or(-1));
        }
        // SAFETY: both handles are valid; GetCurrentProcess returns a pseudo-handle.
        if unsafe { AssignProcessToJobObject(job.0, GetCurrentProcess()) } == 0 {
            return Err(io::Error::last_os_error().raw_os_error().unwrap_or(-1));
        }
        // Keep the handle open for the whole process lifetime (stored in the OnceLock).
        Ok(job)
    });
    match res {
        Ok(_) => Ok(()),
        Err(code) => Err(io::Error::from_raw_os_error(*code)),
    }
}

/// Saved console state, restored on drop.
pub struct ConsoleState {
    out: Owned,
    out_mode: CONSOLE_MODE,
    out_cp: u32,
    in_cp: u32,
}

impl ConsoleState {
    /// Enable VT output processing and UTF-8 code pages. `None` if there is no console.
    pub fn enter() -> Option<Self> {
        let out = open_console("CONOUT$").ok()?;
        let out_mode = get_mode(out.0).ok()?;
        // SAFETY: plain getters with no arguments.
        let (out_cp, in_cp) = unsafe { (GetConsoleOutputCP(), GetConsoleCP()) };
        let _ = set_mode(out.0, out_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        // SAFETY: setting a code page has no memory-safety preconditions.
        unsafe {
            SetConsoleOutputCP(CP_UTF8);
            SetConsoleCP(CP_UTF8);
        }
        Some(Self {
            out,
            out_mode,
            out_cp,
            in_cp,
        })
    }
}

impl Drop for ConsoleState {
    fn drop(&mut self) {
        let _ = set_mode(self.out.0, self.out_mode);
        // SAFETY: restoring previously read code pages.
        unsafe {
            SetConsoleOutputCP(self.out_cp);
            SetConsoleCP(self.in_cp);
        }
    }
}

/// Console handles prepared for a capability probe (VT input on, line/echo off).
pub struct ProbeConsole {
    input: Owned,
    output: Owned,
    in_mode: CONSOLE_MODE,
    out_mode: CONSOLE_MODE,
}

impl ProbeConsole {
    pub fn open() -> io::Result<Self> {
        let input = open_console("CONIN$")?;
        let output = open_console("CONOUT$")?;
        let in_mode = get_mode(input.0)?;
        let out_mode = get_mode(output.0)?;
        set_mode(
            input.0,
            (in_mode | ENABLE_VIRTUAL_TERMINAL_INPUT)
                & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT),
        )?;
        set_mode(output.0, out_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING)?;
        Ok(Self {
            input,
            output,
            in_mode,
            out_mode,
        })
    }

    pub fn write_all(&self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            let mut written = 0u32;
            let len = bytes.len().min(u32::MAX as usize) as u32;
            // SAFETY: `bytes` is valid for `len` bytes; `written` is a valid out-pointer;
            // a null OVERLAPPED means synchronous I/O.
            let ok = unsafe {
                WriteFile(
                    self.output.0,
                    bytes.as_ptr(),
                    len,
                    &mut written,
                    ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            bytes = &bytes[written as usize..];
        }
        Ok(())
    }

    /// Wait up to `timeout` for input and append any typed characters (VT replies) to `buf`.
    pub fn read_chars(&self, timeout: Duration, buf: &mut Vec<u8>) -> io::Result<()> {
        let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
        // SAFETY: waiting on a valid console input handle.
        if unsafe { WaitForSingleObject(self.input.0, ms) } != WAIT_OBJECT_0 {
            return Ok(());
        }
        let mut pending = 0u32;
        // SAFETY: valid handle and out-pointer.
        if unsafe { GetNumberOfConsoleInputEvents(self.input.0, &mut pending) } == 0 || pending == 0
        {
            return Ok(());
        }
        let mut records: Vec<INPUT_RECORD> = (0..pending.min(256))
            .map(|_| INPUT_RECORD::default())
            .collect();
        let mut read = 0u32;
        // SAFETY: `records` has room for `records.len()` entries; `read` is a valid out-pointer.
        let ok = unsafe {
            ReadConsoleInputW(
                self.input.0,
                records.as_mut_ptr(),
                records.len() as u32,
                &mut read,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut units = Vec::new();
        for rec in &records[..read as usize] {
            if u32::from(rec.EventType) == KEY_EVENT {
                // SAFETY: EventType == KEY_EVENT means the KeyEvent union member is active.
                let key = unsafe { rec.Event.KeyEvent };
                // SAFETY: UnicodeChar is the member populated by ReadConsoleInputW.
                let ch = unsafe { key.uChar.UnicodeChar };
                if key.bKeyDown != 0 && ch != 0 {
                    units.push(ch);
                }
            }
        }
        buf.extend(String::from_utf16_lossy(&units).as_bytes());
        Ok(())
    }
}

impl Drop for ProbeConsole {
    fn drop(&mut self) {
        let _ = set_mode(self.input.0, self.in_mode);
        let _ = set_mode(self.output.0, self.out_mode);
    }
}
