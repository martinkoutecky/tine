// Window-chrome preferences (device-local, persisted in tine-settings.json via the
// generic app_bool backend — WebKitGTK localStorage doesn't survive a restart).
//
// Tine's main window is frameless by default (`decorations: false`) so the toolbar
// doubles as the title bar and we save a row — see WindowChrome.tsx. That custom
// chrome reads as alien on macOS (square corners, no traffic lights — GitHub #3), so:
//
//   - macOS: a build-time override (tauri.macos.conf.json) gives the main window
//     `titleBarStyle: "Overlay"` + `hiddenTitle` → native rounded corners + traffic
//     lights, with the content still rising under the transparent title bar (the
//     compact layout is kept). The OS draws the controls, so our custom
//     WindowControls/ResizeGrips are never shown on macOS. Nothing here toggles it;
//     `isMac` just tells the UI to hide custom chrome + reserve the traffic-light gap.
//
//   - Linux/Windows: a restart-time toggle (default OFF = the custom frameless
//     chrome). Tao cannot reliably change GTK decorations on an existing window,
//     so Rust applies the preference while constructing every graph window.
//
// The capture mini-window is deliberately frameless and ignores this preference.

import { createSignal } from "solid-js";
import { backend } from "./backend";

export const KEY_NATIVE_FRAME = "native_window_frame";

declare global {
  // Set by Tauri before frontend code runs. Unlike the saved preference, this
  // describes the decorations actually applied to this process's windows.
  var __TINE_NATIVE_FRAME__: boolean | undefined;
}

// macOS detection: WKWebView's UA contains "Macintosh"/"Mac OS X". navigator.platform
// is deprecated but a reliable fallback. Evaluated once.
export const isMac: boolean =
  typeof navigator !== "undefined" &&
  (/Mac/i.test(navigator.platform ?? "") || /Mac OS X|Macintosh/i.test(navigator.userAgent ?? ""));

// Mobile (Android/iOS) detection: the Tauri WebView's UA carries the platform.
// Evaluated once, same idiom as isMac above. The backend `app_platform` command
// is the authoritative source for file-system logic (see platform.ts / graph.ts);
// this synchronous UA constant is the instant client-side gate for layout/chrome,
// where a one-frame async round-trip would flash desktop-only controls.
export const isMobilePlatform: boolean =
  typeof navigator !== "undefined" && /Android|iPhone|iPad|iPod/i.test(navigator.userAgent ?? "");

// Linux/Windows user preference: use the OS-native window frame instead of our
// custom frameless chrome.
const startupNativeFrame = typeof globalThis !== "undefined" && globalThis.__TINE_NATIVE_FRAME__ === true;
const [nativeFrameActive] = createSignal(startupNativeFrame);
const [nativeFramePreference, setNativeFramePreferenceSig] = createSignal(startupNativeFrame);

/** Reactive: is the OS drawing the window controls (so our custom chrome should
 *  hide)? True on macOS always (the Overlay title bar provides traffic lights),
 *  on Linux/Windows when the user has turned the native frame on, and always on
 *  mobile (Android/iOS have no in-app min/max/close — the OS owns the window). */
export const osDrawsWindowControls = (): boolean =>
  isMac || nativeFrameActive() || isMobilePlatform;

/** Reactive state of the Linux/Windows native-frame toggle (for the Settings switch).
 *  Meaningless on macOS (where the native frame is always on). */
export const nativeFrameEnabled = nativeFramePreference;

/** Persist the Linux/Windows native-frame preference. It takes effect at the next
 *  normal app start, when Rust can construct all graph windows consistently. */
export async function setNativeFrame(on: boolean): Promise<void> {
  if (isMac) return;
  await backend().setAppBool(KEY_NATIVE_FRAME, on);
  setNativeFramePreferenceSig(on);
}

/** Read the saved preference for the Settings switch. Rust already applied the
 *  startup value before constructing this window. */
export async function initNativeChrome(): Promise<void> {
  if (isMac) return; // Overlay frame is fixed in tauri.macos.conf.json
  let on = startupNativeFrame;
  try {
    on = await backend().getAppBool(KEY_NATIVE_FRAME, startupNativeFrame);
  } catch {
    on = startupNativeFrame;
  }
  setNativeFramePreferenceSig(on);
}
