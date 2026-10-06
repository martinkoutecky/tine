import { afterEach, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { backend } from "../backend";
import { doc, setDoc } from "./model";
import { deletePage, resetStore } from "./workingSet";
import { activatePageInstance, createPage } from "./save/engine";
import { setRaw } from "./edits/blocks";
import { pasteClipboardPayload } from "./edits/paste";
import { moveBlock, moveItem } from "./edits/moves";
import { installRenameRefreshHandler, renamePageOnDisk } from "./graphRewrite";
import { bumpGraphEpoch, graphMeta, setGraphMeta } from "../graphSession";
import { invalidateBinding } from "../binding";
import type { GraphMeta } from "../types";
import { setToasts, toasts } from "../toasts";

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
});

it("refuses typing, paste, and move while rename IPC is in flight", async () => {
  setDoc({ byId: {
    a: { id: "a", raw: "first", collapsed: false, parent: null, page: "A", children: [] },
    b: { id: "b", raw: "second", collapsed: false, parent: null, page: "A", children: [] },
  }, pages: [{ name: "A", id: "pages/A.md", kind: "page", title: "A", preBlock: null, roots: ["a", "b"], format: "md", readOnly: false, guide: false }], feed: ["A"], loaded: true });
  activatePageInstance("A");
  let finish!: () => void;
  // The renamed page moved, so it leaves the working set under its old name.
  const rename = vi.spyOn(backend(), "renamePage").mockImplementationOnce(() => new Promise((resolve) => { finish = () => resolve({ outcome: "renamed", touched: [{ path: "pages/A.md", moved: true }] }); }));
  installRenameRefreshHandler(() => expect(doc.pages).toHaveLength(0));
  const pending = renamePageOnDisk("A", "B");
  await vi.waitFor(() => expect(rename).toHaveBeenCalledTimes(1));
  setRaw("a", "typed", { timetracking: false });
  moveItem("a", 1);
  await moveBlock("a", null, 2);
  expect(doc.byId.a.raw).toBe("first");
  expect(doc.pages[0].roots).toEqual(["a", "b"]);
  // The paste intent shares blockWritable with typing and moves.
  expect(await pasteClipboardPayload("a", { op: "copy", generation: 1, graph: "", text: "pasted", sourcePages: [], blocks: [{ raw: "pasted", sourceFormat: "md", children: [] }] })).toBeNull();
  await expect(createPage("New", { name: "New", kind: "page", title: "New", pre_block: null, blocks: [] })).rejects.toMatchObject({ reason: "graph-rewrite" });
  expect(await deletePage("A", "page")).toBe(false);
  finish();
  expect(await pending).toBe("renamed");
  expect(doc.pages).toHaveLength(0);
});

it("routes both rename controls through the document intent", () => {
  for (const file of ["src/components/Page.tsx", "src/components/ContextMenu.tsx"]) {
    const source = readFileSync(file, "utf8");
    expect(source).toContain("renameOrMergePage(");
    expect(source).not.toContain("backend().renamePage(");
  }
  // The one app-layer rename entry resolves the collision, then uses the intent.
  expect(readFileSync("src/graph.ts", "utf8")).toContain("renamePageOnDisk(from, to, target, into, onRefreshed)");
});

it("reports a durable rename failure after the graph owner retires", async () => {
  setToasts([]);
  let rejectRename!: (error: Error) => void;
  const rename = vi.spyOn(backend(), "renamePage").mockImplementationOnce(() =>
    new Promise((_resolve, reject) => { rejectRename = reject; }));
  const pending = renamePageOnDisk("A", "B");
  await vi.waitFor(() => expect(rename).toHaveBeenCalledOnce());
  // The graph owner is the binding (R4): a graph switch/restore retires it.
  invalidateBinding();
  const failure = new Error("rename rollback incomplete");
  rejectRename(failure);
  await expect(pending).rejects.toBe(failure);
  expect(toasts().at(-1)?.message).toContain("rename rollback incomplete");
  setToasts([]);
});

it("refuses a page creation that started before the rename freeze", async () => {
  let finishResolve!: (value: { kind: "absent"; id: string }) => void;
  const resolve = vi.spyOn(backend(), "resolvePage").mockImplementationOnce(() => new Promise((done) => { finishResolve = done; }));
  const save = vi.spyOn(backend(), "savePages");
  const creating = createPage("New", { name: "New", kind: "page", title: "New", pre_block: null, blocks: [] });
  await vi.waitFor(() => expect(resolve).toHaveBeenCalledTimes(1));
  let finishRename!: () => void;
  const rename = vi.spyOn(backend(), "renamePage").mockImplementationOnce(() => new Promise((done) => { finishRename = () => done({ outcome: "renamed", touched: [] }); }));
  installRenameRefreshHandler(() => {});
  const renaming = renamePageOnDisk("A", "B");
  await vi.waitFor(() => expect(rename).toHaveBeenCalledTimes(1));
  finishResolve({ kind: "absent", id: "pages/New.md" });
  await expect(creating).rejects.toMatchObject({ reason: "graph-rewrite" });
  expect(save).not.toHaveBeenCalled();
  finishRename();
  expect(await renaming).toBe("renamed");
});

