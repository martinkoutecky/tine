// Default action for the `[[…]]` / `#…` autocomplete when the typed text is
// neither blank nor an exact existing page name.
//
// - OFF (default — matches OG): the "Create <typed>" item leads the list, so
//   Enter makes a NEW page/tag unless you arrow down to a match.
// - ON: the first matched page leads, so Enter LINKS to it; "Create" moves to
//   the end of the list (still one arrow-down away).
//
// App-level, device-local preference (tine-settings.json) — a workflow choice,
// not graph data — mirroring the smooth-scroll / capture-enter settings.
import { createSignal } from "solid-js";
import { backend } from "../backend";

const [enabled, setEnabled] = createSignal(false);
/** Reactive: does the autocomplete default to LINKING the first match? */
export const linkFirstMatch = enabled;

/** Toggle from the UI: persist the choice and apply it live. */
export function setLinkFirstMatch(on: boolean): void {
  setEnabled(on);
  void backend().setLinkFirstMatch(on).catch(() => {});
}

/** Read the persisted preference at startup. Default OFF (OG behavior). */
export async function initLinkDefault(): Promise<void> {
  try {
    setEnabled(await backend().getLinkFirstMatch());
  } catch {
    /* default off */
  }
}
