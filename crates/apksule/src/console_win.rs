//! Attach a console for CLI flags when the binary uses the Windows GUI subsystem.

#[cfg(windows)]
#[allow(clippy::upper_case_acronyms)]
pub fn ensure_cli_console() {
    // SAFETY: Win32 console attach/alloc is process-local and called before any
    // multithreaded CLI I/O; failure is non-fatal (stdout stays disconnected).
    unsafe {
        type Bool = i32;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn AttachConsole(dw_process_id: u32) -> Bool;
            fn AllocConsole() -> Bool;
        }
        const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            let _ = AllocConsole();
        }
    }
}

#[cfg(not(windows))]
pub fn ensure_cli_console() {}
