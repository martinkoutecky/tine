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
import { bumpGraphEpoch } from "../graphSession";
import { setToasts, toasts } from "../toasts";

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
});

it("refuses typing, paste, and move while rename IPC is in flight", async () => {
  setDoc({ byId: {
    a: { id: "a", raw: "first", collapsed: false, parent: null, page: "A", children: [] },
    b: { id: "b", raw: "second", collapsed: false, parent: null, page: "A", children: [] },
  }, pages: [{ name: "A", kind: "page", title: "A", preBlock: null, roots: ["a", "b"], format: "md", readOnly: false, guide: false }], feed: ["A"], loaded: true });
  activatePageInstance("A");
  let finish!: () => void;
  const rename = vi.spyOn(backend(), "renamePage").mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
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
  expect(await pending).toBe(true);
  expect(doc.pages).toHaveLength(0);
});

it("routes both rename controls through the document intent", () => {
  for (const file of ["src/components/Page.tsx", "src/components/ContextMenu.tsx"]) {
    const source = readFileSync(file, "utf8");
    expect(source).toContain("renameOrMergePage(");
    expect(source).not.toContain("backend().renamePage(");
  }
  // The one app-layer rename entry resolves the collision, then uses the intent.
  expect(readFileSync("src/graph.ts", "utf8")).toContain("renamePageOnDisk(from, to, target, into)");
});

it("reports a durable rename failure after the graph owner retires", async () => {
  setToasts([]);
  let rejectRename!: (error: Error) => void;
  const rename = vi.spyOn(backend(), "renamePage").mockImplementationOnce(() =>
    new Promise<void>((_resolve, reject) => { rejectRename = reject; }));
  const pending = renamePageOnDisk("A", "B");
  await vi.waitFor(() => expect(rename).toHaveBeenCalledOnce());
  bumpGraphEpoch();
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
  const rename = vi.spyOn(backend(), "renamePage").mockImplementationOnce(() => new Promise<void>((done) => { finishRename = done; }));
  installRenameRefreshHandler(() => {});
  const renaming = renamePageOnDisk("A", "B");
  await vi.waitFor(() => expect(rename).toHaveBeenCalledTimes(1));
  finishResolve({ kind: "absent", id: "pages/New.md" });
  await expect(creating).rejects.toMatchObject({ reason: "graph-rewrite" });
  expect(save).not.toHaveBeenCalled();
  finishRename();
  expect(await renaming).toBe(true);
});
