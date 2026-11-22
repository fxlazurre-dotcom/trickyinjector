//! The file picker: GetOpenFileNameW from comdlg32.
//!
//! rfd (IFileOpenDialog over shell COM) returned None inside this host: the main
//! thread lives in the WebView2/tao COM neighborhood and the dialog result came
//! back cancelled even after a successful pick. GetOpenFileNameW needs no shell
//! COM and has worked in every GUI app since Windows 95.

use std::path::PathBuf;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY,
    OFN_PATHMUSTEXIST, OPENFILENAMEW,
};

/// What the file dialog reported.
pub enum PickOutcome {
    /// The user picked a file.
    Selected(PathBuf),
    /// The dialog was closed without a pick.
    Canceled,
    /// The dialog reported a common-dialog error code.
    Failed(u32),
}

/// Shows the owned file picker and reports the outcome.
pub fn pick_dll_file(parent: HWND) -> PickOutcome {
    let filter: Vec<u16> = "DLL files (*.dll)\0*.dll\0All files (*.*)\0*.*\0\0"
        .encode_utf16()
        .collect();
    let title: Vec<u16> = "Pick a DLL to inject\0".encode_utf16().collect();
    let mut file = vec![0u16; 512];

    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = parent;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = file.as_mut_ptr();
    ofn.nMaxFile = file.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY;

    let ok = unsafe { GetOpenFileNameW(&mut ofn) } != 0;
    if !ok {
        let err = unsafe { CommDlgExtendedError() };
        return if err == 0 {
            PickOutcome::Canceled
        } else {
            PickOutcome::Failed(err)
        };
    }

    let len = file.iter().position(|&c| c == 0).unwrap_or(file.len());
    if len == 0 {
        return PickOutcome::Failed(0);
    }
    PickOutcome::Selected(PathBuf::from(String::from_utf16_lossy(&file[..len])))
}
