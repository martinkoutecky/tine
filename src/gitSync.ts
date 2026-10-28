import { t } from "./i18n";
import type { GitSyncStatus } from "./backendTypes";

/** What the user can do about a sync problem (D-20261027-OF: a persistent indicator offers a fix). */
export type GitSyncFix = "sync-now" | "sign-in" | "copy-command" | "open-folder";

export interface GitSyncProblem {
  text: string;
  fixes: GitSyncFix[];
  /** The git command "copy the git command" copies. */
  command?: string;
}

/** One refusal-table row of ADR 0078 §4 in plain language, for Settings → Graph → Git and the app's notice
 *  alike; `null` for a cycle that succeeded or none yet. `root` is the graph folder the git commands name. */
export function gitSyncProblem(sync: GitSyncStatus | undefined | null, root: string | undefined): GitSyncProblem | null {
  if (!sync?.code) return null;
  const message = sync.message ?? "";
  const path = sync.path ?? "";
  const git = (args: string) => root ? `git -C "${root}" ${args}` : `git ${args}`;
  switch (sync.code) {
    case "up-to-date": case "pulled": case "pushed": case "cancelled":
      return null;
    case "diverged":
      return { text: t("git-sync-diverged"), fixes: ["copy-command", "sync-now"], command: git("pull") };
    case "auth-required":
      return { text: t("git-sync-auth-required"), fixes: ["sign-in"] };
    case "network":
      return { text: t("git-sync-network", { message }), fixes: ["sync-now"] };
    case "pull-held-unsaved":
      return { text: t("git-sync-held-unsaved", { path }), fixes: ["sync-now"] };
    case "pull-held-changed":
      return { text: path ? t("git-sync-held-changed", { path }) : t("git-sync-held-changed-unnamed"), fixes: ["open-folder", "sync-now"] };
    case "pull-held-busy": case "busy":
      return { text: t("git-sync-busy"), fixes: ["sync-now"] };
    case "pull-incomplete":
      return { text: t("git-sync-incomplete", { message }), fixes: ["sync-now"] };
    case "pull-refused":
      return { text: t("git-sync-pull-refused", { path, message }), fixes: ["open-folder", "copy-command", "sync-now"], command: git("status") };
    case "pull-stopped":
      return { text: t("git-sync-pull-stopped", { path, message }), fixes: ["open-folder", "copy-command", "sync-now"], command: git("status") };
    case "no-upstream":
      return { text: t("git-sync-no-upstream"), fixes: ["copy-command"], command: git("branch -vv") };
    case "refused":
      return { text: t("git-sync-refused", { message }), fixes: ["open-folder", "sync-now"] };
    default:
      // A row this build has no wording for yet still names what it found and how to go on.
      return { text: path ? t("git-sync-other-path", { path, message }) : t("git-sync-other", { message: message || sync.code }), fixes: ["open-folder", "copy-command", "sync-now"], command: git("status") };
  }
}

/** The last result line, for Settings. */
export function gitSyncResult(sync: GitSyncStatus | undefined): string | null {
  if (!sync) return null;
  if (sync.running) return t("git-sync-running");
  if (!sync.at) return null;
  const time = new Date(sync.at * 1000).toLocaleTimeString();
  switch (sync.code) {
    case "up-to-date": return t("git-sync-last-up-to-date", { time });
    case "pulled": return t("git-sync-last-pulled", { time });
    case "pushed": return t("git-sync-last-pushed", { time });
    default: return sync.ok_at
      ? t("git-sync-last-failed-since", { time, ok: new Date(sync.ok_at * 1000).toLocaleTimeString() })
      : t("git-sync-last-failed", { time });
  }
}
