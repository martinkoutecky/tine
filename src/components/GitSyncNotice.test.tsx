import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { GitSyncNotice } from "./GitSyncNotice";
import { backend } from "../backend";
import type { GitSyncStatus } from "../backendTypes";
import { setGraphMeta } from "../graphSession";
import { settingsOpen, settingsTabRequest, closeSettings } from "../ui";
import type { GraphMeta } from "../types";

afterEach(() => { vi.restoreAllMocks(); setGraphMeta(null); closeSettings(); document.body.innerHTML = ""; });

const sync = (over: Partial<GitSyncStatus> = {}): GitSyncStatus => ({
  running: false, at: 1_700_000_000, code: "diverged", message: null, path: null, ok_at: null, failures: 1, persistent: true, ...over,
});
function mount() {
  setGraphMeta({ root: "/graph" } as GraphMeta);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <GitSyncNotice />, root);
  return { root, dispose };
}

it("shows a persistent sync problem outside Settings with its fixes, and clears when the sync recovers", async () => {
  vi.useFakeTimers();
  try {
    const notice = vi.spyOn(backend(), "gitSyncNotice").mockResolvedValue(sync());
    const { root, dispose } = mount();
    await vi.waitFor(() => expect(root.querySelector("[data-testid=git-sync-notice]")).not.toBeNull());
    expect(root.textContent).toContain("Git sync needs attention");
    expect(root.textContent).toContain("both have new commits");
    expect(root.querySelector("[data-git-sync-command]")!.textContent).toBe('git -C "/graph" pull');
    root.querySelector<HTMLButtonElement>("[data-git-sync-settings]")!.click();
    expect(settingsOpen()).toBe(true);
    expect(settingsTabRequest()).toBe("graph");
    notice.mockResolvedValue(null);
    await vi.advanceTimersByTimeAsync(15_000);
    expect(root.querySelector("[data-testid=git-sync-notice]")).toBeNull();
    dispose();
  } finally {
    vi.useRealTimers();
  }
});

it("offers sign-in for a rejected token and Sync now for a remote that could not be reached", async () => {
  const notice = vi.spyOn(backend(), "gitSyncNotice").mockResolvedValue(sync({ code: "auth-required" }));
  let view = mount();
  await vi.waitFor(() => expect(view.root.querySelector("[data-git-sync-fix=sign-in]")).not.toBeNull());
  view.root.querySelector<HTMLButtonElement>("[data-git-sync-fix=sign-in]")!.click();
  expect(settingsTabRequest()).toBe("graph");
  view.dispose(); document.body.innerHTML = ""; closeSettings();

  notice.mockResolvedValue(sync({ code: "network", message: "connection refused", failures: 3 }));
  const now = vi.spyOn(backend(), "gitSyncNow").mockResolvedValue({ available: true, last: { committed_at: null, commit: null, refused: null, stale_lock: null, sync: sync({ code: "network", running: true }) } });
  view = mount();
  await vi.waitFor(() => expect(view.root.textContent).toContain("couldn't reach the remote (connection refused)"));
  view.root.querySelector<HTMLButtonElement>("[data-git-sync-fix=sync-now]")!.click();
  await vi.waitFor(() => expect(now).toHaveBeenCalledOnce());
  view.dispose();
});

it("shows nothing for a problem that has not persisted", async () => {
  vi.spyOn(backend(), "gitSyncNotice").mockResolvedValue(sync({ code: "network", persistent: false }));
  const { root, dispose } = mount();
  await Promise.resolve(); await Promise.resolve();
  expect(root.querySelector("[data-testid=git-sync-notice]")).toBeNull();
  dispose();
});

it("offers the safe repository repair and clears the notice after the command succeeds", async () => {
  vi.spyOn(backend(), "gitSyncNotice").mockResolvedValue(sync({ code: "refused", line_endings: { safe: true, differing: 0, sample: null, reason: null } }));
  const repair = vi.spyOn(backend(), "gitDisableLineEndings").mockResolvedValue({ available: true });
  const { root, dispose } = mount();
  await vi.waitFor(() => expect(root.textContent).toContain("Turn off line-ending conversion for this repository"));
  root.querySelector<HTMLButtonElement>("[data-git-sync-fix=line-endings]")!.click();
  await vi.waitFor(() => expect(repair).toHaveBeenCalledOnce());
  await vi.waitFor(() => expect(root.querySelector("[data-testid=git-sync-notice]")).toBeNull());
  dispose();
});

