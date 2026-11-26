/** Device preference only; no document writes. Saves go through the shared
 * preference queue (ordered writes, rollback to the confirmed value on a failed
 * save); a startup read lands only if no toggle came after it. Default ON. */
import { createSignal } from "solid-js";
import { backend } from "./backend";
import { preferenceReadCurrent, preferenceRevision, seedPreference, writePreference } from "./preferenceWrites";
import { readOwned, revisionOwner } from "./owned";
import { pushToast } from "./toasts";
import { msg } from "./i18n";

const key = "sticky_parent_bullets";
const [stickyParents, setStickyParents] = createSignal(true);
export { stickyParents };

export async function initStickyParents(): Promise<void> {
  const revision = preferenceRevision(stickyParents);
  const owner = revisionOwner(stickyParents, revision, () => preferenceReadCurrent(stickyParents, revision));
  try {
    const value = await readOwned(owner, backend().getAppString(key, "1"));
    if (value.kind === "current") { setStickyParents(value.value !== "0"); seedPreference(stickyParents); }
  } catch { pushToast(msg("sticky-parents-load-error"), "error"); }
}

/** Apply now and queue a device-local write. A failed save restores the last
 * confirmed value and toasts; return does not confirm persistence. */
export function changeStickyParents(value: boolean): void {
  writePreference(stickyParents, setStickyParents, value,
    (next) => backend().setAppString(key, next ? "1" : "0"), msg("sticky-parents-save-error"));
}
