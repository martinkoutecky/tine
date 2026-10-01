/** Device-local automatic-update preference in tine-settings.json. O(1) plus
 * backend I/O; one cached startup read. Read failure toasts and disables automatic
 * scheduling for this launch. Writes apply now, persist in order, and roll back
 * with a toast on failure; return does not confirm persistence. Manual checks
 * never consult this preference. */
import { createSignal } from "solid-js";
import { backend } from "./backend";
import { writePreference, seedPreference, preferenceRevision, preferenceReadCurrent } from "./preferenceWrites";
import { ownedWhen, readOwned } from "./owned";
import { pushToast } from "./toasts";

const KEY = "check_for_updates_automatically";
const [automatic, setAutomatic] = createSignal(true);
export const checkForUpdatesAutomatically = automatic;
let loaded: Promise<boolean> | undefined;

/** Apply the device-local preference; save failure rolls back and toasts. */
export function setCheckForUpdatesAutomatically(on: boolean): void {
  writePreference(automatic, setAutomatic, on, (next) => backend().setAppBool(KEY, next), "automatic update preference");
}

/** Load once (default ON). Resolve false after a visible read failure. */
export function initUpdateSettings(): Promise<boolean> {
  return loaded ??= (async () => {
    const revision = preferenceRevision(automatic);
    try {
      const result = await readOwned(ownedWhen(() => preferenceReadCurrent(automatic, revision)), backend().getAppBool(KEY, true));
      if (result.kind === "current") { setAutomatic(result.value); seedPreference(automatic); }
      return true;
    } catch {
      pushToast("Could not load automatic update preference.", "error");
      return false;
    }
  })();
}