it("explains differing bytes with a count and sample and offers Guide steps instead of a repair", async () => {
  vi.spyOn(backend(), "gitSyncNotice").mockResolvedValue(sync({ code: "refused", line_endings: { safe: false, differing: 12, sample: "pages/windows.md", reason: null } }));
  const { root, dispose } = mount();
  await vi.waitFor(() => expect(root.textContent).toContain("12 tracked files"));
  expect(root.textContent).toContain("pages/windows.md");
  expect(root.querySelector("[data-git-sync-fix=line-endings]")).toBeNull();
  expect(root.querySelector("[data-git-sync-fix=line-endings-guide]")).not.toBeNull();
  dispose();
});

it("keeps the refusal visible when the click's fresh check fails", async () => {
  vi.spyOn(backend(), "gitSyncNotice").mockResolvedValue(sync({ code: "refused", line_endings: { safe: true, differing: 0, sample: null, reason: null } }));
  vi.spyOn(backend(), "gitDisableLineEndings").mockRejectedValue(new Error("1 tracked file differs: pages/edited.md"));
  const { root, dispose } = mount();
  await vi.waitFor(() => expect(root.querySelector("[data-git-sync-fix=line-endings]")).not.toBeNull());
  root.querySelector<HTMLButtonElement>("[data-git-sync-fix=line-endings]")!.click();
  await vi.waitFor(() => expect(root.querySelector("[role=alert]")?.textContent).toContain("pages/edited.md"));
  expect(root.querySelector("[data-testid=git-sync-notice]")).not.toBeNull();
  dispose();
});

const staleLock = { path: "/graph/.git/index.lock", modified: 1_700_000_000, token: "0:1700000000000000000:0" };

it("GS-LOCK: offers to remove a stale index.lock, passes its token back, and clears after the click", async () => {
  vi.spyOn(backend(), "gitSyncNotice").mockResolvedValue(sync({ code: "stale-lock", path: staleLock.path, stale_lock: staleLock }));
  const remove = vi.spyOn(backend(), "gitRemoveStaleLock").mockResolvedValue({ available: true });
  const { root, dispose } = mount();
  await vi.waitFor(() => expect(root.querySelector("[data-git-sync-fix=remove-stale-lock]")).not.toBeNull());
  expect(root.textContent).toContain("Git commits are waiting");
  expect(root.textContent).toContain("/graph/.git/index.lock");
  expect(root.querySelector("[data-git-sync-fix=sync-now]")).toBeNull();
  const button = root.querySelector<HTMLButtonElement>("[data-git-sync-fix=remove-stale-lock]")!;
  expect(button.textContent).toBe("Remove the stale lock");
  button.click();
  await vi.waitFor(() => expect(remove).toHaveBeenCalledWith(staleLock.token));
  await vi.waitFor(() => expect(root.querySelector("[data-testid=git-sync-notice]")).toBeNull());
  dispose();
});

it("GS-LOCK: keeps the offer and says why when the click's recheck refuses", async () => {
  vi.spyOn(backend(), "gitSyncNotice").mockResolvedValue(sync({ code: "stale-lock", path: staleLock.path, stale_lock: staleLock }));
  vi.spyOn(backend(), "gitRemoveStaleLock").mockRejectedValue(new Error("The lock file changed since Tine offered to remove it"));
  const { root, dispose } = mount();
  await vi.waitFor(() => expect(root.querySelector("[data-git-sync-fix=remove-stale-lock]")).not.toBeNull());
  root.querySelector<HTMLButtonElement>("[data-git-sync-fix=remove-stale-lock]")!.click();
  await vi.waitFor(() => expect(root.querySelector("[role=alert]")?.textContent).toContain("Couldn't remove the lock"));
  expect(root.querySelector("[role=alert]")?.textContent).toContain("changed since Tine offered");
  expect(root.querySelector("[data-testid=git-sync-notice]")).not.toBeNull();
  dispose();
});
