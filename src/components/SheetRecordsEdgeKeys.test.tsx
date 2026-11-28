// A table or board cell that is not a grid cell (a query table title, a query board card, a
// children-table row title, a children-board card) edits only its result's own text. Its
// deleting keys (Backspace at the start, Delete at the end, with any modifier, in either
// editor) used to reach the outline's merge and delete writers, which absorbed or removed a
// source block the table or board never draws. Those writers now refuse there silently;
// everything the keys did before the writer (the cell's own guards, a command bound to the
// key, own-numbering removal, a pending draft's save) is unchanged, as is every other editor
// (grid cells, list faces, the ordinary outline). Related: tine#874.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block, OutlineScopeContext } from "./Block";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetStore } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { RefGroup } from "../types";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { installKeybindings } from "../keybindings";
import { editingId, editingOwner, endEdit } from "../editorController";
import { resetCellSelectionForTests, setCellSel } from "../sheet/selection";
import { maybeEditorIn, type EditorDriver, type EditorKind } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";
import { setWorkflow } from "../ui";
import { installReferenceChangeCounter, setToasts, toasts } from "../toasts";

const saves = vi.hoisted(() => ({ actual: null as Record<string, unknown> | null, markDirty: vi.fn() }));
vi.mock("../document/host/wiring", () => new Proxy({}, {
  get: (_target, key) => key === "then" ? undefined : key === "markDirty" ? saves.markDirty : saves.actual?.[key as string],
  has: () => true,
}));
// The draft cases need a commit the property join refuses once, as a malformed buffer does.
const join = vi.hoisted(() => ({ refuse: false }));
vi.mock("../editor/properties", async importOriginal => {
  const actual = await importOriginal<typeof import("../editor/properties")>();
  return { ...actual, joinProps: (...args: Parameters<typeof actual.joinProps>) => {
    if (join.refuse) throw new Error("join refused");
    return actual.joinProps(...args);
  } };
});

beforeAll(async () => {
  saves.actual = await vi.importActual("../document/host/wiring");
  await initParser();
});

const references = new Map<string, number>();
beforeEach(() => {
  references.clear();
  join.refuse = false;
  setToasts([]);
  setWorkflow("todo");
  installReferenceChangeCounter(ids => ids.reduce((sum, id) => sum + (references.get(id) ?? 0), 0));
});

let disposeKeys: (() => void) | null = null;
const roots = new Set<() => void>();
afterEach(() => {
  join.refuse = false;
  for (const dispose of roots) dispose();
  roots.clear();
  disposeKeys?.();
  disposeKeys = null;
  endEdit("blur");
  vi.restoreAllMocks();
  resetCellSelectionForTests();
  resetSharedQueryResultsForTests();
  resetStore();
  setToasts([]);
  document.body.innerHTML = "";
});

const ID_A = "aaaaaaaa-0000-4000-8000-00000000000a";
const ID_B = "aaaaaaaa-0000-4000-8000-00000000000b";
const ID_P = "aaaaaaaa-0000-4000-8000-00000000000c";
const DATE = "tine.date:: 2026-11-07T13:37Z";
const HIDDEN = "secret:: kept";

type Spec = [id: string, raw: string, children?: Spec[]];
type Face = "table" | "board";

/** Loads pages of nested specs; every block keeps its parent and page. */
function load(pages: [name: string, blocks: Spec[]][]): void {
  const byId: Record<string, StoreNode> = {};
  const add = (pageName: string, parent: string | null) => ([id, raw, children = []]: Spec): string => {
    byId[id] = { id, raw, collapsed: false, parent, page: pageName, children: children.map(add(pageName, id)) };
    return id;
  };
  const feed: FeedPage[] = pages.map(([name, blocks]) =>
    ({ name, kind: "page", title: name, preBlock: null, roots: blocks.map(add(name, null)), format: "md", readOnly: false, guide: false }));
  setDoc({ byId, pages: feed, feed: [pages[0][0]], loaded: true });
}

/** A query on page Sheet whose results are `ids` (blocks of other pages), drawn as `face`. */
function queryOn(face: Face | "list", ids: string[]): void {
  const parse = backend().parseQuery.bind(backend());
  if (face !== "list") vi.spyOn(backend(), "parseQuery").mockImplementation(async (...args) => {
    const parsed = await parse(...args);
    return { ...parsed, view: { ...parsed.view, group_by: "" }, block_presentation: face };
  });
  const byPage = new Map<string, RefGroup>();
  for (const id of ids) {
    const n = doc.byId[id];
    const group = byPage.get(n.page) ?? { page: n.page, kind: "page", blocks: [] };
    group.blocks.push({ id, raw: n.raw, collapsed: false, children: [] });
    byPage.set(n.page, group);
  }
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([...byPage.values()]));
  vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
  vi.spyOn(backend(), "printQuery").mockResolvedValue("(task TODO)");
}

