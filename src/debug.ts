// Frontend half of the startup debug trace (see main.rs → "Startup debug
// logging"). When the backend reports debug mode on (TINE_DEBUG=1 / --debug), we
// forward the webview's own milestones and uncaught errors into the SAME backend
// log file — so a "the window didn't load" report is captured end-to-end (Rust
// startup + did-the-frontend-boot + any JS error) in one file the user sends back.

import { backend } from "./backend";
import { pushToast, pushToastUnique } from "./toasts";

let enabled = false;

/** Append to the opt-in backend log. Cost O(line length); when disabled it is a
 * no-op, and a write failure disables further attempts and shows one toast. */
export function dbg(line: string): void {
  if (enabled) void backend().debugLog(line).catch(() => {
    enabled = false;
    pushToastUnique("Debug log unavailable.", "error");
  });
}

/** Probe debug mode, and if on: forward errors, log that the frontend booted, and
 *  tell the user where the log lives. Fire-and-forget; never throws. */
export async function initDebug(): Promise<void> {
  let info: { enabled: boolean; path: string };
  try {
    info = await backend().debugInfo();
  } catch {
    return; // browser mock / command missing — nothing to do
  }
  if (!info.enabled) return;
  enabled = true;

  // Capture uncaught errors + promise rejections — the usual "white screen" causes.
  window.addEventListener("error", (e) => {
    dbg(`window.onerror: ${e.message} @ ${e.filename}:${e.lineno}:${e.colno}`);
  });
  window.addEventListener("unhandledrejection", (e) => {
    dbg(`unhandledrejection: ${String((e as PromiseRejectionEvent).reason)}`);
  });

  dbg(`frontend booted (ua=${navigator.userAgent})`);
  pushToast(`Debug logging is ON → ${info.path}`, "info");
}
