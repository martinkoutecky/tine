// Input capability, asked of the platform's media features and never of the
// device's name (D-20261018-W1: UI follows room and input, not identity).
//
// Both questions are re-asked at call time, so a test (or a window dragged to
// another monitor) sees the current answer, and the first ask registers one
// `change` listener per query so a Solid computation that called them re-runs
// when the answer changes. O(1) per call.
import { createSignal } from "solid-js";
import { mainWindow } from "./windowRealm";

declare global {
  // E2E-only (src-tauri/src/lib.rs, TINE_E2E_TOUCH_GESTURES): makes the touch
  // gesture layer (block swipe, image viewer) behave as if a touch screen
  // were attached to the desktop WebKitGTK process. Nothing else reads it.
  var __TINE_E2E_TOUCH_GESTURES__: "android" | "ios" | undefined;
}

const [revision, bumpRevision] = createSignal(0);
const watched = new Set<string>();

function mediaMatches(query: string): boolean {
  revision();
  if (typeof mainWindow?.matchMedia !== "function") return false;
  const list = mainWindow.matchMedia(query);
  if (!watched.has(query) && typeof list?.addEventListener === "function") {
    watched.add(query);
    list.addEventListener("change", () => bumpRevision((n) => n + 1));
  }
  return !!list?.matches;
}

/** The primary input is a finger (`pointer: coarse`): a phone, a tablet in
 *  its own shell, a touch-only kiosk. Reactive. A touchscreen laptop whose
 *  primary pointer is its trackpad answers false. */
export function touchPrimary(): boolean {
  return mediaMatches("(pointer: coarse)");
}

/** Some attached input is a finger (`any-pointer: coarse`), whatever the
 *  primary pointer is. Reactive. */
export function touchCapable(): boolean {
  return mediaMatches("(any-pointer: coarse)");
}

/** Whether the touch gesture layer (block swipe, image viewer) is installed:
 *  a touch input exists, or - only in the native E2E harness - the journey
 *  forces it on a desktop process. Read at call time. */
export function touchGestures(): boolean {
  if (touchCapable()) return true;
  const forced = typeof globalThis !== "undefined" ? globalThis.__TINE_E2E_TOUCH_GESTURES__ : undefined;
  return forced === "android" || forced === "ios";
}
