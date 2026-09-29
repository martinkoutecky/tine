// Frontend half of the startup debug trace (see main.rs → "Startup debug
// logging"). When the backend reports debug mode on (TINE_DEBUG=1 / --debug), we
// forward the webview's own milestones and uncaught errors into the SAME backend
// log file — so a "the window didn't load" report is captured end-to-end (Rust
// startup + did-the-frontend-boot + any JS error) in one file the user sends back.
//
// Independently of debug mode, uncaught errors and long main-thread stalls are
// recorded in the privacy-safe flight recorder (GH #343) as a fixed kind plus
// numbers — never the message or file name.

import { backend, type DiagnosticFrontendFields, type DiagnosticFrontendKind } from "./backend";
import { ownedWhen, writeOwned } from "./owned";
import { pushToast, pushToastUnique } from "./toasts";

let enabled = false;
let initialized = false;
let diagnosticsUnavailable = false;

/** A main-thread tick this late (ms) is recorded as a `heartbeat_delay`. */
export const HEARTBEAT_REPORT_MS = 5_000;
const HEARTBEAT_MS = 2_000;

/** Append to the opt-in backend log. Cost O(line length); when disabled it is a
 * no-op, and a write failure disables further attempts and shows one toast. */
export function dbg(line: string): void {
  if (enabled) void backend().debugLog(line).catch(() => {
    enabled = false;
    pushToastUnique("Debug log unavailable.", "error");
  });
}

/** Record one fixed-kind event in the privacy-safe flight recorder (GH #343).
 * Best effort by contract: the returned promise settles once the backend
 * accepted or refused the event and never rejects. A refusal (browser mock
 * without the command, recorder gone) stops further attempts for this run and
 * is noted in the opt-in debug log. O(1) plus one IPC call. */
export function recordDiagnostic(kind: DiagnosticFrontendKind, fields?: DiagnosticFrontendFields): Promise<void> {
  if (diagnosticsUnavailable) return Promise.resolve();
  return writeOwned(ownedWhen(), backend().diagnosticFrontendEvent(kind, fields)).then(
    () => undefined,
    () => {
      diagnosticsUnavailable = true;
      dbg("diagnostic event unrecorded; recorder unavailable");
    },
  );
}

/** Probe debug mode; always install the recorder's error listeners and
 *  heartbeat; if debug mode is on, also forward error text, log that the
 *  frontend booted, and tell the user where the log lives. Idempotent;
 *  fire-and-forget; never throws. */
export async function initDebug(): Promise<void> {
  if (initialized) return;
  initialized = true;
  // The always-on recorder takes only a fixed kind and numeric coordinates;
  // the opt-in trace (dbg) may carry the message and filename.
  window.addEventListener("error", (e) => {
    void recordDiagnostic("uncaught_error", { line: e.lineno || undefined, column: e.colno || undefined });
    dbg(`window.onerror: ${e.message} @ ${e.filename}:${e.lineno}:${e.colno}`);
  });
  window.addEventListener("unhandledrejection", (e) => {
    void recordDiagnostic("unhandled_rejection");
    dbg(`unhandledrejection: ${String((e as PromiseRejectionEvent).reason)}`);
  });
  let expected = performance.now() + HEARTBEAT_MS;
  window.setInterval(() => {
    const now = performance.now();
    const delay = now - expected;
    expected = now + HEARTBEAT_MS;
    if (delay >= HEARTBEAT_REPORT_MS) void recordDiagnostic("heartbeat_delay", { delayMs: Math.round(delay) });
  }, HEARTBEAT_MS);

  let info: { enabled: boolean; path: string };
  try {
    info = await backend().debugInfo();
  } catch {
    return; // browser mock / command missing — nothing to do
  }
  if (!info.enabled) return;
  enabled = true;
  dbg(`frontend booted (ua=${navigator.userAgent})`);
  pushToast(`Debug logging is ON → ${info.path}`, "info");
}

/** Test-only: forget initialization and recorder availability. */
export function resetDebugForTests(): void {
  enabled = false;
  initialized = false;
  diagnosticsUnavailable = false;
}
