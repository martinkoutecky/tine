//! GH #1043: on Windows a frameless window was restored one title bar taller
//! than it was saved, and grew again at every restart.
//!
//! Tao hides a frameless window's caption in its `WM_NCCALCSIZE` handler, but
//! the frame Windows computes while the window is being created never reaches
//! that handler, so a new frameless window still reports a caption-sized
//! non-client area (window minus client 16x39 at 100 %, where its frame is
//! 16x8). The window-state plugin's restore calls `set_inner_size`, which adds
//! that stale difference to the saved size; the handler then gives the window
//! a client area 31 px taller than saved (30 on Windows 11), and the plugin
//! records the taller size at the next resize or close.
//!
//! This plugin is registered before the window-state plugin, so for each
//! window the plugin restores it asks Windows to recompute the frame first
//! (`SWP_FRAMECHANGED`, nothing moved or resized). The restore then measures the
//! real frame and the shown size is the saved size. The saved size stays the
//! plugin's; a window whose frame is already right is left as it is.

/// Refreshes the frame of every frameless window `tracks` names, before any
/// later plugin's `on_window_ready` sees the window.
pub(crate) fn plugin(
    tracks: impl Fn(&str) -> bool + Send + Sync + 'static,
) -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("tine-window-frame")
        .on_window_ready(move |window| {
            if !tracks(window.label()) || window.is_decorated().unwrap_or(true) {
                return;
            }
            if let Ok(hwnd) = window.hwnd() {
                refresh_frame(hwnd.0 as windows_sys::Win32::Foundation::HWND);
            }
        })
        .build()
}

fn refresh_frame(hwnd: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
        SWP_NOZORDER,
    };
    // SAFETY: `hwnd` is the live window Tauri just reported ready; the call
    // only asks Windows to send it a fresh WM_NCCALCSIZE.
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED
                | SWP_NOMOVE
                | SWP_NOSIZE
                | SWP_NOZORDER
                | SWP_NOOWNERZORDER
                | SWP_NOACTIVATE,
        );
    }
}
