// UI failure presentation. Callers provide a fixed family and opaque cause;
// this door owns the user text and opt-in diagnostic sink. Work is O(visible
// toasts + error detail length), with no graph read or persistence. Callers
// neither format the cause for display nor inspect debug-log availability.
import { dbg } from "./debug";
import { pushToastUnique } from "./toasts";

export type UiFailureFamily =
  | "external-link"
  | "capture-preference"
  | "audio-load"
  | "audio-play"
  | "clipboard-association"
  | "window-state"
  | "query-hydration"
  | "page-inventory"
  | "session-read"
  | "template-read"
  | "template-write"
  | "journal-feed"
  | "block-counts"
  | "block-resolution"
  | "backup-read";

const MESSAGES: Record<UiFailureFamily, string> = {
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
  "query-hydration": "Couldn't load this query page for editing.",
};

/** Show a fixed message for `family` and log `error` only when debug is enabled.
 * Cost O(visible toasts + detail length). No graph or network work; duplicate
 * visible failures share one toast. Logging failure is reported by `dbg`. */
export function reportUiFailure(family: UiFailureFamily, error: unknown): void {
  dbg(`${family}: ${String(error)}`);
  pushToastUnique(MESSAGES[family], "error");
}
