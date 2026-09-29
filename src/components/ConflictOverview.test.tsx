import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { backend } from "../backend";
import { resetStore } from "../document";
import { setToasts } from "../toasts";
import type { PaneRouter } from "../router";
import type { ConflictInventory, ConflictObject, SyncConflict } from "../types";
import { ConflictOverview } from "./ConflictOverview";
import { ConflictQueueBadge } from "./Sidebar";
import { setConflictInventory } from "../conflictQueue";

// og 8c: the Conflicts overview (the inventory, never a write surface except
// the recoverable Discard copy) and the sidebar badge that opens it.

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => { for (let i = 0; i < 5; i++) await tick(); };

const copy: SyncConflict = {
  path: "pages/Plan.sync-conflict-20260705-141233-ABCDEFG.md", base_name: "Plan", base_path: "pages/Plan.md",
  kind: "page", tag: "sync-conflict-20260705-141233-ABCDEFG", preview: "copy",
};
const orphan: SyncConflict = {
  path: "pages/Gone.sync-conflict-20260705-141233-ABCDEFG.md", base_name: "Gone", base_path: null,
  kind: "page", tag: "sync-conflict-20260705-141233-ABCDEFG", preview: "orphan",
};
const copyObject: ConflictObject = {
  id: `copy:${copy.path}`, source: "sync-copy", page_name: "Plan", page_path: "pages/Plan.md", kind: "page",
  sides: [{ role: "mine", label: "This device", path: "pages/Plan.md" }, { role: "theirs", label: copy.tag, path: copy.path }],
  block_conflicts: 3,
};
const markerObject: ConflictObject = {
  id: "markers:pages/Merged.md", source: "vcs-markers", page_name: "Merged", page_path: "pages/Merged.md", kind: "page",
  sides: [{ role: "mine", label: "HEAD" }, { role: "theirs", label: "feature" }],
  block_conflicts: null, markers: ["<<<<<<<", ">>>>>>>"],
};
const inventory: ConflictInventory = {
  sync_conflicts: [copy, orphan],
  vcs_markers: [{ path: "pages/Merged.md", name: "Merged", kind: "page", markers: ["<<<<<<<", ">>>>>>>"] }],
  queue: [copyObject, markerObject],
};

function mount(node: () => JSX.Element) {
  const host = document.createElement("div");
  document.body.append(host);
  return { host, dispose: render(node, host) };
}

afterEach(() => {
  setConflictInventory({ sync_conflicts: [], vcs_markers: [], queue: [] });
  setToasts([]);
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

describe("the conflict overview", () => {
  it("lists every conflicted page by source with its block count, absent shown as —", async () => {
    vi.spyOn(backend(), "conflictInventory").mockResolvedValue(inventory);
    const router = { openPageTarget: vi.fn() } as unknown as PaneRouter;
    const { host, dispose } = mount(() => <ConflictOverview router={router} />);
    await settle();
    const groups = [...host.querySelectorAll("section")].map((s) => s.getAttribute("aria-label"));
    expect(groups).toEqual(["Sync conflict copies", "Version-control merge markers"]);
    const rows = [...host.querySelectorAll(".conflict-overview-row")].map((r) =>
      [r.querySelector(".conflict-overview-open")!.textContent, r.querySelector(".conflict-overview-count")!.textContent]);
    expect(rows).toEqual([["Plan", "3 blocks"], ["Gone", "—"], ["Merged", "—"]]);
    expect(host.textContent).toContain("its page no longer exists");
    (host.querySelector("button.conflict-overview-open") as HTMLButtonElement).click();
    expect(router.openPageTarget).toHaveBeenCalledWith({ name: "Plan", pageKind: "page", path: "pages/Plan.md" });
    dispose();
  });

  it("discards a sync copy by its own file, after confirmation", async () => {
    const list = vi.spyOn(backend(), "conflictInventory").mockResolvedValue(inventory);
    vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    const trash = vi.spyOn(backend(), "trashSyncConflict").mockResolvedValue();
    const { host, dispose } = mount(() => <ConflictOverview router={{} as PaneRouter} />);
    await settle();
    (host.querySelector(".settings-btn-danger") as HTMLButtonElement).click();
    await settle();
    expect(trash).toHaveBeenCalledWith(copy.path, "delete-page");
    expect(list).toHaveBeenCalledTimes(2);
    dispose();
  });

  // Moved from the retired Settings sync-conflict panel (I-20): a confirmation
  // answered after a graph switch must not trash a same-named file in the new graph.
  it("does not discard a sync conflict after its confirmation outlives the graph", async () => {
    vi.spyOn(backend(), "conflictInventory").mockResolvedValue(inventory);
    let finish!: (confirmed: boolean) => void;
    vi.spyOn(backend(), "confirm").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const trash = vi.spyOn(backend(), "trashSyncConflict").mockResolvedValue();
    const { host, dispose } = mount(() => <ConflictOverview router={{} as PaneRouter} />);
    try {
      await settle();
      (host.querySelector(".settings-btn-danger") as HTMLButtonElement).click();
      resetStore();
      finish(true);
      await tick();
      expect(trash).not.toHaveBeenCalled();
    } finally { dispose(); }
  });

  it("stays a valid page when nothing is left", async () => {
    vi.spyOn(backend(), "conflictInventory").mockResolvedValue({ sync_conflicts: [], vcs_markers: [], queue: [] });
    const { host, dispose } = mount(() => <ConflictOverview router={{} as PaneRouter} />);
    await settle();
    expect(host.querySelector(".conflict-overview-empty")!.textContent).toContain("No conflicts");
    dispose();
  });
});

describe("the sidebar conflict badge", () => {
  it("counts queued pages plus copies whose page is gone, and hides at zero", async () => {
    const { host, dispose } = mount(() => <ConflictQueueBadge />);
    expect(host.querySelector(".conflict-queue-badge")).toBeNull();
    setConflictInventory(inventory);
    await tick();
    expect(host.querySelector(".conflict-queue-badge")!.textContent).toBe("3 conflicts");
    setConflictInventory({ ...inventory, sync_conflicts: [copy], queue: [markerObject] });
    await tick();
    expect(host.querySelector(".conflict-queue-badge")!.textContent).toBe("1 conflict");
    dispose();
  });
});
