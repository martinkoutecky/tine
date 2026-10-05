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
  | "config-read"
  | "conflict-inventory"
  | "graph-verification"
  | "unreadable-files"
  | "asset-read"
  | "page-delete"
  | "page-refresh"
  | "logbook"
  | "marker-cycle"
  | "export-preview"
  | "sheet-export";

const MESSAGES: Record<UiFailureFamily, string> = {
  "custom-css": "Couldn't read custom.css. No custom CSS was applied.",
  "asset-read": "Couldn't read a file from this graph's assets. It may be unreadable.",
  "page-delete": "Couldn't delete the page. It was not removed.",
  "page-refresh": "Couldn't refresh a page after a change. Reopen it to see the latest.",
  "logbook": "Couldn't update the time-tracking entry.",
  "marker-cycle": "Couldn't update the task marker.",
  "export-preview": "Couldn't prepare a block for the export preview. It is shown unresolved.",
  "sheet-export": "Couldn't read this sheet for export.",
  "config-read": "Couldn't read config.edn. The graph is open read-only; repair the config and reopen the graph.",
  "conflict-inventory": "Couldn't refresh conflicts. The last successful inventory is kept.",
  "graph-verification": "Couldn't verify graph files.",
  "unreadable-files": "Some page files couldn't be read. The rest of the graph is available; the file list is in the debug log.",
  "asset-inventory": "Couldn't inspect orphan assets. The last successful scan is kept.",
  "trash-inventory": "Couldn't inspect recoverable trash. The last successful count is kept.",
  "backup-read": "Couldn't complete the launch backup. Your graph is still open.",
  "backup-feedback": "Couldn't receive backup status updates.",
  "page-inventory": "Couldn't refresh the page list. The last loaded list is kept.",
  "session-read": "Couldn't load the saved session. The current workspace is kept.",
  "template-read": "Couldn't load the template list. Try again before creating a template.",
  "template-write": "Couldn't create the template. This block is no longer writable.",
  "journal-feed": "Couldn't load more journals. Try again.",
  "block-counts": "Couldn't refresh block reference counts. The last loaded counts are kept.",
  "block-resolution": "Couldn't resolve block references. Try again.",

  "external-link": "Couldn't open the link.",
  "capture-preference": "Couldn't load the capture setting.",
  "audio-load": "Couldn't load this audio file.",
  "audio-play": "Couldn't play this audio file.",
  "clipboard-association": "Couldn't paste these blocks.",
  "window-state": "Couldn't read the window state.",
  "window-action": "Couldn't change the window.",
  "pdf-find": "Couldn't search this PDF. The search results were cleared.",
  "query-hydration": "Couldn't load this query page for editing.",
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
  if (!unbound) pushToastUnique(MESSAGES[family], "error");
}
