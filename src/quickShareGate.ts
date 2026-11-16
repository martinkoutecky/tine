import { createSignal } from "solid-js";
import { backend } from "./backend";
import { dbg } from "./debug";

// Whether this build offers quick share links. The answer is a property of the
// build (quick_share::enabled, the viewer attestation), not of the graph, so
// one read serves the whole session. Graph load primes it, so Settings opens
// with its tab list final: a Sharing tab inserted after the dialog painted
// pushed every tab below it down one row, under a click aimed at one of them
// (UI-SETTINGS-SHARING-TAB-SHIFT).
const [offered, setOffered] = createSignal<boolean | undefined>(undefined);
let pending: Promise<void> | undefined;

export function primeQuickShareGate(): Promise<void> {
  // Deferred a microtask, so a backend without the call rejects instead of throwing into graph load.
  pending ??= Promise.resolve().then(() => backend().quickShareList())
    .then((listing) => { setOffered(listing.enabled); })
    .catch((error) => {
      pending = undefined;
      dbg(`quick share gate: reading it failed: ${String(error)}`);
    });
  return pending;
}

/** True once this build is known to offer quick share; false while unknown. */
export const quickShareOffered = () => offered() === true;

/** Test hook: forget the session's answer. */
export function resetQuickShareGateForTest(): void {
  pending = undefined;
  setOffered(undefined);
}
