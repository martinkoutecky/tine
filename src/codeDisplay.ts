/** Device-local code wrapping, shared by every rendered code card and editor.
 * Reads and changes are O(1); startup and changes use the existing app-settings
 * door. Failures report a toast; a delayed startup read cannot undo a user change.
 * Language/body structure remains the parser's answer, never this preference.
 */
import { createSignal } from "solid-js";
import { backend } from "./backend";
import { pushToast } from "./toasts";
import { advanceRevision, currentRevision, readOwned, revisionOwner, writeOwned } from "./owned";

const key = "code_line_wrapping";
const slot = {};
const [codeWrapping, setWrapping] = createSignal(false);
export { codeWrapping };

export async function initCodeDisplay(): Promise<void> {
  try {
    const loaded = await readOwned(revisionOwner(slot, currentRevision(slot)), backend().getAppString(key, "0"));
    if (loaded.kind === "current") setWrapping(loaded.value === "1");
  } catch (error) {
    pushToast(`Could not load code wrapping: ${String(error)}`, "error");
  }
}

export function changeCodeWrapping(value: boolean): void {
  setWrapping(value);
  const owner = revisionOwner(slot, advanceRevision(slot));
  void writeOwned(owner, backend().setAppString(key, value ? "1" : "0"))
    .catch(error => pushToast(`Could not remember code wrapping: ${String(error)}`, "error"));
}
