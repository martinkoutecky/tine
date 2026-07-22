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

const KEY = "space_after_ref_completion";

const [spaceAfter, setSpaceAfterSig] = createSignal(true);

/** Reactive: insert a space after `]]`/`))` when accepting a page/block-ref
 *  completion? ON = Tine default (GH #35); OFF = Logseq (caret right after the
 *  closing brackets, no space). */
export const spaceAfterRefCompletion = spaceAfter;

export function setSpaceAfterRefCompletion(on: boolean): void {
  setSpaceAfterSig(on);
  void backend().setAppBool(KEY, on).catch(() => {});
}

/** Load the persisted preference at startup. Tine default: ON (differs from Logseq). */
export async function initRefCompletionSettings(): Promise<void> {
  try {
    setSpaceAfterSig(await backend().getAppBool(KEY, true));
  } catch {
    /* default on */
  }
}
