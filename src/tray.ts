// Desktop system tray settings (GH #625). Three device-local preferences, all
// OFF by default, stored in tine-settings.json through the generic app_bool
// door (like `native_window_frame` in nativeChrome.ts) so native startup can
// read them before any webview exists. The native side (src-tauri/src/tray.rs)
// owns what they DO; this module only names the keys and answers which
// controls are available. Absent on mobile: the native `tray_apply` reports
// `supported: false` there and the Settings section renders nothing.
import type { TrayStatus } from "./backendTypes";

export const KEY_TRAY_SHOW = "tray_show";
export const KEY_TRAY_MINIMIZE = "tray_minimize";
export const KEY_TRAY_START_MINIMIZED = "tray_start_minimized";

/** "Minimize to tray" and "Start minimized to tray" are enabled only while
 *  "Show Tine in the system tray" is on. The stored values are kept when the
 *  icon setting is turned off (the native side ignores them then). O(1). */
export function trayDependentControlsEnabled(show: boolean): boolean {
  return show;
}

/** The one-line Settings note: shown only when the icon was requested and the
 *  desktop could not show it. O(1). */
export function trayNote(show: boolean, status: TrayStatus | null): string | null {
  return show && status?.supported && status.problem ? status.problem : null;
}
