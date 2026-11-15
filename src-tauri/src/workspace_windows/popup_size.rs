//! One device-local size for fresh workspace popups (D-20261114-PG).
//! Read once on first use; resize patches two numbers in memory, close/exit
//! commits changed numbers through the guarded device-settings writer. No
//! graph work, per-window durable records, positions or native size setters.

use std::sync::{Mutex, OnceLock};
use tauri::Manager;

use super::{place, Area, Geometry, Placement};
#[cfg(desktop)]
use super::{MAX_SIDE, MIN_SIZE};

const KEY: &str = "workspace_popup_size";
static LAST: OnceLock<Mutex<Remembered>> = OnceLock::new();

struct Remembered {
    size: Option<[f64; 2]>,
    saved: Option<[f64; 2]>,
}

impl Remembered {
    fn new(size: Option<[f64; 2]>) -> Self {
        Self { size, saved: size }
    }

    fn flush(&mut self, save: impl FnOnce([f64; 2]) -> Result<(), String>) -> Result<(), String> {
        if self.size != self.saved {
            if let Some(size) = self.size {
                save(size)?;
                self.saved = Some(size);
            }
        }
        Ok(())
    }
}

fn remembered<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> &'static Mutex<Remembered> {
    LAST.get_or_init(|| {
        let size = app
            .path()
            .app_data_dir()
            .ok()
            .and_then(|dir| load_at(&crate::settings::settings_path_in(&dir)));
        Mutex::new(Remembered::new(size))
    })
}

fn load_at(path: &std::path::Path) -> Option<[f64; 2]> {
    crate::settings::read_settings_value_at(path)
        .and_then(|json| serde_json::from_value(json[KEY].clone()).ok())
}

fn save_at(path: &std::path::Path, size: [f64; 2]) -> Result<(), String> {
    crate::settings::update_settings_at(path, |json| {
        json[KEY] = serde_json::json!(size);
    })
}

/// Validate logical dimensions with the existing rectangle policy, then fit
/// the current work area. O(1), no I/O, always position-free and infallible.
pub(super) fn fresh_placement(size: Option<[f64; 2]>, area: Option<Area>) -> Placement {
    let mut placement = place(
        size.map(|[width, height]| Geometry {
            x: 0.0,
            y: 0.0,
            width,
            height,
        }),
        &[],
    );
    if let Some(area) = area {
        placement.width = placement.width.min(area.width);
        placement.height = placement.height.min(area.height);
    }
    placement.position = None;
    placement
}

/// Fresh-window dimensions, clamped to the opener's current monitor work area.
/// O(1): first use reads device settings once; each call queries one monitor.
/// Missing/unusable settings use the existing fallback; no monitor leaves the
/// validated dimensions unclamped. Session-restored rectangles bypass this.
pub(super) fn placement(window: &tauri::Window) -> Placement {
    let size = remembered(window.app_handle()).lock().unwrap().size;
    let area = window.current_monitor().ok().flatten().map(|monitor| {
        let scale = monitor.scale_factor();
        let work = monitor.work_area();
        let position = work.position.to_logical::<f64>(scale);
        let size = work.size.to_logical::<f64>(scale);
        Area {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        }
    });
    fresh_placement(size, area)
}

fn resized_logical_size(
    previous: &mut [f64; 2],
    physical: tauri::PhysicalSize<u32>,
    scale: f64,
) -> Option<[f64; 2]> {
    if tauri::LogicalSize::new(previous[0], previous[1]).to_physical::<u32>(scale) == physical {
        return None;
    }
    let logical = physical.to_logical::<f64>(scale);
    let size = [logical.width, logical.height];
    *previous = size;
    Some(size)
}

