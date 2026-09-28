// GH #535 (master 861cfb435d89): one page whose save is refused on EVERY
// attempt used to block every rename, because the rename needed the whole
// graph saved before it reset the whole working set. It now blocks only a
// rename that would change that page, and every page the rename did not touch
// keeps its state, unsaved edits included.
import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { renameOrMergePage, renameOutcomeMessage } from "./graph";
import { doc, setDoc } from "./document/model";
import { conflicts, isDirty, resetStore } from "./document";
import { activatePageInstance } from "./document/save/engine";
import { setRaw } from "./document/edits/blocks";
import { undo } from "./document/history";
import type { PageRead } from "./types";

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
});

const node = (id: string, raw: string, page: string) =>
  ({ id, raw, collapsed: false, parent: null, page, children: [] as string[] });
const page = (name: string, root: string) =>
  ({ name, id: `pages/${name}.md`, kind: "page" as const, title: name, preBlock: null, roots: [root], format: "md" as const, readOnly: false, guide: false });

function graph(otherText: string) {
  setDoc({
    byId: {
      old: node("old", "Body", "Old"),
      other: node("other", otherText, "Other"),
      ref: node("ref", "see [[Old]]", "Referrer"),
    },
    pages: [page("Old", "old"), page("Other", "other"), page("Referrer", "ref")],
    feed: ["Old"],
    loaded: true,
  });
  for (const name of ["Old", "Other", "Referrer"]) activatePageInstance(name);
}

const cases = [
  ["unrelated page refused on every save", "io:PermissionDenied"],
  ["unrelated page conflicted on every save", "conflict"],
  ["stuck page mentions the old name", "io:PermissionDenied"],
  ["renamed page itself stuck", "io:PermissionDenied"],
] as const;

it.each(cases)("a stuck page blocks a rename only when the rename would touch it: %s", async (label, family) => {
  const mentions = label === "stuck page mentions the old name";
  const itself = label === "renamed page itself stuck";
  graph("Other");
  const save = vi.spyOn(backend(), "savePages").mockResolvedValue({ failed: { index: 0, family, diskRev: "disk", undoFailed: [] } });
  vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: "pages/New.md" });
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({
    outcome: "renamed",
    touched: [{ path: "pages/Old.md", moved: true }, { path: "pages/Referrer.md", moved: false }],
  });
  const rewritten: PageRead = {
    id: "pages/Referrer.md", name: "Referrer", kind: "page", title: "Referrer", pre_block: null, rev: "r2",
    blocks: [{ id: "ref2", raw: "see [[New]]", collapsed: false, children: [] }],
  } as PageRead;
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue(rewritten);
  // A real edit, refused on every save attempt.
  setRaw(itself ? "old" : "other", mentions ? "draft about [[old]]" : "Other draft", { timetracking: false });

  const outcome = await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" });

  expect(save).toHaveBeenCalled();
  if (itself || mentions) {
    expect(rename).not.toHaveBeenCalled();
    const blocker = itself ? "Old" : "Other";
    expect(outcome).toEqual({ unsaved: blocker, mentions });
    const message = renameOutcomeMessage(outcome, "Old", "New")!;
    expect(message).toContain(`“${blocker}”`);
    if (mentions) expect(message).toContain("mention “Old”");
    expect(doc.byId[itself ? "old" : "other"].raw).toBe(mentions ? "draft about [[old]]" : "Other draft");
    return;
  }
  expect(outcome).toBe("renamed");
  // The stuck page's file is handed to the backend, which refuses to rewrite it.
  expect(rename).toHaveBeenCalledWith("Old", "New", "rename-page", "pages/Old.md", undefined, ["pages/Other.md"]);
  // Still stuck, and its unsaved text and undo survive the rename's refresh.
  expect(isDirty("Other") || conflicts().includes("Other")).toBe(true);
  expect(doc.byId.other.raw).toBe("Other draft");
  // The moved page leaves under its old name; the rewritten one is reloaded.
  expect(doc.pages.some((loaded) => loaded.name === "Old")).toBe(false);
  expect(doc.pages.find((loaded) => loaded.name === "Referrer")?.roots.map((id) => doc.byId[id].raw)).toEqual(["see [[New]]"]);
  if (family !== "conflict") {
    undo();
    expect(doc.byId.other.raw).toBe("Other");
  }
});

it("a rename with every page saved sends no unsaved paths and keeps untouched pages", async () => {
  graph("Other");
  vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: "pages/New.md" });
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "renamed", touched: [{ path: "pages/Old.md", moved: true }] });
  const other = doc.byId.other;
  expect(await renameOrMergePage("Old", "New")).toBe("renamed");
  expect(rename).toHaveBeenCalledWith("Old", "New", "rename-page", undefined, undefined, []);
  // Same object: the untouched page was not reloaded or rebuilt.
  expect(doc.byId.other).toBe(other);
  expect(doc.pages.map((loaded) => loaded.name).sort()).toEqual(["Other", "Referrer"]);
});

it("a rewritten page that cannot be re-read leaves the working set instead of staying stale", async () => {
  graph("Other");
  vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: "pages/New.md" });
  vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "renamed", touched: [{ path: "pages/Referrer.md", moved: false }] });
  vi.spyOn(backend(), "getPageByPath").mockRejectedValue(new Error("io:NotFound"));
  expect(await renameOrMergePage("Old", "New")).toBe("renamed");
  expect(doc.pages.some((loaded) => loaded.name === "Referrer")).toBe(false);
  expect(doc.byId.other.raw).toBe("Other");
});