const QUERY_HOST = (face: Face) => `{{query (task TODO)}}\ntine.view:: ${face}`;

function mount(host: string, wrap: (block: JSX.Element) => JSX.Element = b => b): HTMLDivElement {
  const root = document.createElement("div");
  document.body.appendChild(root);
  roots.add(render(() => wrap(<Block id={host} />), root));
  disposeKeys = installKeybindings();
  return root;
}

function remapKeys(bindings: Record<string, string>): void {
  disposeKeys?.();
  disposeKeys = installKeybindings(bindings);
}

/** Opens the sheet editor of `id` (row `row` of a table, or its board card) with the caret at `caret`. */
async function editCell(root: HTMLElement, face: Face, id: string, caret: "start" | "end", row = 0): Promise<EditorDriver> {
  const sheet = await vi.waitFor(() => {
    const el = root.querySelector<HTMLElement>(`.sheet-${face}`);
    expect(el).not.toBeNull();
    return el!;
  });
  const target = await vi.waitFor(() => {
    const el = face === "board"
      ? sheet.querySelector<HTMLElement>(`.sheet-board-card[data-block-id="${id}"]`)
      : sheet.querySelector<HTMLElement>(`.sheet-cell[data-row="${row}"][data-col="0"]:not(.sheet-header-cell)`);
    expect(el).not.toBeNull();
    return el!;
  });
  target.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, button: 0 }));
  const editor = await vi.waitFor(() => {
    const ed = maybeEditorIn(sheet, id);
    expect(ed).not.toBeNull();
    return ed!;
  });
  editor.select(caret === "end" ? editor.value().length : 0);
  return editor;
}

/** Every block's text, parent, children and page, and every page's roots. */
const snapshot = () => JSON.stringify({
  blocks: Object.keys(doc.byId).sort().map(id => [id, doc.byId[id].raw, doc.byId[id].parent, doc.byId[id].children, doc.byId[id].page]),
  pages: doc.pages.map(p => [p.name, p.roots]),
});

/** Presses `init` and checks that nothing was written: the page, the dirty calls, the
 *  toasts, the editor (block, owner, text and caret) are as they were. */
async function expectNoWrite(editor: EditorDriver, id: string, init: KeyboardEventInit & { key: string }): Promise<KeyboardEvent> {
  const before = snapshot(), owner = editingOwner(), text = editor.value(), caret = editor.selection();
  expect(owner).toMatch(/^sheet:/);
  saves.markDirty.mockClear();
  setToasts([]);
  const event = editor.keydown(init);
  await Promise.resolve();
  await Promise.resolve();
  expect(snapshot()).toBe(before);
  expect(saves.markDirty).not.toHaveBeenCalled();
  expect(toasts()).toEqual([]);
  expect(editingId()).toBe(id);
  expect(editingOwner()).toBe(owner);
  expect(editor.value()).toBe(text);
  expect(editor.selection()).toEqual(caret);
  return event;
}

const MODIFIERS: [string, KeyboardEventInit][] = [
  ["plain", {}],
  ["Shift", { shiftKey: true }],
  ["Ctrl", { ctrlKey: true }],
  ["Alt", { altKey: true }],
  ["Meta", { metaKey: true }],
  ["Ctrl+Shift", { ctrlKey: true, shiftKey: true }],
  ["Alt+Shift", { altKey: true, shiftKey: true }],
  ["Meta+Shift", { metaKey: true, shiftKey: true }],
];
const MODIFIED = MODIFIERS.filter(([name]) => name !== "plain" && name !== "Shift");

/** Page Work: `todo-a` (the one result) after a writable block and before another, each
 *  with a child and a hidden property, so a merge, a reparenting or a dropped property shows. */
function workPage(aRaw = "TODO Write tests"): Spec[] {
  return [
    ["before", `Before\n${HIDDEN}`, [["before-kid", "Before kid"]]],
    ["todo-a", aRaw],
    ["after", `After\n${HIDDEN}`, [["after-kid", "After kid"]]],
  ];
}

/** A dated message `m` drawn as a children `face`: rows `a`, then `b`, which the face's filter hides. */
function datedChildrenSheet(face: Face, aChildren: Spec[] = []): void {
  const view = face === "table" ? "tine.view:: table" : "tine.view:: board\ntine.group-by:: state";
  load([["Messages", [["passage", "Passage", [
    ["m", `First message\ntine.quote:: Passage\n${DATE}\n${view}\ntine.fields:: n=number\ntine.filter:: n > 1`, [
      ["a", "TODO Alpha\nn:: 2", aChildren],
      ["b", `TODO Beta\nn:: 0\n${HIDDEN}`, [["b-kid", "Beta kid"]]],
    ]],
  ]]]]]);
}

