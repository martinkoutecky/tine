/** Restore a graph snapshot from Settings. O(dirty pages + snapshot files).
 * A failed flush or native restore reports an error and leaves the current graph
 * usable; a successful restore reloads the graph. Callers need not coordinate
 * graph binding or transition state. */
import { backend, type BackupInfo } from "./backend";
import { captureBinding, stillBound } from "./binding";
import { flushAll } from "./document";
import { loadGraphPath } from "./graph";
import { graphMeta } from "./graphSession";
import { pushToast } from "./toasts";
import { setGraphTransitioning } from "./ui";

export async function restoreBackupFromSettings(
  backup: BackupInfo,
  when: string,
  setBusy: (busy: boolean) => void,
  refresh: () => void,
): Promise<void> {
  const binding = captureBinding(), root = graphMeta()?.root ?? "";
  const ownsTransition = () => stillBound(binding) || (!!root && graphMeta()?.root === root);
  if (!(await backend().confirm(
    `Restore the snapshot from ${when}?\n\n` +
      `This overwrites journals/ and pages/ with the ${backup.files} file(s) in that backup. ` +
      `Your current state is snapshotted first, so this is reversible.`
  ))) return;
  if (!stillBound(binding)) return;
  setBusy(true);
  setGraphTransitioning(true);
  try {
    if (!(await flushAll())) {
      if (stillBound(binding)) pushToast("Some pages couldn't be saved — resolve conflicts before restoring.", "error");
      return;
    }
    if (!stillBound(binding)) return;
    await backend().restoreBackup(backup.stamp, "replace-page");
    if (!stillBound(binding)) return;
    const outcome = await loadGraphPath(root, { forceRefresh: true, transitionHeld: true });
    if (!ownsTransition()) return;
    if (outcome.kind !== "loaded" && outcome.kind !== "already_current") {
      pushToast("Snapshot restored, but the graph couldn't be reloaded. Reopen it to see the restored files.", "error");
      return;
    }
    pushToast(`Restored snapshot from ${when}`, "success");
    refresh();
  } catch (error) {
    if (stillBound(binding)) pushToast(`Restore failed: ${String(error)}`, "error");
  } finally {
    if (ownsTransition()) setGraphTransitioning(false);
    setBusy(false);
  }
}
