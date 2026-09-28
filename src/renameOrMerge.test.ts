import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { renameOrMergePage, renameOutcomeMessage } from "./graph";
import { tryFreezeGraphRewrite } from "./document/graphRewriteState";
import { bumpGraphEpoch } from "./graphSession";
import { resetStore } from "./document";

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
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
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValueOnce({ outcome: "renamed", touched: [] }).mockResolvedValueOnce({ outcome: "unchanged", touched: [] });
  resolve.mockResolvedValueOnce({ kind: "absent", id: "pages/New.md" });
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("renamed");
  // A case-only rename resolves to the source itself; the backend writes
  // nothing and the outcome says so (Rule 2 B2), never "renamed".
  resolve.mockResolvedValueOnce({ kind: "existing", id: "pages/Old.md", others: [] })
    .mockResolvedValueOnce({ kind: "existing", id: "pages/Old.md", others: [] });
  expect(await renameOrMergePage("Old", "old")).toBe("unchanged");
  expect(renameOutcomeMessage("unchanged", "Old", "old")).toContain("Nothing renamed");
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
  bumpGraphEpoch();
  finish();
  expect(await pending).toBe("uncertain");
  expect(renameOutcomeMessage("uncertain", "Old", "New")).toContain("Check whether");
  expect(renameOutcomeMessage("busy", "Old", "New")).not.toContain("pending edits");
});

// og 12b Rule 2 B2: the backend also writes nothing for a name no file and no
// reference uses (a never-saved page nobody links to), so the message may not
// claim a case-only rename there.
it("words an unchanged rename truthfully whether or not it was case-only", async () => {
  vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: "pages/Other.md" });
  vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "unchanged", touched: [] });
  const outcome = await renameOrMergePage("Draft", "Other");
  expect(outcome).toBe("unchanged");
  const message = renameOutcomeMessage(outcome, "Draft", "Other")!;
  expect(message).not.toContain("same page name");
  expect(message).toContain("“Draft”");
  expect(renameOutcomeMessage("unchanged", "Old", "old")).toContain("same page name");
});
