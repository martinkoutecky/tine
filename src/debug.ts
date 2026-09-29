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
import { platformKind } from "./nativeChrome";
import { ownedWhen, writeOwned } from "./owned";
import { recordErrorToastText } from "./errorToastHistory";
import { pushToast, pushToastUnique, recordErrorToastsWith } from "./toasts";

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
  // A backend without the command throws synchronously; that is the same
  // "recorder unavailable" refusal, and must not escape into pushToast.
  let accepted: Promise<void>;
  try { accepted = backend().diagnosticFrontendEvent(kind, fields); } catch (error) { accepted = Promise.reject(error); }
  return writeOwned(ownedWhen(), accepted).then(
    () => undefined,
    () => {
      diagnosticsUnavailable = true;
      dbg("diagnostic event unrecorded; recorder unavailable");
    },
  );
}

/** Tell the recorder whether this mobile session is live (GH #426; see
 * sessionActivity.ts). Best effort like `recordDiagnostic`: never rejects; a
 * refusal is noted in the opt-in debug log. O(1) plus one IPC call. */
export function recordSessionActive(active: boolean): Promise<void> {
  return writeOwned(ownedWhen(), backend().diagnosticSessionActive(active)).then(
    () => undefined,
    () => dbg("diagnostic session edge unrecorded"),
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
  // Every error toast: its occurrence in the persisted recorder (fixed kind,
  // no text: it may name pages), its full text in the opt-in debug log, and its
  // text in the in-memory session list Diagnostics shows (never persisted).
  recordErrorToastsWith((message) => {
    recordErrorToastText(message);
    void recordDiagnostic("error_toast");
    dbg(`error toast: ${message}`);
  });
  let expected = performance.now() + HEARTBEAT_MS;
  window.setInterval(() => {
    const now = performance.now();
    const delay = now - expected;
    expected = now + HEARTBEAT_MS;
    if (delay >= HEARTBEAT_REPORT_MS) void recordDiagnostic("heartbeat_delay", { delayMs: Math.round(delay) });
  }, HEARTBEAT_MS);

  let info: { enabled: boolean; path: string; previousExitUnclean?: boolean };
  try {
    info = await backend().debugInfo();
  } catch {
    return; // browser mock / command missing — nothing to do
  }
  // The persisted recorder found the previous run's session marker: it ended
  // without an orderly exit (og ADR 0058). Its events are in the report.
  if (info.previousExitUnclean) {
    pushToast("Tine did not close cleanly last time. A privacy-safe diagnostic report is available.", "warn", {
      sticky: true,
      // ui.ts imports this module's dependents; load it only when clicked.
      action: { label: "Diagnostics", run: () => void import("./ui").then((ui) => ui.openSettings("diagnostics")) },
    });
  }
  if (!info.enabled) return;
  enabled = true;
  // platform= is the identity Rust injected, which is NOT derivable from ua=
  // on iPadOS (GH #446). Keeping both in one line makes that divergence
  // readable in a bug report.
  dbg(`frontend booted (platform=${platformKind} ua=${navigator.userAgent})`);
  pushToast(`Debug logging is ON → ${info.path}`, "info");
}

/** Test-only: forget initialization and recorder availability. */
export function resetDebugForTests(): void {
  enabled = false;
  initialized = false;
  diagnosticsUnavailable = false;
}
