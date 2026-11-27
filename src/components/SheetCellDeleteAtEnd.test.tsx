// Delete at the end of an ordinary sheet cell editor (a table title or a board card) stays
// in the cell, as Backspace at its start already does (tine#874). The outline's Delete
// merged the block's next sibling into it: for a query result, a block of the result's
// source page that the sheet never shows; for a children sheet, the next row or card.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetStore } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { RefGroup } from "../types";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { installKeybindings } from "../keybindings";
import { editingId, endEdit } from "../editorController";
import { resetCellSelectionForTests } from "../sheet/selection";
import { maybeEditorIn, type EditorKind } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";
import { setWorkflow } from "../ui";

const saves = vi.hoisted(() => ({ actual: null as Record<string, unknown> | null, markDirty: vi.fn() }));
vi.mock("../document/host/wiring", () => new Proxy({}, {
  get: (_target, key) => key === "then" ? undefined : key === "markDirty" ? saves.markDirty : saves.actual?.[key as string],
  has: () => true,
}));

beforeAll(async () => {
  saves.actual = await vi.importActual("../document/host/wiring");
  await initParser();
});

let disposeKeys: (() => void) | null = null;
const roots = new Set<() => void>();
afterEach(() => {
  for (const dispose of roots) dispose();
  roots.clear();
  disposeKeys?.();
  disposeKeys = null;
  endEdit("blur");
  vi.restoreAllMocks();
  resetCellSelectionForTests();
  resetSharedQueryResultsForTests();
  resetStore();
  document.body.innerHTML = "";
});

const node = (id: string, raw: string, parent: string | null, children: string[] = [], page = "Sheet"): StoreNode =>
  ({ id, raw, collapsed: false, parent, page, children });
const page = (name: string, roots: string[]): FeedPage =>
  ({ name, kind: "page", title: name, preBlock: null, roots, format: "md", readOnly: false, guide: false });
const HIDDEN = "Hidden neighbour\nsecret:: kept";

/** A query host on page Sheet whose one result `todo-a` sits on page Work before a block
 *  the query does not return. `grid` makes the host a grid as well (one cell). */
function queryHost(face: "table" | "board", grid: boolean): void {
  const hostRaw = grid ? `{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: ${face}` : `{{query (task TODO)}}\ntine.view:: ${face}`;
  const cells: Record<string, StoreNode> = grid
    ? { "host-r0": node("host-r0", "", "host", ["host-c00"]), "host-c00": node("host-c00", "Alpha", "host-r0") }
    : {};
  setDoc({ byId: {
    host: node("host", hostRaw, null, grid ? ["host-r0"] : []), ...cells,
    "todo-a": node("todo-a", "TODO Write tests", null, [], "Work"),
    after: node("after", HIDDEN, null, [], "Work"),
  }, pages: [page("Sheet", ["host"]), page("Work", ["todo-a", "after"])], feed: ["Sheet"], loaded: true });
  const parse = backend().parseQuery.bind(backend());
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (...args) => {
    const parsed = await parse(...args);
    return { ...parsed, view: { ...parsed.view, group_by: "" }, block_presentation: face };
  });
  const groups: RefGroup[] = [{ page: "Work", kind: "page", blocks: [{ id: "todo-a", raw: "TODO Write tests", collapsed: false, children: [] }] }];
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups));
  vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
  vi.spyOn(backend(), "printQuery").mockResolvedValue("(task TODO)");
}

/** A children table or board: rows `one` (with a child `kid`) and `two` under the host. */
function childrenHost(face: "table" | "board"): void {
  setDoc({ byId: {
    host: node("host", `Sheet\ntine.view:: ${face}`, null, ["one", "two"]),
    one: node("one", "TODO One", "host", ["kid"]),
    kid: node("kid", "Kid\nsecret:: kept", "one"),
    two: node("two", "TODO Two\nsecret:: kept", "host"),
  }, pages: [page("Sheet", ["host"])], feed: ["Sheet"], loaded: true });
}

function mount(): HTMLDivElement {
  const root = document.createElement("div");
  document.body.appendChild(root);
  roots.add(render(() => <Block id="host" />, root));
  disposeKeys = installKeybindings();
  return root;
}

/** Double-click the sheet's title or card for `id`, wait for its editor, put the caret at the end. */
async function editAtEnd(root: HTMLElement, face: "table" | "board", id: string) {
  const sheet = await vi.waitFor(() => {
    const el = root.querySelector<HTMLElement>(`.sheet-${face}`);
    expect(el).not.toBeNull();
    return el!;
  });
  const target = await vi.waitFor(() => {
    const el = face === "board"
      ? sheet.querySelector<HTMLElement>(`.sheet-board-card[data-block-id="${id}"]`)
      : sheet.querySelector<HTMLElement>('.sheet-cell[data-row="0"][data-col="0"]:not(.sheet-header-cell)');
    expect(el).not.toBeNull();
    return el!;
  });
  target.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, button: 0 }));
  const editor = await vi.waitFor(() => {
    const ed = maybeEditorIn(sheet, id);
    expect(ed).not.toBeNull();
    return ed!;
  });
  editor.select(editor.value().length);
  return editor;
}

describe.each<EditorKind>(["textarea", "cm6"])("Delete at the end of a sheet cell editor stays in the cell (%s)", kind => {
  useEditorKind(kind);

  it.each([
    ["a query table title", "table", false],
    ["a query board card", "board", false],
    ["a query table title on a grid host", "table", true],
    ["a query board card on a grid host", "board", true],
  ] as const)("%s: the result's hidden next block stays where it is", async (_label, face, grid) => {
    setWorkflow("todo");
    queryHost(face, grid);
    const root = mount();
    const editor = await editAtEnd(root, face, "todo-a");
    const text = editor.value();
    saves.markDirty.mockClear();
    editor.keydown(new KeyboardEvent("keydown", { key: "Delete", bubbles: true, cancelable: true }));
    await Promise.resolve();
    expect(editingId()).toBe("todo-a");
    expect(editor.value()).toBe(text);
    expect(doc.byId["todo-a"].raw).toBe("TODO Write tests");
    expect(doc.byId.after?.raw).toBe(HIDDEN);
    expect(doc.pages.find(p => p.name === "Work")?.roots).toEqual(["todo-a", "after"]);
    expect(saves.markDirty).not.toHaveBeenCalled();
    if (grid) expect(doc.byId["host-c00"].raw).toBe("Alpha");
  });

  it.each([["a children table title", "table"], ["a children board card", "board"]] as const)(
    "%s: the row's child and the next row stay their own blocks", async (_label, face) => {
      setWorkflow("todo");
      childrenHost(face);
      const root = mount();
      const editor = await editAtEnd(root, face, "one");
      saves.markDirty.mockClear();
      editor.keydown(new KeyboardEvent("keydown", { key: "Delete", bubbles: true, cancelable: true }));
      await Promise.resolve();
      expect(editingId()).toBe("one");
      expect(doc.byId.one.raw).toBe("TODO One");
      expect(doc.byId.two?.raw).toBe("TODO Two\nsecret:: kept");
      expect(doc.byId.host.children).toEqual(["one", "two"]);
      expect(doc.byId.one.children).toEqual(["kid"]);
      expect(doc.byId.kid?.raw).toBe("Kid\nsecret:: kept");
      expect(saves.markDirty).not.toHaveBeenCalled();
    });

});
