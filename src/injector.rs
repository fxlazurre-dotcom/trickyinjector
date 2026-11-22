//! A classic LoadLibrary injector via CreateRemoteThread
//! plus process lookup through Toolhelp32.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows_sys::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows_sys::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, OpenProcess, WaitForSingleObject,
    LPTHREAD_START_ROUTINE, PROCESS_CREATE_THREAD, PROCESS_QUERY_INFORMATION,
    PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
};

/// The MACHINE code for x64 in a PE header.
pub const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;

const PROCESS_RIGHTS: u32 = PROCESS_CREATE_THREAD
    | PROCESS_QUERY_INFORMATION
    | PROCESS_VM_OPERATION
    | PROCESS_VM_READ
    | PROCESS_VM_WRITE;

type RemoteThreadFn = unsafe extern "system" fn(*mut c_void) -> u32;

/// Finds the PIDs of every process with the given exe name (case insensitive).
pub fn find_process_pids(exe_name: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return pids;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                if name.eq_ignore_ascii_case(exe_name) {
                    pids.push(entry.th32ProcessID);
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    pids
}

/// Injects a DLL into a process: LoadLibraryW through CreateRemoteThread.
pub fn inject_dll(pid: u32, dll_path: &Path) -> Result<(), String> {
    if !dll_path.exists() {
        return Err(format!("DLL file not found: {}", dll_path.display()));
    }
    // LoadLibrary in the target process resolves the path relative to itself, so it must be absolute
    let dll_abs = dll_path
        .canonicalize()
        .map_err(|e| format!("Failed to resolve the absolute DLL path: {e}"))?;
    // UTF-16 plus a NUL terminator for LoadLibraryW (Cyrillic paths keep working)
    let path_w: Vec<u16> = dll_abs
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let path_len = path_w.len() * std::mem::size_of::<u16>();

    unsafe {
        let process = OpenProcess(PROCESS_RIGHTS, 0, pid);
        if process.is_null() {
            let err = GetLastError();
            let hint = if err == 5 {
                ", run the injector as administrator"
            } else {
                ""
            };
            return Err(format!(
                "OpenProcess(PID {pid}) failed with Win32 error {err}{hint}"
            ));
        }
        let result = inject_into_handle(process, &path_w, path_len);
        CloseHandle(process);
        result
    }
}

unsafe fn inject_into_handle(
    process: HANDLE,
    path_w: &[u16],
    path_len: usize,
) -> Result<(), String> {
    // 1. Allocate memory for the DLL path in the target process
    let remote_mem = VirtualAllocEx(
        process,
        std::ptr::null(),
        path_len,
        MEM_COMMIT | MEM_RESERVE,
        PAGE_READWRITE,
    );
    if remote_mem.is_null() {
        return Err(format!("VirtualAllocEx failed with Win32 error {}", GetLastError()));
    }

    // 2. Write the path there
    let mut written: usize = 0;
    let ok = WriteProcessMemory(
        process,
        remote_mem.cast_const(),
        path_w.as_ptr().cast(),
        path_len,
        &mut written,
    );
    if ok == 0 || written != path_len {
        VirtualFreeEx(process, remote_mem, 0, MEM_RELEASE);
        return Err(format!("WriteProcessMemory failed with Win32 error {}", GetLastError()));
    }

    // 3. The LoadLibraryW address from kernel32 (the kernel32 base is shared by all processes)
    let kernel32 = GetModuleHandleA(b"kernel32.dll\0".as_ptr());
    if kernel32.is_null() {
        VirtualFreeEx(process, remote_mem, 0, MEM_RELEASE);
        return Err("GetModuleHandleA(kernel32) returned NULL".into());
    }
    let load_library_w = GetProcAddress(kernel32, b"LoadLibraryW\0".as_ptr());
    let addr = match load_library_w {
        Some(f) => f as usize,
        None => {
            VirtualFreeEx(process, remote_mem, 0, MEM_RELEASE);
            return Err("Failed to resolve LoadLibraryW in kernel32".into());
        }
    };
    let start_routine: LPTHREAD_START_ROUTINE =
        Some(std::mem::transmute::<usize, RemoteThreadFn>(addr));

    // 4. Start the remote thread: LoadLibraryW(path_to_dll)
    let thread = CreateRemoteThread(
        process,
        std::ptr::null(),
        0,
        start_routine,
        remote_mem.cast_const(),
        0,
        std::ptr::null_mut(),
    );
    if thread.is_null() {
        VirtualFreeEx(process, remote_mem, 0, MEM_RELEASE);
        return Err(format!("CreateRemoteThread failed with Win32 error {}", GetLastError()));
    }

    // 5. Wait for it: the exit code is the HMODULE (0 means LoadLibrary failed to load the DLL)
    let wait = WaitForSingleObject(thread, 10_000);
    let mut exit_code: u32 = 0;
    GetExitCodeThread(thread, &mut exit_code);

    VirtualFreeEx(process, remote_mem, 0, MEM_RELEASE);
    CloseHandle(thread);

    if wait != 0 {
        return Err("Timed out waiting for the remote thread".into());
    }
    if exit_code == 0 {
        return Err(
            "LoadLibraryW returned NULL in the target process. Check that the DLL is built \
             for x64 and that all of its dependencies (e.g. the MSVC runtime) are available"
                .into(),
        );
    }
    Ok(())
}

/// Returns the MACHINE code of the PE file (0x8664 means x64).
pub fn dll_machine(dll_path: &Path) -> Result<u16, String> {
    let data =
        std::fs::read(dll_path).map_err(|e| format!("Failed to read the DLL: {e}"))?;
    if data.len() < 0x40 || data[0..2] != *b"MZ" {
        return Err(format!("{}: not a PE file (no MZ signature)", dll_path.display()));
    }
    let pe_off = u32::from_le_bytes(data[0x3c..0x40].try_into().unwrap()) as usize;
    if data.len() < pe_off + 6 || data[pe_off..pe_off + 4] != *b"PE\0\0" {
        return Err(format!("{}: not a PE file (no PE signature)", dll_path.display()));
    }
    Ok(u16::from_le_bytes(data[pe_off + 4..pe_off + 6].try_into().unwrap()))
}
