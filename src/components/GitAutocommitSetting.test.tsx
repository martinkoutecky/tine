import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { GitAutocommitSetting } from "./GitAutocommitSetting";
import { backend, type GitAutocommitStatus } from "../backend";
import { setGraphMeta } from "../graphSession";
import type { GraphMeta } from "../types";

afterEach(() => { vi.restoreAllMocks(); setGraphMeta(null); document.body.innerHTML = ""; });

const repo = (over: Partial<GitAutocommitStatus> = {}): GitAutocommitStatus => ({
  available: true, repository: true, branch: "main", unusable: null, policy: "off", interval_minutes: 10,
  last: { committed_at: null, commit: null, refused: null, stale_lock: null }, ...over,
});
function mount() {
  setGraphMeta({ root: "/graph" } as GraphMeta);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <GitAutocommitSetting />, root);
  return { root, dispose };
}

it("offers the conversion repair before sync can be enabled", async () => {
  vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValue(repo({ sync_unusable: ["refused", "core.autocrlf"], line_endings: { safe: true, differing: 0, sample: null, reason: null } }));
  const repair = vi.spyOn(backend(), "gitDisableLineEndings").mockResolvedValue(repo());
  const { root, dispose } = mount();
  await vi.waitFor(() => expect(root.querySelector("[data-git-sync-fix=line-endings]")).not.toBeNull());
  expect(root.querySelector<HTMLOptionElement>('[value="sync-on-demand"]')!.disabled).toBe(true);
  root.querySelector<HTMLButtonElement>("[data-git-sync-fix=line-endings]")!.click();
  await vi.waitFor(() => expect(repair).toHaveBeenCalledOnce());
  await vi.waitFor(() => expect(root.querySelector<HTMLOptionElement>('[value="sync-on-demand"]')!.disabled).toBe(false));
  dispose();
});

it("shows deferred cleanup separately from a successful commit and clears it on recovery", async () => {
  vi.useFakeTimers();
  try {
    const last = { committed_at: 1_700_000_000, commit: "0123456789abcdef", refused: null, stale_lock: null };
    const status = vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValue(repo({
      policy: "commit", last: { ...last, repack_deferred: true },
    }));
    const { root, dispose } = mount();
    await vi.waitFor(() => expect(root.textContent).toContain("Git cleanup is waiting"));
    expect(root.textContent).toContain("Last commit 01234567");
    expect(root.textContent).toContain("commits continue");
    expect(root.textContent).toContain("retry automatically");
    expect(root.textContent).not.toContain("Last attempt failed");
    expect(root.querySelector("[role=alert]")).toBeNull();
    status.mockResolvedValue(repo({ policy: "commit", last }));
    await vi.advanceTimersByTimeAsync(15_000);
    expect(root.textContent).not.toContain("Git cleanup is waiting");
    dispose();
  } finally {
    vi.useRealTimers();
  }
});

it("turns autocommit on for the bound graph and changes its interval", async () => {
  vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValue(repo());
  const set = vi.spyOn(backend(), "setGitAutocommit").mockImplementation(async (policy, minutes) => repo({ policy, interval_minutes: minutes }));
  const { root, dispose } = mount();
  await vi.waitFor(() => expect(root.querySelector("[data-git-policy]")).not.toBeNull());
  expect(root.textContent).toContain("Branch: main");
  expect(root.querySelector("[data-git-interval]")).toBeNull();
  const select = root.querySelector<HTMLSelectElement>("[data-git-policy]")!;
  select.value = "commit"; select.dispatchEvent(new Event("change"));
  await vi.waitFor(() => expect(root.querySelector("[data-git-interval]")).not.toBeNull());
  expect(set).toHaveBeenCalledWith("commit", 10);
  const interval = root.querySelector<HTMLInputElement>("[data-git-interval]")!;
  interval.value = "30"; interval.dispatchEvent(new Event("change"));
  await vi.waitFor(() => expect(set).toHaveBeenLastCalledWith("commit", 30));
  dispose();
});

