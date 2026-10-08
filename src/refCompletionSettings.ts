// Device-local editor preference: after accepting a page/block-reference completion,
// insert a space after the closing `]]`/`))` so typing continues cleanly. Persisted
// in tine-settings.json via the app_bool backend so independent WebViews consume the
// same value. Read once at startup by initRefCompletionSettings().
//
// DIFFERS from Logseq by default: OG (and file-based Logseq) leave the caret right
// after the closing brackets with no space (verified in og handler/page.cljs →
// commands/insert!). Tine defaults to adding the space (nicer writing flow, GH #35);
// toggle OFF in Settings → Editor ("Match Logseq") to restore the OG behavior.

import { createSignal } from "solid-js";
import { backend } from "./backend";
import { writePreference, seedPreference, preferenceRevision, preferenceReadCurrent } from "./preferenceWrites";
import { pushToast } from "./toasts";

const KEY = "space_after_ref_completion";

const [spaceAfter, setSpaceAfterSig] = createSignal(true);

/** Reactive: insert a space after `]]`/`))` when accepting a page/block-ref
 *  completion? ON = Tine default (GH #35); OFF = Logseq (caret right after the
 *  closing brackets, no space). */
export const spaceAfterRefCompletion = spaceAfter;

/** Apply now and queue a device-local write. Failure rolls back and toasts;
 * return does not confirm persistence. O(1) plus backend write. */
export function setSpaceAfterRefCompletion(on: boolean): void {
  writePreference(spaceAfter, setSpaceAfterSig, on, (next) => backend().setAppBool(KEY, next), "reference completion preference");
}

/** Load the device preference at startup (default ON); read failure toasts and resolves. */
export async function initRefCompletionSettings(): Promise<void> {
  const revision = preferenceRevision(spaceAfter);
  try {
    const value = await backend().getAppBool(KEY, true);
    if (preferenceReadCurrent(spaceAfter, revision)) { setSpaceAfterSig(value); seedPreference(spaceAfter); }
  } catch {
    pushToast("Could not load reference completion preference.", "error");
  }
}
