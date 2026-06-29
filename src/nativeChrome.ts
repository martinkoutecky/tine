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
//   - Linux/Windows: a runtime toggle (default OFF = the custom frameless chrome).
//     ON → ask the OS for a native frame via setDecorations(true) and hide our
//     controls/grips. Persisted so it re-applies at next launch.
//
// Only call initNativeChrome()/setNativeFrame() from the MAIN window (App.tsx) — the
// capture mini-window is deliberately frameless and must not get decorations.

import { createSignal } from "solid-js";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { backend } from "./backend";

const KEY_NATIVE_FRAME = "native_window_frame";

// macOS detection: WKWebView's UA contains "Macintosh"/"Mac OS X". navigator.platform
// is deprecated but a reliable fallback. Evaluated once.
export const isMac: boolean =
  typeof navigator !== "undefined" &&
  (/Mac/i.test(navigator.platform ?? "") || /Mac OS X|Macintosh/i.test(navigator.userAgent ?? ""));

// Linux/Windows user preference: use the OS-native window frame instead of our
// custom frameless chrome.
const [nativeFrame, setNativeFrameSig] = createSignal(false);

/** Reactive: is the OS drawing the window controls (so our custom chrome should
 *  hide)? True on macOS always (the Overlay title bar provides traffic lights), and
 *  on Linux/Windows when the user has turned the native frame on. */
export const osDrawsWindowControls = (): boolean => isMac || nativeFrame();

/** Reactive state of the Linux/Windows native-frame toggle (for the Settings switch).
 *  Meaningless on macOS (where the native frame is always on). */
export const nativeFrameEnabled = nativeFrame;

/** Flip the Linux/Windows native-frame preference: persist it and apply it to the
 *  main window now. No-op on macOS (the frame is fixed at build time there). */
export function setNativeFrame(on: boolean): void {
  if (isMac) return;
  setNativeFrameSig(on);
  void backend().setAppBool(KEY_NATIVE_FRAME, on).catch(() => {});
  void getCurrentWindow().setDecorations(on).catch(() => {});
}

/** Read the persisted preference at startup and apply it. macOS keeps its build-time
 *  Overlay frame (we only sync the signal so the UI hides the custom chrome). */
export async function initNativeChrome(): Promise<void> {
  if (isMac) return; // Overlay frame is fixed in tauri.macos.conf.json
  let on = false;
  try {
    on = await backend().getAppBool(KEY_NATIVE_FRAME, false);
  } catch {
    on = false;
  }
  setNativeFrameSig(on);
  if (on) await getCurrentWindow().setDecorations(true).catch(() => {});
}
