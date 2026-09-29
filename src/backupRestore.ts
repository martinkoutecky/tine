/** Confirm, flush dirty pages, then restore backup.stamp; when is display text.
 * Native restore validates the snapshot, takes a safety snapshot, and may replace
 * graph pages, asset sidecars and config. A native failure may leave changed files
 * and recovery copies. A successful restore attempts a forced graph reload;
 * refresh runs only after that reload succeeds. Refusal or stale binding returns
 * silently; most errors toast and resolve, so resolution does not imply success.
 * Busy and transition state are managed here. Cost includes dirty pages, live and
 * snapshot file counts and bytes, and graph reload. */
import { backend, type BackupInfo } from "./backend";
import { graphOwner, ownedWhen, readOwned, writeOwned } from "./owned";
import { flushAll } from "./document";
import { loadGraphPath } from "./graph";
import { graphMeta } from "./graphSession";
import { pushToast } from "./toasts";
import { setGraphTransitioning } from "./ui";

/** Confirm and flush pending pages, then restore the selected snapshot. The
 * native restore may replace pages, journals, assets and config and can leave
 * changed files or recovery copies on failure. A successful restore force-
 * reloads the graph; refresh runs only after that reload succeeds. Refusal or
 * stale graph ownership returns silently. Most errors toast and resolve, so
 * completion is not proof of success. Cost follows dirty pages, live/snapshot
 * files and bytes, and graph reload. */
export async function restoreBackupFromSettings(
  backup: BackupInfo,
  when: string,
  setBusy: (busy: boolean) => void,
  refresh: () => void,
): Promise<void> {
  const owner = graphOwner(), root = graphMeta()?.root ?? "";
  const ownsTransition = ownedWhen(() => owner() || (!!root && graphMeta()?.root === root));
  const confirmed = await readOwned(owner, backend().confirm(
    `Restore the snapshot from ${when}?\n\n` +
      `This restores the ${backup.files} file(s) in that backup to their original locations. ` +
      `Your current state is snapshotted first, so this is reversible.`
  ));
  if (confirmed.kind === "stale" || !confirmed.value) return;
  setBusy(true);
  setGraphTransitioning(true);
  try {
    if (!(await flushAll())) {
      if (owner()) pushToast("Some pages couldn't be saved — resolve conflicts before restoring.", "error");
      return;
    }
    if (!owner()) return;
    const restored = await writeOwned(owner, backend().restoreBackup(backup.stamp, "replace-page"));
    if (restored.kind === "stale") return;
    const outcome = await loadGraphPath(root, { forceRefresh: true, transitionHeld: true });
    if (!ownsTransition()) return;
    if (outcome.kind !== "loaded" && outcome.kind !== "already_current") {
      pushToast("Snapshot restored, but the graph couldn't be reloaded. Reopen it to see the restored files.", "error");
      return;
    }
    pushToast(`Restored snapshot from ${when}`, "success");
    refresh();
  } catch (error) {
    pushToast(`Restore failed: ${String(error)}`, "error");
  } finally {
    if (ownsTransition()) setGraphTransitioning(false);
    setBusy(false);
  }
}