it("offers no control outside a repository or off desktop, and names why Tine can't commit", async () => {
  const status = vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValue(repo({ repository: false, branch: null, unusable: ["not-a-repository", "The graph is not in a git repository."] }));
  let view = mount();
  await vi.waitFor(() => expect(view.root.textContent).toContain("isn't in a git repository"));
  expect(view.root.querySelector("[data-git-policy]")).toBeNull();
  view.dispose(); document.body.innerHTML = "";

  status.mockResolvedValue({ available: false });
  view = mount();
  await vi.waitFor(() => expect(view.root.textContent).toContain("not available on this device yet"));
  expect(view.root.querySelector("[data-git-policy]")).toBeNull();
  view.dispose(); document.body.innerHTML = "";

  status.mockResolvedValue(repo({ policy: "commit", unusable: ["sparse", "The repository uses sparse checkout."], last: { committed_at: 1_700_000_000, commit: "0123456789abcdef", refused: null, stale_lock: "/graph/.git/index.lock" } }));
  view = mount();
  await vi.waitFor(() => expect(view.root.querySelector("[data-git-unusable=sparse]")).not.toBeNull());
  expect(view.root.textContent).toContain("Last commit 01234567");
  expect(view.root.textContent).toContain("/graph/.git/index.lock has been held");
  view.dispose();
});

it("keeps a failed change on screen and shows the device's actual choice", async () => {
  const status = vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValue(repo());
  vi.spyOn(backend(), "setGitAutocommit").mockRejectedValue(new Error("disk full"));
  const { root, dispose } = mount();
  await vi.waitFor(() => expect(root.querySelector("[data-git-policy]")).not.toBeNull());
  const select = root.querySelector<HTMLSelectElement>("[data-git-policy]")!;
  select.value = "commit"; select.dispatchEvent(new Event("change"));
  await vi.waitFor(() => expect(root.querySelector("[role=alert]")?.textContent).toContain("Couldn't change git settings: disk full"));
  expect(status).toHaveBeenCalledTimes(2);
  expect(root.querySelector<HTMLSelectElement>("[data-git-policy]")!.value).toBe("off");
  expect(root.querySelector("[data-git-interval]")).toBeNull();
  dispose();
});

it("names the ignore file and rule, explains removal, and discloses a failed setup", async () => {
  const status = vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValue(repo({
    policy: "commit", last: {
      committed_at: null, commit: null, refused: null, stale_lock: null,
      gitignore_added: "/graph/.gitignore", gitignore_lines: [".syncthing.*"],
    },
  }));
  let view = mount();
  await vi.waitFor(() => expect(view.root.textContent).toContain("Tine added .syncthing.* to /graph/.gitignore"));
  expect(view.root.textContent).toContain("You can remove the line; Tine won't add it again");
  expect(view.root.textContent).toContain("Conflict copies are still committed");
  view.dispose(); document.body.innerHTML = "";

  status.mockResolvedValue(repo({
    policy: "commit", last: {
      committed_at: null, commit: null, refused: null, stale_lock: null,
      gitignore_failed: true, gitignore_lines: [".syncthing.*"],
    },
  }));
  view = mount();
  await vi.waitFor(() => expect(view.root.textContent).toContain("couldn't confirm adding .syncthing.*"));
  expect(view.root.textContent).toContain("Temporary files are still excluded from Tine's commits");
  view.dispose();
});

