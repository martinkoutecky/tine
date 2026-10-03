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

import { createSignal } from "solid-js";
import { backend } from "./backend";

export const KEY_NATIVE_FRAME = "native_window_frame";

declare global {
  // Set by Tauri before frontend code runs. Unlike the saved preference, this
  // describes the decorations actually applied to this process's windows.
  var __TINE_NATIVE_FRAME__: boolean | undefined;
  // Set by Tauri before frontend code runs (src-tauri/src/lib.rs), from the
  // build's own `app_platform()`. See platformKind below.
  var __TINE_PLATFORM__: "android" | "ios" | "desktop" | undefined;
  // E2E-only (src-tauri/src/lib.rs, TINE_E2E_TOUCH_GESTURES): makes the touch
  // gesture layer (edge swipe, block swipe, image viewer) behave as on that
  // OS in a desktop WebKitGTK process. It changes nothing else: layout,
  // keyboard chrome and platform branches still see a desktop.
  var __TINE_E2E_TOUCH_GESTURES__: "android" | "ios" | undefined;
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

/** Android or iOS: a mobile OS that owns the window and drives touch input.
 *  True on iPad too; ask `isSinglePaneShell()` for phone-shaped layout. */
export const isMobilePlatform: boolean = platformKind !== "desktop";

/** Which touch OS the gesture layer (edge swipe, block swipe, image viewer)
 *  should behave as: the real mobile platform, or - only in the native E2E
 *  harness - the platform named by TINE_E2E_TOUCH_GESTURES. Null on desktop.
 *  Read at call time so a test can set the global. O(1). */
export function touchGesturePlatform(): "android" | "ios" | null {
  if (platformKind !== "desktop") return platformKind;
  const forced = typeof globalThis !== "undefined" ? globalThis.__TINE_E2E_TOUCH_GESTURES__ : undefined;
  return forced === "android" || forced === "ios" ? forced : null;
}

/** Stamp the resolved platform on <html> so CSS can ask the same question the
 *  TypeScript does. Styling that depends on touch input — suppressing the
 *  native long-press selection under Tine's own long-press menu, GH #452 —
 *  has no other way to reach it, and must not guess from a media query. */
export function installPlatformAttribute(): void {
  if (typeof document === "undefined") return;
  document.documentElement.setAttribute("data-platform", platformKind);
  const touch = touchGesturePlatform();
  // CSS for the gesture layer (touch-action on block rows) keys on this, which
  // also covers the E2E override that data-platform deliberately ignores.
  if (touch) document.documentElement.setAttribute("data-touch-gestures", touch);
}

// macOS detection: WKWebView's UA contains "Macintosh"/"Mac OS X". navigator.platform
// is deprecated but a reliable fallback. Evaluated once, and only on desktop,
// because iPadOS reports both of those strings too.
export const isMac: boolean =
  platformKind === "desktop" &&
  typeof navigator !== "undefined" &&
  browserPlatform(navigator.userAgent ?? "", navigator.platform ?? "").macDesktop;

/** Tablet-or-larger: the shorter viewport edge has room for two documents
 *  side by side. iPad mini portrait is 744pt across; every iPhone in either
 *  orientation is at most 430pt. Reads the current viewport; O(1). */
export function isTabletViewport(
  width = typeof window === "undefined" ? 0 : window.innerWidth,
  height = typeof window === "undefined" ? 0 : window.innerHeight,
): boolean {
  return Math.min(width, height) >= 700;
}

/** Does this device present the single-pane mobile shell (no split panes,
 *  PDFs open in place, no restored multi-pane layout)? A mobile OS on a
 *  phone-shaped viewport. Split panes need screen room and a precise pointer,
 *  which a tablet has, so an iPad keeps them (GH #446). O(1). */
export function isSinglePaneShell(): boolean {
  return isMobilePlatform && !isTabletViewport();
}

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
