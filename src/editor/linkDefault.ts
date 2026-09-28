// Device-local policy for the `[[…]]` / `#…` completion default action. This is
// deliberately app UI state (tine-settings.json), not graph configuration.
import { createSignal } from "solid-js";
import { backend } from "../backend";
import type { LinkAutocompletePolicy } from "./autocomplete";
import { writePreference, seedPreference } from "../preferenceWrites";
import { pushToast } from "../toasts";

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

/** Apply now and queue the generic string key only. A failed write rolls back
 * and toasts; return does not confirm persistence. O(1) plus backend write. */
export function setLinkAutocompletePolicy(next: LinkAutocompletePolicy): void {
  ++refreshGeneration;
  writePreference(policy, setPolicy, next, (value) => backend().setAppString(POLICY_KEY, value), "link autocomplete policy");
}

/** Refresh this WebView from the device-local string key; Quick Capture refreshes
 * on each show. Missing or invalid values fall back to the legacy boolean
 * (true = existing, otherwise adaptive), possibly writing a migrated string.
 * Only the latest refresh applies. Read errors toast and resolve. O(1) reads
 * plus an optional migration write. */
export async function initLinkDefault(): Promise<void> {
  const generation = ++refreshGeneration;
  const applyIfCurrent = (next: LinkAutocompletePolicy) => {
    if (generation === refreshGeneration) { setPolicy(next); seedPreference(policy); }
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
      pushToast("Could not load legacy link preference.", "error");
    }
    const migrated = migrateLinkAutocompletePolicy(stored, legacy);
    applyIfCurrent(migrated);
    if (legacy !== undefined && generation === refreshGeneration) {
      void backend().setAppString(POLICY_KEY, migrated)
        .catch(() => pushToast("Could not save migrated link autocomplete policy.", "error"));
    }
  } catch {
    applyIfCurrent("adaptive");
    pushToast("Could not load link autocomplete policy.", "error");
  }
}

// Compatibility surface for patch callers and the retained Rust commands. New
// UI code must use the three-mode API above.
export const linkFirstMatch = () => policy() === "existing";
/** Map true to existing, false to adaptive; write generic and legacy keys
 * independently, with failures toasted. */
export function setLinkFirstMatch(on: boolean): void {
  setLinkAutocompletePolicy(on ? "existing" : "adaptive");
  void backend().setLinkFirstMatch(on)
    .catch(() => pushToast("Could not save legacy link preference.", "error"));
}