it("names the Tine line, alone or with the Syncthing line, in the same status line (D-20261029-GT)", async () => {
  const status = vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValue(repo({
    policy: "sync-automatically", last: {
      committed_at: null, commit: null, refused: null, stale_lock: null,
      gitignore_added: "/graph/.gitignore", gitignore_lines: ["logseq/.tine-*"],
    },
  }));
  let view = mount();
  await vi.waitFor(() => expect(view.root.textContent).toContain("Tine added logseq/.tine-* to /graph/.gitignore"));
  expect(view.root.textContent).toContain("Copies already in git are removed from it, not from disk, in a separate Tine commit");
  expect(view.root.textContent).not.toContain(".syncthing.*");
  view.dispose(); document.body.innerHTML = "";

  status.mockResolvedValue(repo({
    policy: "commit", last: {
      committed_at: null, commit: null, refused: null, stale_lock: null,
      gitignore_added: "/graph/.gitignore", gitignore_lines: [".syncthing.*", "logseq/.tine-*"],
    },
  }));
  view = mount();
  await vi.waitFor(() => expect(view.root.textContent).toContain("Tine added .syncthing.* and logseq/.tine-* to /graph/.gitignore"));
  expect(view.root.querySelectorAll("[role=status]").length).toBe(1);
  view.dispose(); document.body.innerHTML = "";

  status.mockResolvedValue(repo({
    policy: "commit", last: {
      committed_at: null, commit: null, refused: null, stale_lock: null,
      gitignore_failed: true, gitignore_lines: ["logseq/.tine-*"],
    },
  }));
  view = mount();
  await vi.waitFor(() => expect(view.root.textContent).toContain("couldn't confirm adding logseq/.tine-*"));
  expect(view.root.textContent).toContain("Tine's own files are still excluded from its commits");
  view.dispose();
});

const sync = (over: Partial<NonNullable<NonNullable<GitAutocommitStatus["last"]>["sync"]>> = {}) => ({
  running: false, at: 1_700_000_000, code: "pushed", message: null, path: null, ok_at: 1_700_000_000, failures: 0, persistent: false, ...over,
});
const remote = { name: "origin", url: "https://example.org/me/notes.git", https_origin: "https://example.org" };

it("offers the sync options with the remote, runs Sync now, and refuses them with the repository's reason", async () => {
  const status = vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValue(repo({ remote }));
  const set = vi.spyOn(backend(), "setGitAutocommit").mockImplementation(async (policy, minutes) => repo({ policy, interval_minutes: minutes, remote }));
  const now = vi.spyOn(backend(), "gitSyncNow").mockResolvedValue(repo({ policy: "sync-on-demand", remote, last: { committed_at: null, commit: null, refused: null, stale_lock: null, sync: sync({ running: true }) } }));
  let view = mount();
  await vi.waitFor(() => expect(view.root.querySelector("[data-git-remote=origin]")).not.toBeNull());
  expect(view.root.textContent).toContain("Remote: origin (https://example.org/me/notes.git)");
  expect(view.root.querySelector("[data-git-sync-now]")).toBeNull();
  const select = view.root.querySelector<HTMLSelectElement>("[data-git-policy]")!;
  expect([...select.options].map((o) => o.value)).toEqual(["off", "commit", "sync-on-demand", "sync-automatically"]);
  select.value = "sync-on-demand"; select.dispatchEvent(new Event("change"));
  await vi.waitFor(() => expect(view.root.querySelector("[data-git-sync-now]")).not.toBeNull());
  expect(set).toHaveBeenCalledWith("sync-on-demand", 10);
  expect(view.root.querySelector("[data-git-interval]")).not.toBeNull();
  view.root.querySelector<HTMLButtonElement>("[data-git-sync-now]")!.click();
  await vi.waitFor(() => expect(view.root.textContent).toContain("Syncing…"));
  expect(now).toHaveBeenCalledOnce();
  expect(view.root.querySelector<HTMLButtonElement>("[data-git-sync-now]")!.disabled).toBe(true);
  view.dispose(); document.body.innerHTML = "";

  status.mockResolvedValue(repo({ sync_unusable: ["refused", "The graph folder is inside a bigger repository."] }));
  view = mount();
  await vi.waitFor(() => expect(view.root.querySelector("[data-git-sync-unusable=refused]")).not.toBeNull());
  expect(view.root.textContent).toContain("Tine can't sync here: The graph folder is inside a bigger repository.");
  const options = [...view.root.querySelector<HTMLSelectElement>("[data-git-policy]")!.options];
  expect(options.filter((o) => o.disabled).map((o) => o.value)).toEqual(["sync-on-demand", "sync-automatically"]);
  view.dispose();
});

