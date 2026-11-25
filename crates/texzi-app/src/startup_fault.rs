//! Process-local test fixture for the pinned NSH Win32 creation boundary.
//! No system registration, files, hooks or another process are modified.

use nsh_platform_windows::windows::{
    Win32::{
        Foundation::{ERROR_ACCESS_DENIED, HINSTANCE, HWND, LPARAM, LRESULT, SetLastError, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{DefWindowProcW, RegisterClassExW, WM_NCCREATE, WNDCLASSEXW},
    },
    core::{Result, w},
};

unsafe extern "system" fn reject_creation(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        log::warn!("TX00 startup fixture: rejecting real NshWindow WM_NCCREATE");
        // SAFETY: only this thread's last-error slot is changed.
        unsafe { SetLastError(ERROR_ACCESS_DENIED) };
        return LRESULT(0);
    }
    // SAFETY: the OS supplied the window-procedure arguments.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

pub fn arm() -> Result<()> {
    // The pinned backend uses the executable's HINSTANCE and NshWindow.
    // Its subsequent RegisterClassExW sees this process-local class already
    // registered. Its actual CreateWindowExW then calls our rejecting proc,
    // propagating an OS creation failure through AppView::make/run_app.
    // SAFETY: strings and callback remain valid for the entire process.
    unsafe {
        let module = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            hInstance: HINSTANCE(module.0),
            lpfnWndProc: Some(reject_creation),
            lpszClassName: w!("NshWindow"),
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(nsh_platform_windows::windows::core::Error::from_thread());
        }
    }
    log::warn!("TX00 startup fixture armed; entering unchanged NSH run_app initialization");
    Ok(())
}
