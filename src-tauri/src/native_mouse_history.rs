use tauri::Emitter;

const HISTORY_NAVIGATE_EVENT: &str = "history-navigate";

#[derive(Clone, serde::Serialize)]
struct HistoryNavigatePayload {
    direction: &'static str,
    target: String,
}

fn emit_navigation(window: &tauri::WebviewWindow, direction: &'static str) {
    let _ = window.emit(
        HISTORY_NAVIGATE_EVENT,
        HistoryNavigatePayload {
            direction,
            // Tauri's Emitter::emit broadcasts to every webview. Keep the
            // originating graph authoritative in the frontend listener.
            target: window.label().to_string(),
        },
    );
}

/// Install the graph window's native history handler. Linux owns side buttons
/// at the GTK widget. Windows divides ownership by the origin of the command.
pub(crate) fn install(window: &tauri::WebviewWindow) {
    platform::install(window);
}

/// Who owns side-button presses in this build, handed to the frontend as
/// `__TINE_MOUSE_HISTORY__` before it runs: `gtk` (the GTK widget consumes them),
/// `webview2` (WebView2 delivers DOM events; the subclass takes forwarded copies) or
/// `dom` (no native handler). The build knows this; a user agent can be changed (#948).
pub(crate) const HISTORY_SOURCE: &str = platform::SOURCE;

#[cfg(target_os = "linux")]
mod platform {
    pub(super) const SOURCE: &str = "gtk";
    use super::emit_navigation;
    use std::cell::Cell;

    // Wry's WebKitGTK backend verifies the native GDK mapping and translates
    // these to DOM buttons 3/4 in synthetic_mouse_events.rs.
    const GDK_BUTTON_BACK: u32 = 8;
    const GDK_BUTTON_FORWARD: u32 = 9;

    pub(super) fn install(window: &tauri::WebviewWindow) {
        let event_window = window.clone();
        let _ = window.with_webview(move |platform_webview| {
            use gtk::{
                gdk::{EventButton, EventMask, EventType, FromEvent, ModifierType},
                prelude::{WidgetExt, WidgetExtManual},
            };

            // One slot per side button (Back, Forward): both can be held at once (#948).
            let presses = [Cell::new(None::<(f64, f64)>), Cell::new(None::<(f64, f64)>)];
            let cancel_all = move |presses: &[Cell<Option<(f64, f64)>>; 2]| {
                presses.iter().for_each(|p| p.set(None))
            };
            platform_webview
                .inner()
                .add_events(EventMask::POINTER_MOTION_MASK);
            platform_webview.inner().connect_event(move |_, event| {
                if event.event_type() == EventType::MotionNotify {
                    // No held side button: nothing to cancel, and no event copy (review).
                    if presses.iter().all(|p| p.get().is_none()) {
                        return gtk::glib::Propagation::Proceed;
                    }
                    if let Some((next_x, next_y)) = event.coords() {
                        for press in &presses {
                            if press
                                .get()
                                .is_some_and(|(x, y)| (next_x - x).hypot(next_y - y) > 5.0)
                            {
                                press.set(None);
                            }
                        }
                    }
                    return gtk::glib::Propagation::Proceed;
                }
                if !matches!(
                    event.event_type(),
                    EventType::ButtonPress | EventType::ButtonRelease
                ) {
                    return gtk::glib::Propagation::Proceed;
                }
                let Ok(button_event) = <EventButton as FromEvent>::from(event.clone()) else {
                    return gtk::glib::Propagation::Proceed;
                };
                let (direction, press) = match button_event.button() {
                    GDK_BUTTON_BACK => ("back", &presses[0]),
                    GDK_BUTTON_FORWARD => ("forward", &presses[1]),
                    _ => {
                        // Another button owns a text selection/native drag.
                        cancel_all(&presses);
                        return gtk::glib::Propagation::Proceed;
                    }
                };

                // GtkWidget::event runs before the more-specific
                // button-{press,release}-event signal. Stop both halves here so
                // Wry cannot also synthesize DOM input/default history; navigate
                // once, on release, to preserve normal click semantics.
                if event.event_type() == EventType::ButtonPress {
                    let occupied = button_event.state().intersects(
                        ModifierType::BUTTON1_MASK
                            | ModifierType::BUTTON2_MASK
                            | ModifierType::BUTTON3_MASK,
                    );
                    let (x, y) = button_event.position();
                    press.set((!occupied).then_some((x, y)));
                } else if let Some((x, y)) = press.take() {
                    let (next_x, next_y) = button_event.position();
                    if (next_x - x).hypot(next_y - y) <= 5.0 {
                        emit_navigation(&event_window, direction);
                    }
                }
                gtk::glib::Propagation::Stop
            });
        });
    }
}