it("names each sync problem in plain language with a way to fix it", async () => {
  const last = { committed_at: null, commit: null, refused: null, stale_lock: null };
  const status = vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValue(repo({
    policy: "sync-on-demand", remote, last: { ...last, sync: sync({ code: "diverged", ok_at: null, persistent: true, failures: 1 }) },
  }));
  let view = mount();
  await vi.waitFor(() => expect(view.root.querySelector("[data-git-sync-problem=diverged]")).not.toBeNull());
  expect(view.root.textContent).toContain("both have new commits");
  expect(view.root.textContent).toContain("didn't finish");
  expect(view.root.querySelector("[data-git-sync-command]")!.textContent).toBe('git -C "/graph" pull');
  expect(view.root.querySelector("[data-git-sync-fix=copy-command]")).not.toBeNull();
  view.dispose(); document.body.innerHTML = "";

  status.mockResolvedValue(repo({
    policy: "sync-automatically", remote,
    last: { ...last, sync: sync({ code: "pull-refused", path: "pages/link.md", message: "it is a symbolic link", persistent: true }) },
  }));
  view = mount();
  await vi.waitFor(() => expect(view.root.textContent).toContain("Tine can't update pages/link.md: it is a symbolic link."));
  view.dispose(); document.body.innerHTML = "";

  // A refusal after some files were updated names the path and says so.
  status.mockResolvedValue(repo({
    policy: "sync-automatically", remote,
    last: { ...last, sync: sync({ code: "pull-stopped", path: "pages/a.md", message: "a staged change", persistent: true }) },
  }));
  view = mount();
  await vi.waitFor(() => expect(view.root.textContent).toContain("Tine stopped updating the graph at pages/a.md: a staged change. Some files were already updated. Fix it in git"));
  view.dispose(); document.body.innerHTML = "";

  // A row this build has no wording for still names the path and how to go on.
  status.mockResolvedValue(repo({
    policy: "sync-automatically", remote,
    last: { ...last, sync: sync({ code: "pull-later-row", path: "pages/a.md", message: "a symlink appeared", persistent: true }) },
  }));
  view = mount();
  await vi.waitFor(() => expect(view.root.textContent).toContain("Syncing stopped at pages/a.md: a symlink appeared. Some files may already be updated."));
  view.dispose();
});

it("signs in to the remote's host without keeping the token on the page, and signs out", async () => {
  const last = { committed_at: null, commit: null, refused: null, stale_lock: null, sync: sync({ code: "auth-required", persistent: true }) };
  vi.spyOn(backend(), "gitAutocommitStatus").mockResolvedValue(repo({ policy: "sync-on-demand", remote, last }));
  const signIn = vi.spyOn(backend(), "gitSignIn").mockResolvedValue(repo({ policy: "sync-on-demand", remote, last, signed_in: "me" }));
  const signOut = vi.spyOn(backend(), "gitSignOut").mockResolvedValue(repo({ policy: "sync-on-demand", remote, last }));
  const { root, dispose } = mount();
  await vi.waitFor(() => expect(root.textContent).toContain("Not signed in to https://example.org."));
  expect(root.textContent).toContain("Sign in again with a new token");
  root.querySelector<HTMLButtonElement>("[data-git-sync-fix=sign-in]")!.click();
  await vi.waitFor(() => expect(root.querySelector("[data-git-sign-in-form]")).not.toBeNull());
  expect(root.textContent).toContain("GitHub");
  expect(root.textContent).toContain("Codeberg");
  const token = root.querySelector<HTMLInputElement>("[data-git-token]")!;
  expect(token.type).toBe("password");
  root.querySelector<HTMLInputElement>("[data-git-username]")!.value = "me";
  token.value = "secret-token";
  root.querySelector<HTMLFormElement>("[data-git-sign-in-form]")!.dispatchEvent(new Event("submit", { cancelable: true }));
  await vi.waitFor(() => expect(root.textContent).toContain("Signed in to https://example.org as me."));
  expect(signIn).toHaveBeenCalledWith("me", "secret-token");
  expect(root.querySelector("[data-git-sign-in-form]")).toBeNull();
  expect(root.innerHTML).not.toContain("secret-token");
  root.querySelector<HTMLButtonElement>("[data-git-sign-out]")!.click();
  await vi.waitFor(() => expect(root.textContent).toContain("Not signed in"));
  expect(signOut).toHaveBeenCalledOnce();
  dispose();
});
