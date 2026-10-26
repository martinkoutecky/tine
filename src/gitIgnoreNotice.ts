/** The one-time toast after git autocommit added `.syncthing.*` to the graph's
 *  `.gitignore` (D-20261026-GI; Martin 2026-10-26 G2: "yes" to a toast as well
 *  as the Settings line, because users who already autocommit never open Settings).
 *
 *  The backend appends the rule at most once per graph and device, and reports it
 *  in the worker's status until the worker ends; this module only shows that
 *  report. The worker sets it up just after the graph binds, so the status is
 *  asked a few times. A window shows it once per graph root. */
import { msg } from "./i18n";
import { backend } from "./backend";
import { bindingOwner, readOwned } from "./owned";
import { pushToast } from "./toasts";

const shown = new Set<string>();
export const NOTICE_DELAYS_MS = [1_000, 5_000, 15_000, 45_000];

export async function maybeShowSyncthingIgnoreNotice(
  root: string,
  wait: (ms: number) => Promise<void> = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
): Promise<void> {
  const owner = bindingOwner();
  let waited = 0;
  for (const at of NOTICE_DELAYS_MS) {
    await wait(at - waited);
    waited = at;
    if (shown.has(root) || !owner()) return;
    let last;
    try {
      const answer = await readOwned(owner, backend().gitAutocommitStatus());
      if (answer.kind === "stale") return;
      last = answer.value.last;
    } catch {
      return; // A missing notice never harms the graph.
    }
    if (last?.syncthing_ignore_added) {
      shown.add(root);
      pushToast(msg("git-settings-syncthing-ignore-added", { path: last.syncthing_ignore_added }), "info", { sticky: true });
      return;
    }
    if (last?.syncthing_ignore_failed) {
      shown.add(root);
      pushToast(msg("git-settings-syncthing-ignore-failed"), "info", { sticky: true });
      return;
    }
  }
}

/** Tests only. */
export function resetSyncthingIgnoreNoticeForTests(): void {
  shown.clear();
}
