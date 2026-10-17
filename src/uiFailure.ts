import { msg, type Msg } from "./i18n";
// UI failure presentation. Callers provide a fixed family and opaque cause;
// this door owns the user text and opt-in diagnostic sink. Work is O(visible
// toasts + error detail length), with no graph read or persistence. Callers
// neither format the cause for display nor inspect debug-log availability.
import { captureBinding } from "./binding";
import { dbg } from "./debug";
import { pushToastUnique } from "./toasts";

export type UiFailureFamily =
  | "external-link"
  | "capture-preference"
  | "audio-load"
  | "audio-play"
  | "clipboard-association"
  | "window-state"
  | "window-action"
  | "pdf-find"
  | "query-hydration"
  | "page-inventory"
  | "session-read"
  | "template-read"
  | "template-write"
  | "journal-feed"
  | "block-counts"
  | "block-resolution"
  | "backup-read"
  | "backup-feedback"
  | "asset-inventory"
  | "trash-inventory"
  | "custom-css"
  | "css-draft"
  | "config-read"
  | "conflict-inventory"
  | "graph-verification"
  | "unreadable-files"
  | "asset-read"
  | "page-refresh"
  | "logbook"
  | "marker-read"
  | "page-header-read"
  | "export-preview"
  | "sheet-export";

const MESSAGES: Record<UiFailureFamily, Msg> = {
  "custom-css": msg("ui-failure-custom-css"),
  "css-draft": msg("ui-failure-css-draft"),
  "asset-read": msg("ui-failure-asset-read"),
  "page-refresh": msg("ui-failure-page-refresh"),
  "logbook": msg("ui-failure-logbook"),
  "marker-read": msg("ui-failure-marker-read"),
  "page-header-read": msg("ui-failure-page-header-read"),
  "export-preview": msg("ui-failure-export-preview"),
  "sheet-export": msg("ui-failure-sheet-export"),
  "config-read": msg("ui-failure-config-read"),
  "conflict-inventory": msg("ui-failure-conflict-inventory"),
  "graph-verification": msg("ui-failure-graph-verification"),
  "unreadable-files": msg("ui-failure-unreadable-files"),
  "asset-inventory": msg("ui-failure-asset-inventory"),
  "trash-inventory": msg("ui-failure-trash-inventory"),
  "backup-read": msg("ui-failure-backup-read"),
  "backup-feedback": msg("ui-failure-backup-feedback"),
  "page-inventory": msg("ui-failure-page-inventory"),
  "session-read": msg("ui-failure-session-read"),
  "template-read": msg("ui-failure-template-read"),
  "template-write": msg("ui-failure-template-write"),
  "journal-feed": msg("ui-failure-journal-feed"),
  "block-counts": msg("ui-failure-block-counts"),
  "block-resolution": msg("ui-failure-block-resolution"),

  "external-link": msg("ui-failure-external-link"),
  "capture-preference": msg("ui-failure-capture-preference"),
  "audio-load": msg("ui-failure-audio-load"),
  "audio-play": msg("ui-failure-audio-play"),
  "clipboard-association": msg("ui-failure-clipboard-association"),
  "window-state": msg("ui-failure-window-state"),
  "window-action": msg("ui-failure-window-action"),
  "pdf-find": msg("ui-failure-pdf-find"),
  "query-hydration": msg("ui-failure-query-hydration"),
};

/** Families whose cause is a read of the window's bound graph. */
const GRAPH_READS: ReadonlySet<UiFailureFamily> = new Set<UiFailureFamily>([
  "query-hydration", "page-inventory", "session-read", "template-read", "journal-feed",
  "block-counts", "block-resolution", "backup-read", "asset-inventory", "trash-inventory",
  "custom-css", "config-read", "conflict-inventory", "graph-verification", "unreadable-files",
  "asset-read", "page-refresh", "export-preview", "sheet-export",
]);

/** Show a fixed message for `family` and log `error` only when debug is enabled.
 * A graph read in a window with no graph binding (launch load still running,
 * Welcome screen, Quick Capture, a switch in progress) was refused because
 * there is no graph yet: a transient state, not a failure, so it is logged and
 * raises no toast (OG-TOAST; Martin 2026-09-29). A read issued before a bind
 * that lands meanwhile is already dropped by its retired owner. Cost O(visible
 * toasts + detail length). No graph or network work; duplicate visible failures
 * share one toast. Logging failure is reported by `dbg`. */
function detail(error: unknown): string {
  if (error instanceof Error || typeof error !== "object" || error === null) return String(error);
  try { return JSON.stringify(error); } catch { return String(error); }
}

export function reportUiFailure(family: UiFailureFamily, error: unknown): void {
  const unbound = GRAPH_READS.has(family) && captureBinding().backendGeneration === 0;
  dbg(`${family}${unbound ? " (window unbound; not reported)" : ""}: ${detail(error)}`);
  if (!unbound) pushToastUnique(msg(MESSAGES[family].id), "error");
}