describe.each<EditorKind>(["textarea", "cm6"])("a table or board cell's deleting keys write only its own result (%s)", kind => {
  useEditorKind(kind);
  // CodeMirror leaves modified edge keys to its own text editing, which does nothing at an edge.
  const modifiedReachOutline = kind === "textarea";

  describe("Delete at the end never absorbs a block the face does not draw", () => {
    it.each(["table", "board"] as const)("query %s on a plain host, with a writable next source block", async face => {
      load([["Sheet", [["host", QUERY_HOST(face)]]], ["Work", workPage()]]);
      queryOn(face, ["todo-a"]);
      const root = mount("host");
      const editor = await editCell(root, face, "todo-a", "end");
      for (const [, mods] of MODIFIERS) await expectNoWrite(editor, "todo-a", { key: "Delete", ...mods });
    });

    it.each(["table", "board"] as const)("children %s on a dated message, the next body sibling filtered out", async face => {
      datedChildrenSheet(face);
      const root = mount("m");
      const editor = await editCell(root, face, "a", "end");
      expect(root.querySelector(`.sheet-${face} [data-block-id="b"]`)).toBeNull();
      for (const [, mods] of MODIFIERS) await expectNoWrite(editor, "a", { key: "Delete", ...mods });
    });

    it.each(["table", "board"] as const)("children %s on a dated message, a dated reply under the edited row", async face => {
      datedChildrenSheet(face, [["reply", `Reply\n${DATE}`, [["reply-body", "Reply body"]]]]);
      const root = mount("m");
      const editor = await editCell(root, face, "a", "end");
      for (const [, mods] of MODIFIERS) await expectNoWrite(editor, "a", { key: "Delete", ...mods });
    });

    it("query table under a zoom on its host", async () => {
      load([["Sheet", [["host", QUERY_HOST("table")]]], ["Work", workPage()]]);
      queryOn("table", ["todo-a"]);
      const root = mount("host", b => <OutlineScopeContext.Provider value={{ roots: ["host"] }}>{b}</OutlineScopeContext.Provider>);
      const editor = await editCell(root, "table", "todo-a", "end");
      await expectNoWrite(editor, "todo-a", { key: "Delete" });
    });

    it("a referenced next block: no write and no refusal message", async () => {
      load([["Sheet", [["host", QUERY_HOST("table")]]], ["Work", [
        ["todo-a", `TODO Write tests\nid:: ${ID_A}`],
        ["after", `After\nid:: ${ID_B}`],
      ]], ["Elsewhere", [["ref", `See ((${ID_B}))`]]]]);
      references.set(ID_B, 1);
      queryOn("table", ["todo-a"]);
      const root = mount("host");
      const editor = await editCell(root, "table", "todo-a", "end");
      await expectNoWrite(editor, "todo-a", { key: "Delete" });
    });
  });

  describe("modified Backspace at the start never merges or deletes the result", () => {
    it.each([
      ["a nonempty title", "TODO Write tests"],
      ["an empty title", ""],
      ["an id-only title, referenced", `id:: ${ID_A}`],
    ])("%s after a writable previous source block", async (_label, aRaw) => {
      load([["Sheet", [["host", QUERY_HOST("table")]]], ["Work", workPage(aRaw)]]);
      references.set(ID_A, 1);
      queryOn("table", ["todo-a"]);
      const root = mount("host");
      const editor = await editCell(root, "table", "todo-a", "start");
      for (const [, mods] of MODIFIED) await expectNoWrite(editor, "todo-a", { key: "Backspace", ...mods });
    });

    it("an empty first row with a following row", async () => {
      load([["Sheet", [["host", QUERY_HOST("table")]]], ["Work", [["empty", ""], ["next", "TODO Next"]]]]);
      queryOn("table", ["empty", "next"]);
      const root = mount("host");
      const editor = await editCell(root, "table", "empty", "start");
      for (const [, mods] of MODIFIED) await expectNoWrite(editor, "empty", { key: "Backspace", ...mods });
    });

    it("an id-only referenced row whose merge into the previous row is refused", async () => {
      load([["Messages", [["passage", "Passage", [
        ["m", `First message\ntine.quote:: Passage\n${DATE}\ntine.view:: table`, [
          ["p", `TODO P title\nid:: ${ID_P}`],
          ["a", `id:: ${ID_A}`],
          ["b", "TODO B"],
        ]],
      ]]]], ["Elsewhere", [["ref", `See ((${ID_A}))`]]]]);
      references.set(ID_A, 1);
      const root = mount("m");
      const editor = await editCell(root, "table", "a", "start", 1);
      await expectNoWrite(editor, "a", { key: "Backspace", ctrlKey: true });
    });
  });

  describe("what the keys did before the writer is unchanged", () => {
    it.each([false, true])("plain Backspace at the start is consumed and saves nothing (Clear block on Backspace: %s)", async remapped => {
      load([["Sheet", [["host", QUERY_HOST("table")]]], ["Work", workPage()]]);
      queryOn("table", ["todo-a"]);
      const root = mount("host");
      if (remapped) remapKeys({ "editor/clear-block": "backspace" });
      const editor = await editCell(root, "table", "todo-a", "start");
      const event = await expectNoWrite(editor, "todo-a", { key: "Backspace" });
      expect(event.defaultPrevented).toBe(true);
    });

    it("Clear block bound to Delete clears only the result", async () => {
      load([["Sheet", [["host", QUERY_HOST("table")]]], ["Work", workPage()]]);
      queryOn("table", ["todo-a"]);
      const root = mount("host");
      remapKeys({ "editor/clear-block": "delete" });
      const editor = await editCell(root, "table", "todo-a", "end");
      editor.keydown({ key: "Delete" });
      await Promise.resolve();
      expect(doc.byId["todo-a"].raw).toBe("");
      expect(doc.byId.after.raw).toBe(`After\n${HIDDEN}`);
      expect(doc.byId.before.raw).toBe(`Before\n${HIDDEN}`);
      expect(doc.pages.find(p => p.name === "Work")!.roots).toEqual(["before", "todo-a", "after"]);
      expect(editingId()).toBe("todo-a");
    });

    it("modified Backspace on a numbered result removes only its numbering", async () => {
      load([["Sheet", [["host", QUERY_HOST("table")]]], ["Work", workPage("TODO Write tests\nlogseq.order-list-type:: number")]]);
      queryOn("table", ["todo-a"]);
      const root = mount("host");
      const editor = await editCell(root, "table", "todo-a", "start");
      editor.keydown({ key: "Backspace", ctrlKey: true });
      await Promise.resolve();
      expect(doc.byId["todo-a"].raw).toBe(modifiedReachOutline ? "TODO Write tests" : "TODO Write tests\nlogseq.order-list-type:: number");
      expect(doc.byId.before.raw).toBe(`Before\n${HIDDEN}`);
      expect(doc.pages.find(p => p.name === "Work")!.roots).toEqual(["before", "todo-a", "after"]);
      expect(editingId()).toBe("todo-a");
    });

    it.each([true, false])("a pending draft commits on Delete only where develop's next-block preflight passes (next block: %s)", async withNext => {
      load([["Sheet", [["host", QUERY_HOST("table")]]], ["Work", withNext ? workPage() : [["todo-a", "TODO Write tests"]]]]);
      queryOn("table", ["todo-a"]);
      const root = mount("host");
      const editor = await editCell(root, "table", "todo-a", "end");
      // The refused join throws out of the input handler, as it does in the app.
      const swallow = (e: ErrorEvent) => { if (e.message.includes("join refused")) e.preventDefault(); };
      window.addEventListener("error", swallow);
      join.refuse = true;
      try { editor.input("TODO Write tests!", 17); } catch { /* CodeMirror reports it synchronously */ }
      await Promise.resolve();
      join.refuse = false;
      window.removeEventListener("error", swallow);
      expect(doc.byId["todo-a"].raw).toBe("TODO Write tests");
      if (kind === "cm6") return; // CodeMirror commits no draft on this key; its text stays in the editor alone.
      editor.keydown({ key: "Delete" });
      await Promise.resolve();
      expect(doc.byId["todo-a"].raw).toBe(withNext ? "TODO Write tests!" : "TODO Write tests");
      if (withNext) {
        expect(doc.byId.after.raw).toBe(`After\n${HIDDEN}`);
        expect(doc.byId.after.children).toEqual(["after-kid"]);
        expect(doc.pages.find(p => p.name === "Work")!.roots).toEqual(["before", "todo-a", "after"]);
      }
      // A released draft lets the editor close; a held one keeps it open.
      endEdit("blur");
      expect(editingId()).toBe(withNext ? null : "todo-a");
    });

    it("an ordinary children table on a routed page writes nothing on either edge", async () => {
      load([["Sheet", [["host", "Sheet\ntine.view:: table", [
        ["one", "TODO One", [["kid", `Kid\n${HIDDEN}`]]],
        ["two", `TODO Two\n${HIDDEN}`],
      ]]]]]);
      const root = mount("host");
      const editor = await editCell(root, "table", "one", "end");
      await expectNoWrite(editor, "one", { key: "Delete" });
      await expectNoWrite(editor, "one", { key: "Delete", ctrlKey: true });
    });

    if (kind === "cm6") it("Shift+Delete at the end of a calc title is still consumed", async () => {
      load([["Sheet", [["host", "Sheet\ntine.view:: table", [["calc", "```calc\n1 + 1\n```"], ["two", "TODO Two"]]]]]]);
      const root = mount("host");
      const editor = await editCell(root, "table", "calc", "end");
      const event = await expectNoWrite(editor, "calc", { key: "Delete", shiftKey: true });
      expect(event.defaultPrevented).toBe(true);
    });
  });

  describe("every other editor keeps the outline's writers", () => {
    it("a list-face result absorbs its next source block on Delete", async () => {
      load([["Sheet", [["host", "{{query (task TODO)}}"]]], ["Work", workPage()]]);
      queryOn("list", ["todo-a"]);
      const root = mount("host");
      const row = await vi.waitFor(() => {
        const el = root.querySelector<HTMLElement>('[data-block-id="todo-a"] .block-content');
        expect(el).not.toBeNull();
        return el!;
      });
      row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      const editor = await vi.waitFor(() => {
        const ed = maybeEditorIn(root, "todo-a");
        expect(ed).not.toBeNull();
        return ed!;
      });
      editor.select(editor.value().length);
      editor.keydown({ key: "Delete" });
      await Promise.resolve();
      expect(doc.byId.after).toBeUndefined();
      expect(doc.byId["todo-a"].raw).toContain("TODO Write testsAfter");
      expect(doc.byId["todo-a"].children).toEqual(["after-kid"]);
    });

    it.each(["Backspace", "Delete"] as const)("an ordinary outline block merges on %s", async key => {
      load([["Page", [["one", "One"], ["two", "Two"]]]]);
      const root = mount("one");
      const two = document.createElement("div");
      document.body.appendChild(two);
      roots.add(render(() => <Block id="two" />, two));
      const target = key === "Delete" ? "one" : "two";
      const host = key === "Delete" ? root : two;
      host.querySelector<HTMLElement>(`[data-block-id="${target}"] .block-content`)!
        .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      const editor = await vi.waitFor(() => {
        const ed = maybeEditorIn(host, target);
        expect(ed).not.toBeNull();
        return ed!;
      });
      editor.select(key === "Delete" ? editor.value().length : 0);
      editor.keydown({ key });
      await Promise.resolve();
      expect(doc.byId.two).toBeUndefined();
      expect(doc.byId.one.raw).toBe("OneTwo");
    });

    it.each(["Backspace", "Delete"] as const)("a grid cell inside a dated message removes its empty first child on %s", async key => {
      load([["Messages", [["passage", "Passage", [
        ["m", `First message\ntine.quote:: Passage\n${DATE}`, [
          ["grid", "Grid\ntine.view:: grid", [["r1", "", [["c1", "Cell", [["k", ""]]], ["c2", "Beta"]]]]],
        ]],
      ]]]]]);
      const root = mount("m");
      await vi.waitFor(() => expect(root.querySelector('.sheet-cell[data-row="0"][data-col="0"]')).not.toBeNull());
      setCellSel({ gridId: "grid", row: 0, col: 0 });
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      await new Promise(r => setTimeout(r, 0));
      let editor = maybeEditorIn(root)!;
      editor.select(editor.value().length);
      if (key === "Backspace") {
        editor.keydown({ key: "ArrowDown" });
        await new Promise(r => setTimeout(r, 0));
        expect(editingId()).toBe("k");
        editor = await vi.waitFor(() => {
          const ed = maybeEditorIn(root);
          expect(editingId()).toBe("k");
          expect(ed).not.toBeNull();
          return ed!;
        });
        editor.select(0);
      }
      editor.keydown({ key });
      await new Promise(r => setTimeout(r, 0));
      expect(doc.byId.k).toBeUndefined();
      expect(doc.byId.c1.raw).toBe("Cell");
      expect(doc.byId.r1.children).toEqual(["c1", "c2"]);
      expect(editingId()).toBe("c1");
      expect(editingOwner()).toMatch(/^sheet:/);
      expect(maybeEditorIn(root)!.value()).toBe("Cell");
      expect(maybeEditorIn(root)!.selection()).toEqual([4, 4]);
    });
  });
});
