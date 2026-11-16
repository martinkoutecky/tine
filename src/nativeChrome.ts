import { browserPlatform } from "./browserPlatform";
// Window-chrome preferences (device-local, persisted in tine-settings.json via the
// generic app_bool backend so native startup can read them before a WebView exists).
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

import { createEffect, createRoot, createSignal } from "solid-js";
import { backend } from "./backend";
import { touchGestures } from "./inputCapability";
import { advanceRevision, currentRevision, ownedWhen, readOwned, revisionOwner, serializeDurable } from "./owned";

export const KEY_NATIVE_FRAME = "native_window_frame";

declare global {
  // Set by Tauri before frontend code runs. Unlike the saved preference, this
  // describes the decorations actually applied to this process's windows.
  var __TINE_NATIVE_FRAME__: boolean | undefined;
  // Set by Tauri before frontend code runs (src-tauri/src/lib.rs), from the
  // build's own `app_platform()`. See platformKind below.
  var __TINE_PLATFORM__: "android" | "ios" | "desktop" | undefined;
}

export type PlatformKind = "android" | "ios" | "desktop";

// The user agent cannot answer "what am I running on". iPadOS 13+ serves a
// desktop-class `Macintosh; Intel Mac OS X` UA from a stock WKWebView, so an
// iPad both FAILED the mobile test and PASSED the Mac test and rendered as a
// Mac desktop throughout (GH #446: no editing toolbar on iPad).
//
// So the build tells us instead. Rust injects `__TINE_PLATFORM__` from the same
// `app_platform()` that answers the async backend command (platform.ts), before
// any frontend code runs, which keeps this synchronous: a one-frame async
// round-trip would flash desktop-only chrome.
//
// The UA branch is the fallback for contexts Tauri never initializes: the
// browser dev server, the mock backend and unit tests.
function detectPlatformKind(): PlatformKind {
  const injected = typeof globalThis !== "undefined" ? globalThis.__TINE_PLATFORM__ : undefined;
  if (injected === "android" || injected === "ios" || injected === "desktop") return injected;
  const ua = typeof navigator !== "undefined" ? (navigator.userAgent ?? "") : "";
  const hints = browserPlatform(ua);
  if (hints.android) return "android";
  if (hints.ios) return "ios";
  return "desktop";
}

/** What the app runs on, decided by the build (UA only outside Tauri).
 *  Evaluated once at module load; O(1). */
export const platformKind: PlatformKind = detectPlatformKind();

/** Android or iOS: a mobile OS that owns the window. Identity, not layout and
 *  not input: UI features ask `viewportClass.ts` (room) and `inputCapability.ts`
 *  (pointer) instead (D-20261018-W1). The importers that remain are listed with
 *  a reason in `src/PLATFORM_IDENTITY.txt`, pinned by platformIdentity.guard.test.ts. */
export const isMobilePlatform: boolean = platformKind !== "desktop";

/** Whether this build can show a folder in the system file manager. The native
 *  `reveal_*` commands are compiled for the desktop targets (Linux, Windows,
 *  macOS: `cfg(desktop)`, the predicate the injected platform reports) and
 *  refuse on iOS and Android, which have no file manager to hand a folder to;
 *  callers hide the action there instead of offering one that can only fail. */
export function canRevealInFileManager(): boolean {
  return platformKind === "desktop";
}

/** Which OS's back-gesture model the edge swipe follows (iOS edge-swipe Back,
 *  Android SafeBack): the real mobile platform, or - only in the native E2E
 *  harness - the platform named by TINE_E2E_TOUCH_GESTURES. Null on desktop.
 *  Whether touch gestures exist at all is `touchGestures()`
 *  (inputCapability.ts). Read at call time so a test can set the global. O(1). */
export function touchGesturePlatform(): "android" | "ios" | null {
  if (platformKind !== "desktop") return platformKind;
  const forced = typeof globalThis !== "undefined" ? globalThis.__TINE_E2E_TOUCH_GESTURES__ : undefined;
  return forced === "android" || forced === "ios" ? forced : null;
}

/** Stamp the touch-gesture layer on <html> so CSS (touch-action on block
 *  rows, swipe feedback) can key on it. The value is the OS whose back-gesture
 *  model applies, or "touch" on a desktop-class device with a touch screen.
 *  CSS never asks which platform this is (D-20261018-W1): the former
 *  `data-platform` attribute is gone. The stamp follows the input capability
 *  (set when a touch input appears, removed when it goes away), so CSS and the
 *  swipe wiring agree. Returns the stop function. */
export function installPlatformAttribute(): () => void {
  if (typeof document === "undefined") return () => {};
  const root = document.documentElement;
  return createRoot((dispose) => {
    createEffect(() => {
      if (touchGestures()) root.setAttribute("data-touch-gestures", touchGesturePlatform() ?? "touch");
      else root.removeAttribute("data-touch-gestures");
    });
    return dispose;
  });
}

// macOS detection: WKWebView's UA contains "Macintosh"/"Mac OS X". navigator.platform
// is deprecated but a reliable fallback. Evaluated once, and only on desktop,
// because iPadOS reports both of those strings too.
export const isMac: boolean =
  platformKind === "desktop" &&
  typeof navigator !== "undefined" &&
  browserPlatform(navigator.userAgent ?? "", navigator.platform ?? "").macDesktop;

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

// One revision/queue key for the device-local preference: writes run in call
// order, and a startup read that lands after a newer choice never overwrites it
// (I-20/I-21).
const nativeFramePref = {};

/** Persist the Linux/Windows native-frame preference. It takes effect at the next
 *  normal app start, when Rust can construct all graph windows consistently.
 *  Writes are ordered; the switch shows the newest choice only once it is on
 *  disk, and a write failure rejects (Settings reports it). O(1). */
export async function setNativeFrame(on: boolean): Promise<void> {
  if (isMac) return;
  const revision = advanceRevision(nativeFramePref);
  await serializeDurable(nativeFramePref, ownedWhen(), () => backend().setAppBool(KEY_NATIVE_FRAME, on));
  if (currentRevision(nativeFramePref) === revision) setNativeFramePreferenceSig(on);
}

/** Read the saved preference for the Settings switch. Rust already applied the
 *  startup value before constructing this window. A choice made while the read
 *  was pending wins over the read. */
export async function initNativeChrome(): Promise<void> {
  if (isMac) return; // Overlay frame is fixed in tauri.macos.conf.json
  const owner = revisionOwner(nativeFramePref, currentRevision(nativeFramePref));
  let on = startupNativeFrame;
  try {
    const read = await readOwned(owner, backend().getAppBool(KEY_NATIVE_FRAME, startupNativeFrame));
    if (read.kind === "stale") return;
    on = read.value;
  } catch {
    // The applied frame is the truth for this window; an unreadable saved
    // preference only affects the restart toggle, which shows the applied state.
    on = startupNativeFrame;
  }
  if (owner()) setNativeFramePreferenceSig(on);
}
