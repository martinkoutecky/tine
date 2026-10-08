import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { renameOrMergePage, renameOutcomeMessage } from "./graph";
import { tryFreezeGraphRewrite } from "./document/graphRewriteState";
import { bumpGraphEpoch } from "./graphSession";
import { invalidateBinding } from "./binding";
import { resetStore } from "./document";
import { recentPages, setRecentPages, rightSidebar, setRightSidebar } from "./ui";

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  setRecentPages([]);
  setRightSidebar([]);
});

// GH #327 / OG `merge-pages!`: renaming onto another page's name offers a merge.
it("asks before merging onto an existing page and names the confirmed survivor", async () => {
  vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "existing", id: "pages/New.md", others: [] });
  const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "merged", touched: [] });
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("merged");
  expect(confirm).toHaveBeenCalledWith("Page “New” already exists. Merge “Old” into it?");
  expect(rename).toHaveBeenCalledWith("Old", "New", "rename-page", "pages/Old.md", "pages/New.md", []);
});

it("declining the merge writes nothing", async () => {
  vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "existing", id: "pages/New.md", others: [] });
  vi.spyOn(backend(), "confirm").mockResolvedValue(false);
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "renamed", touched: [] });
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("cancelled");
  expect(rename).not.toHaveBeenCalled();
});

it("renames without asking when the name is free or is the page's own", async () => {
  const resolve = vi.spyOn(backend(), "resolvePage");
  const confirm = vi.spyOn(backend(), "confirm");
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValueOnce({ outcome: "renamed", touched: [] }).mockResolvedValueOnce({ outcome: "renamed", touched: [] });
  resolve.mockResolvedValueOnce({ kind: "absent", id: "pages/New.md" });
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("renamed");
  // GH #609: the same identity changes spelling without a merge prompt.
  resolve.mockResolvedValueOnce({ kind: "existing", id: "pages/Old.md", others: [] })
    .mockResolvedValueOnce({ kind: "existing", id: "pages/Old.md", others: [] });
  expect(await renameOrMergePage("Old", "old")).toBe("renamed");
  expect(renameOutcomeMessage("renamed", "Old", "old")).toBeNull();
  expect(confirm).not.toHaveBeenCalled();
  expect(rename).toHaveBeenNthCalledWith(1, "Old", "New", "rename-page", "pages/Old.md", undefined, []);
  expect(rename).toHaveBeenNthCalledWith(2, "Old", "old", "rename-page", undefined, undefined, []);
});

// Rule 2 B1: an alias reaches its owner's page, and a reference-only `from`
// repoints its references there, so both ask before merging.
it("asks before merging onto an alias owner or from a file-less name", async () => {
  const resolve = vi.spyOn(backend(), "resolvePage");
  const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "merged", touched: [] });
  resolve.mockResolvedValueOnce({ kind: "alias", owners: ["pages/Owner.md"] });
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("merged");
  expect(rename).toHaveBeenLastCalledWith("Old", "New", "rename-page", "pages/Old.md", "pages/Owner.md", []);
  resolve.mockResolvedValueOnce({ kind: "existing", id: "pages/New.md", others: [] })
    .mockResolvedValueOnce({ kind: "absent", id: "pages/Ghost.md" });
  expect(await renameOrMergePage("Ghost", "New")).toBe("merged");
  expect(rename).toHaveBeenLastCalledWith("Ghost", "New", "rename-page", undefined, "pages/New.md", []);
  expect(confirm).toHaveBeenCalledTimes(2);
});

// Rule 2 B7: a refused start, a failed flush and a possibly committed rename
// are different outcomes with different messages.
it("distinguishes a busy rewrite from an unsaved edit and an uncertain commit", async () => {
  vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: "pages/New.md" });
  const release = tryFreezeGraphRewrite()!;
  expect(await renameOrMergePage("Old", "New")).toBe("busy");
  release();
  let finish!: () => void;
  vi.spyOn(backend(), "renamePage").mockImplementationOnce(() => new Promise((done) => { finish = () => done({ outcome: "renamed", touched: [] }); }));
  const pending = renameOrMergePage("Old", "New");
  await vi.waitFor(() => expect(backend().renamePage).toHaveBeenCalledOnce());
  // The graph owner is the binding (R4): a graph switch/restore retires it.
  invalidateBinding();
  finish();
  expect(await pending).toBe("uncertain");
  expect(renameOutcomeMessage("uncertain", "Old", "New")).toContain("Check whether");
  expect(renameOutcomeMessage("busy", "Old", "New")).not.toContain("pending edits");
});

// R4 / I-20 (og-flow3): a display repaint (here: another page's rename bumping
// the epoch) on the same graph does not make a landed rename "uncertain".
it("a rename that lands after a display repaint reports what it did", async () => {
  vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: "pages/New.md" });
  let finish!: () => void;
  vi.spyOn(backend(), "renamePage").mockImplementationOnce(() => new Promise((done) => { finish = () => done({ outcome: "renamed", touched: [] }); }));
  const pending = renameOrMergePage("Old", "New");
  await vi.waitFor(() => expect(backend().renamePage).toHaveBeenCalledOnce());
  bumpGraphEpoch();
  finish();
  expect(await pending).toBe("renamed");
});

// og 12b Rule 2 B2: the backend also writes nothing for a name no file and no
// reference uses (a never-saved page nobody links to), so the message may not
// claim a case-only rename there.
it("words unchanged spelling and absent source truthfully", async () => {
  vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: "pages/Other.md" });
  vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "unchanged", touched: [] });
  const outcome = await renameOrMergePage("Draft", "Other");
  expect(outcome).toBe("unchanged");
  const message = renameOutcomeMessage(outcome, "Draft", "Other")!;
  expect(message).not.toContain("same page name");
  expect(message).toContain("“Draft”");
  expect(renameOutcomeMessage("unchanged", "Old", "Old")).toContain("already has that spelling");
  expect(renameOutcomeMessage("unchanged", "Old", "old")).toContain("no page file or reference");
});

it("a case-only rename refreshes recent and right-sidebar names through the ordinary intent", async () => {
  setRecentPages([{ name: "my note", kind: "page", path: "pages/my note.md" }]);
  setRightSidebar([{ kind: "page", name: "my note", pageKind: "page", path: "pages/my note.md" }]);
  vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "existing", id: "pages/my note.md", others: [] });
  vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "renamed", touched: [{ path: "pages/my note.md", moved: true }] });
  const confirmed = vi.spyOn(backend(), "confirm");
  expect(await renameOrMergePage("my note", "My Note", { name: "my note", pageKind: "page", path: "pages/my note.md" })).toBe("renamed");
  expect(recentPages()).toEqual([{ name: "My Note", kind: "page" }]);
  expect(rightSidebar()).toEqual([{ kind: "page", name: "My Note", pageKind: "page", path: undefined }]);
  expect(confirmed).not.toHaveBeenCalled();
});
