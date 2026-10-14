/** Device preference only; no document writes. O(1) changes, revision-owned
 * startup reads cannot overwrite a later toggle. Default ON (R2 manager). */
import { createSignal } from "solid-js";
import { backend } from "./backend";
import { advanceRevision, currentRevision, readOwned, revisionOwner, writeOwned } from "./owned";
import { pushToast } from "./toasts";
import { msg } from "./i18n";

const key = "sticky_parent_bullets", slot = {};
const [stickyParents, setStickyParents] = createSignal(true);
export { stickyParents };

export async function initStickyParents(): Promise<void> {
  try {
    const value = await readOwned(revisionOwner(slot, currentRevision(slot)), backend().getAppString(key, "1"));
    if (value.kind === "current") setStickyParents(value.value !== "0");
  } catch { pushToast(msg("sticky-parents-load-error"), "error"); }
}

export function changeStickyParents(value: boolean): void {
  setStickyParents(value);
  void writeOwned(revisionOwner(slot, advanceRevision(slot)), backend().setAppString(key, value ? "1" : "0"))
    .catch(() => pushToast(msg("sticky-parents-save-error"), "error"));
}
