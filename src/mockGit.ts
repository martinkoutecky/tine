// Git (autocommit, seen, sync) needs a repository: the browser preview and the
// test mock have none, so each status reports it unavailable and every action
// is refused.
import type { Backend } from "./backendInterface";

export const mockGitCommands = {
  gitCloneGraph: () => Promise.reject(new Error("Git is not available in the browser mock.")),
  gitSetupSignIn: () => Promise.reject(new Error("Git is not available in the browser mock.")),
  gitInitGraph: () => Promise.reject(new Error("Git is not available in the browser mock.")),
  gitFinishSetup: () => Promise.reject(new Error("Git is not available in the browser mock.")),
  async gitAutocommitStatus() {
    return { available: false };
  },
  async getPageChanges() {
    return { available: false, reason: "Git is not available in the browser mock.", head_commit: null,
      baseline_commit: null, current_text_sha256: "", identity_known: false, unattributed: false, blocks: [], removed: [] };
  },
  async markPageSeen(): Promise<never> { throw new Error("Git is not available in the browser mock."); },
  async markGraphSeen(): Promise<never> { throw new Error("Git is not available in the browser mock."); },
  async forgetSeenPage(): Promise<void> {},
  async forgetSeenState(): Promise<void> {},
  async setGitAutocommit(): Promise<never> {
    throw new Error("git is not available in the browser mock");
  },
  async gitSyncNotice() {
    return null;
  },
  // Git sync, like the setting above, has no browser-mock repository.
  gitSyncNow: () => Promise.reject(new Error("git is not available in the browser mock")),
  gitDisableLineEndings: () => Promise.reject(new Error("git is not available in the browser mock")),
  gitRemoveStaleLock: () => Promise.reject(new Error("git is not available in the browser mock")),
  gitSignIn: () => Promise.reject(new Error("git is not available in the browser mock")),
  gitSignOut: () => Promise.reject(new Error("git is not available in the browser mock")),
  // No repository, so no git thread to tell.
  async gitAppLifecycle(): Promise<void> {},
} satisfies Partial<Backend>;
