//! Windows console handling for the windowed entry points (`--gui`, setup, autostart).
//!
//! The binary uses the console subsystem so the CLI keeps working from a terminal, which
//! means Windows opens a console window whenever it's started any other way (double-click,
//! shortcut, `Run` key). These are the only places that need `unsafe` (raw Win32 calls).

#[cfg(windows)]
#[allow(unsafe_code)]
pub(crate) fn hide_own_console() {
    use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};

    let mut pids = [0u32; 2];
    // SAFETY: `pids` is valid for writes of `pids.len()` u32s for the duration of the call.
    let attached = unsafe { GetConsoleProcessList(pids.as_mut_ptr(), 2) };
    // Only this process is attached, so the console was created just for us (not a
    // terminal the user launched us from); detaching closes the window.
    if attached == 1 {
        // SAFETY: no preconditions; we never touch the console handles afterwards except
        // through std, whose writes to a detached console fail silently.
        unsafe { FreeConsole() };
    }
}

#[cfg(not(windows))]
pub(crate) fn hide_own_console() {}

/// Shows a modal message box; the only feedback channel once the console is hidden.
#[cfg(windows)]
#[allow(unsafe_code)]
pub(crate) fn message_box(text: &str, is_error: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW,
    };

    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let text = wide(text);
    let caption = wide("Deadlock API Ingest");
    let icon = if is_error {
        MB_ICONERROR
    } else {
        MB_ICONINFORMATION
    };
    // SAFETY: both strings are NUL-terminated UTF-16 buffers that outlive the call; a null
    // owner window is allowed.
    unsafe {
        MessageBoxW(
            core::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | icon,
        )
    };
}

#[cfg(not(windows))]
pub(crate) fn message_box(text: &str, is_error: bool) {
    if is_error {
        tracing::error!("{text}");
    } else {
        tracing::info!("{text}");
    }
}
