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
      syncthing_ignore_added: "/graph/.gitignore",
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
      syncthing_ignore_failed: true,
    },
  }));
  view = mount();
  await vi.waitFor(() => expect(view.root.textContent).toContain("couldn't confirm adding .syncthing.*"));
  expect(view.root.textContent).toContain("Temporary files are still excluded from Tine's commits");
  view.dispose();
});
