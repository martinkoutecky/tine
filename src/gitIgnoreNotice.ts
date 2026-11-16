/** The one-time toast after git autocommit added `.syncthing.*` (D-20261026-GI)
 *  and/or `logseq/.tine-*` (D-20261029-GT) to the graph's `.gitignore`; one
 *  combined notice when both were added at once (Martin 2026-10-26 G2: "yes" to a
 *  toast as well as the Settings line, because users who already autocommit never
 *  open Settings).
 *
 *  The backend appends each line at most once per graph and device, and reports it
 *  in the worker's status until the worker ends; this module only shows that
 *  report. The worker sets it up just after the graph binds, so the status is
 *  asked a few times. A window shows it once per graph root while that graph is
 *  open; reopening needs no memory, as the backend reports an addition only once. */
import { msg } from "./i18n";
import { backend } from "./backend";
import { bindingOwner, readOwned } from "./owned";
import { clearOnBindingInvalidated } from "./binding";
import { pushToast } from "./toasts";
import type { GitAutocommitStatus } from "./backendTypes";

/** Which lines one setup event names: the notice's `which` variant. */
export function ignoreLines(last: GitAutocommitStatus["last"]): "syncthing" | "tine" | "both" {
  const lines = last?.gitignore_lines ?? [];
  const tine = lines.includes("logseq/.tine-*");
  return tine && lines.includes(".syncthing.*") ? "both" : tine ? "tine" : "syncthing";
}

const shown = new Set<string>();
clearOnBindingInvalidated(() => shown.clear());
export const NOTICE_DELAYS_MS = [1_000, 5_000, 15_000, 45_000];

export async function maybeShowGitIgnoreNotice(
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
    if (last?.gitignore_added) {
      shown.add(root);
      pushToast(msg("git-settings-ignore-added", { path: last.gitignore_added, which: ignoreLines(last) }), "info", { sticky: true });
      return;
    }
    if (last?.gitignore_failed) {
      shown.add(root);
      pushToast(msg("git-settings-ignore-failed", { which: ignoreLines(last) }), "info", { sticky: true });
      return;
    }
  }
}

/** Tests only. */
export function resetGitIgnoreNoticeForTests(): void {
  shown.clear();
}