#[cfg(target_os = "windows")]
mod platform {
    pub(super) const SOURCE: &str = "webview2";
    use super::emit_navigation;
    use std::ffi::c_void;

    type Hwnd = *mut c_void;
    type SubclassProc =
        Option<unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize>;

    // DefWindowProc can generate WM_APPCOMMAND for an XBUTTON release.
    const WM_NCDESTROY: u32 = 0x0082;
    const WM_APPCOMMAND: u32 = 0x0319;
    const FAPPCOMMAND_MASK: u16 = 0xf000;
    const FAPPCOMMAND_MOUSE: u16 = 0x8000;
    const APPCOMMAND_BROWSER_BACKWARD: u16 = 1;
    const APPCOMMAND_BROWSER_FORWARD: u16 = 2;
    const HISTORY_SUBCLASS_ID: usize = 0x5449_4e45; // "TINE"

    #[link(name = "user32")]
    extern "system" {
        fn IsChild(parent: Hwnd, child: Hwnd) -> i32;
    }

    #[link(name = "comctl32")]
    extern "system" {
        fn SetWindowSubclass(
            hwnd: Hwnd,
            callback: SubclassProc,
            id: usize,
            reference_data: usize,
        ) -> i32;
        fn RemoveWindowSubclass(hwnd: Hwnd, callback: SubclassProc, id: usize) -> i32;
        fn DefSubclassProc(hwnd: Hwnd, message: u32, wparam: usize, lparam: isize) -> isize;
    }

    unsafe extern "system" fn history_subclass_proc(
        hwnd: Hwnd,
        message: u32,
        wparam: usize,
        lparam: isize,
        _id: usize,
        reference_data: usize,
    ) -> isize {
        let window_ptr = reference_data as *mut tauri::WebviewWindow;

        if message == WM_APPCOMMAND && !window_ptr.is_null() {
            // GET_APPCOMMAND_LPARAM: command is the high word with the device
            // bits masked off by ~FAPPCOMMAND_MASK.
            let command_and_device = (lparam as usize >> 16) as u16;
            let direction = match command_and_device & !FAPPCOMMAND_MASK {
                APPCOMMAND_BROWSER_BACKWARD => Some("back"),
                APPCOMMAND_BROWSER_FORWARD => Some("forward"),
                _ => None,
            };
            if let Some(direction) = direction {
                // WebView2 owns mouse presses in its child HWNDs. If Win32 also
                // forwards a mouse command, consume it here before it can reach
                // the router. The host still owns its own mouse commands and
                // standalone Back/Forward commands from keys or OEM devices.
                let from_webview_mouse = command_and_device & FAPPCOMMAND_MASK == FAPPCOMMAND_MOUSE
                    && unsafe { IsChild(hwnd, wparam as Hwnd) } != 0;
                if from_webview_mouse {
                    return 1;
                }
                // SAFETY: reference_data owns a boxed WebviewWindow until
                // WM_NCDESTROY below; the subclass runs on its owning UI thread.
                emit_navigation(unsafe { &*window_ptr }, direction);
                return 1; // TRUE: suppress WebView2/default browser navigation.
            }
        }

        if message == WM_NCDESTROY && !window_ptr.is_null() {
            // Keep the state alive while the remaining subclass chain handles
            // destruction, then release the box exactly once.
            unsafe {
                RemoveWindowSubclass(hwnd, Some(history_subclass_proc), HISTORY_SUBCLASS_ID);
                let result = DefSubclassProc(hwnd, message, wparam, lparam);
                drop(Box::from_raw(window_ptr));
                return result;
            }
        }

        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }

    pub(super) fn install(window: &tauri::WebviewWindow) {
        let Ok(hwnd) = window.hwnd() else {
            return;
        };
        let state = Box::into_raw(Box::new(window.clone()));
        // SAFETY: setup/graph creation invokes this on Tao's UI thread. The box
        // is transferred to the subclass on success and freed at WM_NCDESTROY.
        let installed = unsafe {
            SetWindowSubclass(
                hwnd.0 as Hwnd,
                Some(history_subclass_proc),
                HISTORY_SUBCLASS_ID,
                state as usize,
            )
        };
        if installed == 0 {
            // SAFETY: ownership was not transferred when installation failed.
            unsafe { drop(Box::from_raw(state)) };
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod platform {
    pub(super) const SOURCE: &str = "dom";
    pub(super) fn install(_window: &tauri::WebviewWindow) {}
}
