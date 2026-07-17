// Device-local policy for the `[[…]]` / `#…` completion default action. This is
// deliberately app UI state (tine-settings.json), not graph configuration.
import { createSignal } from "solid-js";
import { backend } from "../backend";
import type { LinkAutocompletePolicy } from "./autocomplete";

export type { LinkAutocompletePolicy } from "./autocomplete";

const POLICY_KEY = "link_autocomplete_policy";
const validPolicies = new Set<LinkAutocompletePolicy>(["adaptive", "existing", "typed"]);
const [policy, setPolicy] = createSignal<LinkAutocompletePolicy>("adaptive");
// `initLinkDefault` is called again whenever persistent Quick Capture is shown.
// Reads can overlap, so only the most recently started refresh may mutate this
// WebView's shared signal. A direct Settings update also invalidates older reads.
let refreshGeneration = 0;

export const linkAutocompletePolicy = policy;

/** Pure, restart-stable migration for the former boolean preference. A legacy
 * true meant prefer an existing match; false/missing was historically called
 * "OG" but now maps to the actual OG adaptive behavior. */
export function migrateLinkAutocompletePolicy(value: unknown, legacy?: boolean | null): LinkAutocompletePolicy {
  if (typeof value === "string" && validPolicies.has(value as LinkAutocompletePolicy)) {
    return value as LinkAutocompletePolicy;
  }
  return legacy === true ? "existing" : "adaptive";
}

/** Apply a new setting live and persist only the generic string key. */
export function setLinkAutocompletePolicy(next: LinkAutocompletePolicy): void {
  ++refreshGeneration;
  setPolicy(next);
  void backend().setAppString(POLICY_KEY, next).catch(() => {});
}

/** Read the string policy in each WebView. Quick Capture is an independent
 * WebView, and calls this again every time its persistent window is shown. */
export async function initLinkDefault(): Promise<void> {
  const generation = ++refreshGeneration;
  const applyIfCurrent = (next: LinkAutocompletePolicy) => {
    if (generation === refreshGeneration) setPolicy(next);
  };
  try {
    const stored = await backend().getAppString(POLICY_KEY, "");
    if (validPolicies.has(stored as LinkAutocompletePolicy)) {
      applyIfCurrent(stored as LinkAutocompletePolicy);
      return;
    }
    let legacy: boolean | undefined;
    try {
      legacy = await backend().getLinkFirstMatch();
    } catch {
      // Backend/read failure remains the safe current default.
    }
    const migrated = migrateLinkAutocompletePolicy(stored, legacy);
    applyIfCurrent(migrated);
    if (legacy !== undefined && generation === refreshGeneration) {
      void backend().setAppString(POLICY_KEY, migrated).catch(() => {});
    }
  } catch {
    applyIfCurrent("adaptive");
  }
}

// Compatibility surface for patch callers and the retained Rust commands. New
// UI code must use the three-mode API above.
export const linkFirstMatch = () => policy() === "existing";
export function setLinkFirstMatch(on: boolean): void {
  setLinkAutocompletePolicy(on ? "existing" : "adaptive");
  void backend().setLinkFirstMatch(on).catch(() => {});
}