it("names the referrers a rename left untouched because they are mid-merge (og 21a, master a8fd4230d)", async () => {
  setToasts([]);
  vi.spyOn(backend(), "renamePage").mockResolvedValueOnce({ outcome: "renamed", touched: [],
    skipped_conflicted_referrers: ["pages/Conflicted.md"] });
  installRenameRefreshHandler(() => {});
  expect(await renamePageOnDisk("A", "B")).toBe("renamed");
  const note = toasts().find((toast) => toast.message.includes("pages/Conflicted.md"));
  expect(note).toMatchObject({ kind: "warn", sticky: true });
  expect(note!.message).toContain("“A”");
  setToasts([]);
});

it("takes in the home page a rename moved with it (og 22b, OG rename-page-aux)", async () => {
  setGraphMeta({ root: "/g", default_home: "Start" } as GraphMeta);
  vi.spyOn(backend(), "renamePage").mockResolvedValueOnce({ outcome: "renamed", touched: [], home_page: "Begin" });
  installRenameRefreshHandler(() => {});
  expect(await renamePageOnDisk("Start", "Begin")).toBe("renamed");
  expect(graphMeta()?.default_home).toBe("Begin");
  vi.spyOn(backend(), "renamePage").mockResolvedValueOnce({ outcome: "renamed", touched: [], home_page: null });
  expect(await renamePageOnDisk("Other", "Kit")).toBe("renamed");
  expect(graphMeta()?.default_home).toBe("Begin");
  setGraphMeta(null);
});

it("keeps rewritten referrers frozen until their disk reload lands (OG-P10C)", async () => {
  setDoc({ byId: { ref: { id: "ref", raw: "[[Old]]", collapsed: false, parent: null, page: "Ref", children: [] } },
    pages: [{ name: "Ref", id: "pages/Ref.md", kind: "page", title: "Ref", preBlock: null, roots: ["ref"], format: "md", readOnly: false, guide: false }], feed: [], loaded: true });
  activatePageInstance("Ref");
  vi.spyOn(backend(), "renamePage").mockResolvedValueOnce({ outcome: "renamed", touched: [{ path: "pages/Ref.md", moved: false }] });
  let finish!: (value: null) => void;
  const reload = vi.spyOn(backend(), "getPageByPath").mockImplementationOnce(() => new Promise((done) => { finish = done; }));
  installRenameRefreshHandler(() => bumpGraphEpoch());
  const refreshed = vi.fn();
  let completed = false;
  const pending = renamePageOnDisk("Old", "New", undefined, undefined, refreshed).then((value) => { completed = true; return value; });
  await vi.waitFor(() => expect(reload).toHaveBeenCalledOnce());
  expect(refreshed).toHaveBeenCalledOnce(); expect(completed).toBe(false);
  setRaw("ref", "typed during reload", { timetracking: false });
  expect(doc.byId.ref.raw).toBe("[[Old]]");
  finish(null); expect(await pending).toBe("renamed");
  expect(doc.pages).toHaveLength(0);
});

it("GH #623 refresh matches each loaded page once across a large touched set", async () => {
  const loaded = Array.from({ length: 100 }, (_, i) => ({ name: `Ref${i}`, id: `pages/Ref${i}.md`, kind: "page" as const,
    title: `Ref${i}`, preBlock: null, roots: [], format: "md" as const, readOnly: false, guide: false }));
  const loadedCount = loaded.length;
  setDoc({ byId: {}, pages: loaded, feed: [], loaded: true });
  const touched = Array.from({ length: 200 }, (_, i) => ({ path: `pages/Ref${i}.md`, moved: false }));
  vi.spyOn(backend(), "renamePage").mockResolvedValueOnce({ outcome: "renamed", touched });
  const reload = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
  installRenameRefreshHandler(() => {});
  // Count visits at the literal intent, before unreadable reloads drop pages.
  let visits = 0;
  const originalFind = doc.pages.find;
  const find = vi.spyOn(doc.pages, "find").mockImplementation((predicate, thisArg) =>
    originalFind.call(doc.pages, (page, index, array) => { visits++; return predicate.call(thisArg, page, index, array); }));
  expect(await renamePageOnDisk("Old", "New")).toBe("renamed");
  expect(reload).toHaveBeenCalledTimes(100);
  expect(doc.pages).toHaveLength(0);
  find.mockRestore();
  expect(visits).toBeLessThanOrEqual(loadedCount + touched.length);
});

it("does not reload a moved page reported again in the touched set", async () => {
  setDoc({ byId: {}, pages: [{ name: "Old", id: "pages/Old.md", kind: "page", title: "Old",
    preBlock: null, roots: [], format: "md", readOnly: false, guide: false }], feed: [], loaded: true });
  vi.spyOn(backend(), "renamePage").mockResolvedValueOnce({ outcome: "renamed",
    touched: [{ path: "pages/Old.md", moved: true }, { path: "pages/Old.md", moved: false }] });
  const reload = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
  installRenameRefreshHandler(() => {});
  expect(await renamePageOnDisk("Old", "New")).toBe("renamed");
  expect(doc.pages).toHaveLength(0);
  expect(reload).not.toHaveBeenCalled();
});