/// Observe a built popup, outside plugin creation hooks. Initial geometry is
/// the baseline, so opening/restoring a popup does not count as a user resize.
/// Like electron-window-state 5.0.3, retain normal bounds in memory on resize
/// and persist on close; unlike it, save only dimensions and use guarded I/O.
/// O(1) per event, fixed process memory plus one size pair per live popup.
#[cfg(desktop)]
pub(super) fn track<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>, placement: Placement) {
    let previous = Mutex::new([placement.width, placement.height]);
    let observed = window.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Resized(size) => {
            let Ok(scale) = observed.scale_factor() else {
                return;
            };
            let Some(size) = resized_logical_size(&mut previous.lock().unwrap(), *size, scale)
            else {
                return;
            };
            if observed.is_maximized().ok() != Some(false)
                || observed.is_minimized().ok() != Some(false)
                || observed.is_fullscreen().ok() != Some(false)
            {
                return;
            }
            if size[0] >= MIN_SIZE.0
                && size[1] >= MIN_SIZE.1
                && size[0] <= MAX_SIDE
                && size[1] <= MAX_SIDE
            {
                remembered(observed.app_handle()).lock().unwrap().size = Some(size);
            }
        }
        tauri::WindowEvent::ScaleFactorChanged {
            new_inner_size,
            scale_factor,
            ..
        } => {
            let size = new_inner_size.to_logical::<f64>(*scale_factor);
            *previous.lock().unwrap() = [size.width, size.height];
        }
        tauri::WindowEvent::Destroyed => flush(observed.app_handle()),
        _ => {}
    });
}

/// Commit a changed shared size at popup destruction or orderly app exit.
/// O(device-settings bytes), one guarded update, no native window calls.
/// Failure retains the dirty size for a later retry and records the error.
pub(crate) fn flush<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let Some(last) = LAST.get() else { return };
    let result = last.lock().unwrap().flush(|size| {
        let dir = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?;
        save_at(&crate::settings::settings_path_in(&dir), size)
    });
    if let Err(error) = result {
        crate::debug::diag_private("workspace-popup-size-save-failed", error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_geometry_rounding_on_another_scale_is_not_a_resize() {
        let requested = [639.2, 540.2];
        let mut previous = requested;
        let created = tauri::LogicalSize::new(requested[0], requested[1]).to_physical::<u32>(1.5);
        assert_eq!(resized_logical_size(&mut previous, created, 1.5), None);
        assert_eq!(previous, requested);
        assert_eq!(
            resized_logical_size(&mut previous, tauri::PhysicalSize::new(1200, 900), 1.5),
            Some([800.0, 600.0])
        );
        assert_eq!(
            resized_logical_size(&mut previous, tauri::PhysicalSize::new(1200, 900), 1.5),
            None
        );
    }

    #[test]
    fn native_geometry_is_set_only_on_the_builder() {
        for source in [
            include_str!("../workspace_windows.rs"),
            include_str!("popup_size.rs"),
        ] {
            let production = source.split("#[cfg(test)]\nmod tests").next().unwrap();
            assert!(!production.contains(".set_size("));
            assert!(!production.contains(".set_position("));
            assert!(!production.contains(".restore_state("));
        }
    }

    #[test]
    fn closed_popup_size_survives_reload_without_changing_other_device_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::settings::settings_path_in(dir.path());
        assert_eq!(load_at(&path), None);
        crate::settings::update_settings_at(&path, |json| {
            json["unrelated"] = serde_json::json!(42)
        })
        .unwrap();
        let mut last = Remembered::new(load_at(&path));
        last.size = Some([800.0, 600.0]);
        last.flush(|size| save_at(&path, size)).unwrap();
        let reloaded = fresh_placement(load_at(&path), None);
        assert_eq!((reloaded.width, reloaded.height), (800.0, 600.0));
        assert_eq!(reloaded.position, None);
        assert_eq!(
            crate::settings::read_settings_value_at(&path).unwrap()["unrelated"],
            42
        );
    }

    #[test]
    fn resize_is_memory_only_and_close_writes_once_per_change_and_retries_errors() {
        let mut last = Remembered::new(Some([800.0, 600.0]));
        let mut writes = 0;
        last.flush(|_| {
            writes += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(writes, 0);
        let mut previous = [800.0, 600.0];
        for width in 801..900 {
            if let Some(size) =
                resized_logical_size(&mut previous, tauri::PhysicalSize::new(width, 600), 1.0)
            {
                last.size = Some(size);
            }
        }
        assert_eq!(writes, 0);
        assert!(last.flush(|_| Err("disk failure".into())).is_err());
        assert_ne!(last.saved, last.size);
        last.flush(|size| {
            writes += 1;
            assert_eq!(size, [899.0, 600.0]);
            Ok(())
        })
        .unwrap();
        last.flush(|_| {
            writes += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(writes, 1);
    }
}
