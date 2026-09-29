// Device-local Settings dialog size. The signal lives beyond the dialog mount
// so a remembered size applies before the next open.
import { createSignal } from "solid-js";
import { backend } from "./backend";
import { pushToast } from "./toasts";
import { advanceRevision, currentRevision, readOwned, revisionOwner, writeOwned } from "./owned";

const KEY = "settings_dialog_maximized";
const [maximized, setMaximized] = createSignal(false);
const preferenceKey = {};

/** Whether Settings should occupy the near-viewport desktop geometry.
 * O(1), no I/O. */
export const settingsMaximized = maximized;

/** Remember the dialog size for future opens and launches. The current size
 * changes immediately; a failed device-local write is reported. O(1). */
export function setSettingsMaximized(on: boolean): void {
  const revision = advanceRevision(preferenceKey);
  setMaximized(on);
  void writeOwned(revisionOwner(preferenceKey, revision), backend().setAppBool(KEY, on))
    .catch((error) => pushToast(`Could not remember Settings size: ${String(error)}`, "error"));
}

/** Hydrate the remembered size once at startup. A user's later choice wins a
 * delayed read; read failures keep the small default and report an error. */
export async function initSettingsLayout(): Promise<void> {
  const owner = revisionOwner(preferenceKey, currentRevision(preferenceKey));
  try {
    const loaded = await readOwned(owner, backend().getAppBool(KEY, false));
    if (loaded.kind === "current") setMaximized(loaded.value);
  } catch (error) {
    pushToast(`Could not load Settings size: ${String(error)}`, "error");
  }
}
