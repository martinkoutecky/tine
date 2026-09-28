/** Persisted block navigation for sidebar and tab actions. Each action waits
 * for one target page save before opening a route; a refused save shows a toast
 * and opens nothing. Callers need no page-save or debounce state. */
import { persistentBlockRef } from "./document";
import { openInNewTab } from "./router";
import { openBlockInSidebar } from "./ui";
import { pushToast } from "./toasts";

/** Open a persisted block destination only after its target ID has reached disk. */
export async function openDurableBlock(id: string, destination: "sidebar" | "tab"): Promise<void> {
  try {
    const ref = await persistentBlockRef(id);
    if (ref) {
      if (destination === "sidebar") openBlockInSidebar(ref);
      else openInNewTab({ kind: "page", name: ref.page, pageKind: ref.pageKind, block: ref.uuid, ...(ref.path ? { path: ref.path } : {}) });
      return;
    }
  } catch { /* The save error is reported below. */ }
  pushToast(`Could not save the block ID before opening the ${destination}.`, "error");
}
