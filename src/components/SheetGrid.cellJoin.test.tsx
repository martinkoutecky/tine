// GH #874: Backspace at the start of a grid cell's first child and Delete at the end of the
// cell's own text join that child onto the cell's visible text, in both editors. A block
// that is not an ordinary block (a hosted sheet, one carrying sheet settings), a folded
// cell's hidden child, and the text the cell editor hides are never joined or written into.
import { maybeEditorIn, type EditorDriver } from "../tests/editorDriver";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { resetStore, undo } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { editingId, editingOwner } from "../editorController";
import { installKeybindings } from "../keybindings";
import { resetCellSelectionForTests, setCellSel } from "../sheet/selection";

beforeAll(async () => {
  await initParser();
});

let disposeKeys: (() => void) | null = null;
const mounted = new Set<() => void>();

afterEach(() => {
  for (const dispose of mounted) dispose();
  mounted.clear();
  disposeKeys?.();
  disposeKeys = null;
  resetCellSelectionForTests();
  resetStore();
  document.body.innerHTML = "";
});

type Format = "md" | "org";

function node(id: string, raw: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

const prop = (format: Format, key: string, value: string) =>
  format === "md" ? `${key}:: ${value}` : `:PROPERTIES:\n:${key}: ${value}\n:END:`;

/** A grid whose top-left cell `c1` holds `cellRaw` and the given child blocks. */
function load(format: Format, cellRaw: string, kids: StoreNode[], extra: StoreNode[] = []) {
  const page: FeedPage = { name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["grid"], format, readOnly: false, guide: false };
  const byId: Record<string, StoreNode> = {
    grid: node("grid", `Grid\n${prop(format, "tine.view", "grid")}`, null, ["r1"]),
    r1: node("r1", "", "grid", ["c1", "c2"]),
    c1: node("c1", cellRaw, "r1", kids.map(k => k.id)),
    c2: node("c2", "Beta", "r1"),
  };
  for (const k of [...kids, ...extra]) byId[k.id] = k;
  setDoc({ byId, pages: [page], feed: ["Sheet"], loaded: true });
  disposeKeys = installKeybindings();
  const root = document.createElement("div");
  document.body.appendChild(root);
  const unmount = render((): JSX.Element => <Block id="grid" />, root);
  const dispose = () => { mounted.delete(dispose); unmount(); };
  mounted.add(dispose);
  return root;
}

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

function editor(root: HTMLElement): EditorDriver {
  const found = maybeEditorIn(root) as EditorDriver | null;
  if (!found) throw new Error("missing active editor");
  return found;
}

function keydown(target: EditorDriver | EventTarget, key: string): void {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, code: key.startsWith("Arrow") ? key : "" });
  if ("kind" in target) target.keydown(event); else target.dispatchEvent(event);
}

/** Edit the cell's own text, caret at its visible end. */
async function editCellEnd(root: HTMLElement): Promise<EditorDriver> {
  setCellSel({ gridId: "grid", row: 0, col: 0 });
  keydown(window, "Enter");
  await tick();
  const ed = editor(root);
  const end = ed.value().length;
  ed.select(end, end);
  return ed;
}

/** Edit the cell's first child, caret at its start (Down from the cell text's end). */
async function editFirstChildStart(root: HTMLElement): Promise<EditorDriver> {
  const ed = await editCellEnd(root);
  keydown(ed, "ArrowDown");
  await tick();
  const child = editor(root);
  child.select(0, 0);
  return child;
}

const snapshot = () => JSON.stringify(Object.keys(doc.byId).sort().map(id => [id, doc.byId[id].raw, doc.byId[id].children, doc.byId[id].collapsed]));

const keys = ["Delete", "Backspace"] as const;
type Key = typeof keys[number];

async function press(root: HTMLElement, key: Key): Promise<void> {
  const ed = key === "Delete" ? await editCellEnd(root) : await editFirstChildStart(root);
  keydown(ed, key);
  await tick();
}

function expectJoinedInCell(root: HTMLElement, visible: string, caret: number) {
  expect(editingId()).toBe("c1");
  expect(editingOwner()).toMatch(/^sheet:/);
  expect(editor(root).value()).toBe(visible);
  expect(editor(root).selection()).toEqual([caret, caret]);
  expect(doc.byId.r1.children).toEqual(["c1", "c2"]);
}

describe.each(keys)("GH874 cell join: %s", (key) => {
  it.each(["md", "org"] as const)("joins after the visible text, the hidden sheet setting unchanged (%s)", async (format) => {
    const setting = prop(format, "tine.col-widths", "0=120");
    const root = load(format, `Cell\n${setting}`, [node("k", "Child", "c1")]);
    await press(root, key);
    expect(doc.byId.c1.raw).toBe(`CellChild\n${setting}`);
    expect(doc.byId.k).toBeUndefined();
    expectJoinedInCell(root, "CellChild", 4);
  });

  it.each(["md", "org"] as const)("a blank cell body takes the child's text before the hidden setting (%s)", async (format) => {
    const setting = prop(format, "tine.col-widths", "0=120");
    const root = load(format, format === "md" ? `\n${setting}` : setting, [node("k", "Child", "c1")]);
    await press(root, key);
    expect(doc.byId.k).toBeUndefined();
    expect(doc.byId.c1.raw.startsWith("Child")).toBe(true);
    expect(doc.byId.c1.raw.endsWith(setting)).toBe(true);
    expectJoinedInCell(root, "Child", 0);
  });

  it("visible body after the hidden setting: the join and the caret are at the visible end", async () => {
    const root = load("md", "Cell\ntine.col-widths:: 0=120\nBody", [node("k", "Child", "c1")]);
    await press(root, key);
    // The merge re-places the hidden rows as every outline merge does (kernel `join`).
    expect(doc.byId.c1.raw).toBe("Cell\nBodyChild\ntine.col-widths:: 0=120");
    expectJoinedInCell(root, "Cell\nBodyChild", 9);
  });

  it("a metadata-only first child carrying a sheet setting is not joined and shows nothing", async () => {
    const root = load("md", "Cell", [node("k", "tine.col-widths:: 0=50", "c1")]);
    const before = snapshot();
    await press(root, key);
    expect(snapshot()).toBe(before);
    expect(editor(root).value()).not.toContain("tine.");
  });

  it("a metadata-only first child with only built-in hidden properties joins as an empty block", async () => {
    const root = load("md", "Cell", [node("k", "background-color:: red", "c1")]);
    await press(root, key);
    expect(doc.byId.k).toBeUndefined();
    expect(doc.byId.c1.raw).toBe("Cell");
    expectJoinedInCell(root, "Cell", 4);
  });

  it.each([["grid", "md"], ["table", "md"], ["board", "md"], ["grid", "org"], ["table", "org"], ["board", "org"]] as const)(
    "a hosted %s first in the cell is never joined (%s)", async (view, format) => {
      const root = load(format, "Cell", [
        node("hosted", `Hosted\n${prop(format, "tine.view", view)}`, "c1", ["hr"]),
        node("sib", "Sibling", "c1"),
      ], [node("hr", view === "grid" ? "" : "Row", "hosted", view === "grid" ? ["hc"] : []), ...(view === "grid" ? [node("hc", "Inner", "hr")] : [])]);
      const before = snapshot();
      if (key === "Delete") {
        keydown(await editCellEnd(root), "Delete");
      } else {
        // The hosted sheet's own title, edited at its start.
        const ed = await editCellEnd(root);
        keydown(ed, "ArrowDown");
        await tick();
        expect(editingId()).toBe("hosted");
        editor(root).select(0, 0);
        keydown(editor(root), "Backspace");
      }
      await tick();
      expect(snapshot()).toBe(before);
    });
});

describe("GH874 cell join: a folded cell", () => {
  it.each(["md", "org"] as const)("Delete at the end of a folded cell's text leaves its hidden child alone (%s)", async (format) => {
    const root = load(format, `Cell\n${prop(format, "collapsed", "true")}`, [node("k", "Hidden child", "c1")]);
    setDoc("byId", "c1", "collapsed", true);
    const before = snapshot();
    keydown(await editCellEnd(root), "Delete");
    await tick();
    expect(snapshot()).toBe(before);
    expect(editingId()).toBe("c1");
  });
});

describe("GH874 cell join: Undo", () => {
  it.each(keys)("one Undo after %s restores the child, with the caret in the cell", async (key) => {
    const root = load("md", "Cell", [node("k", "", "c1")]);
    await press(root, key);
    expect(doc.byId.k).toBeUndefined();
    expect(doc.byId.c1.raw).toBe("Cell");
    undo();
    await tick();
    expect(doc.byId.c1.children).toEqual(["k"]);
    expect(doc.byId.k.raw).toBe("");
    expect(editingId()).toBe(key === "Delete" ? "c1" : "k");
    expect(editingOwner()).toMatch(/^sheet:/);
    expect(root.querySelector('.sheet-cell[data-row="0"][data-col="0"]')!.contains(editor(root).element)).toBe(true);
    expect(editor(root).selection()).toEqual(key === "Delete" ? [4, 4] : [0, 0]);
  });
});

describe("GH874 cell join: positive controls", () => {
  it("Delete joins a first child with its own children; one Undo restores it", async () => {
    const root = load("md", "Cell", [node("k", "Child", "c1", ["g"]), node("later", "Later", "c1")], [node("g", "Grand", "k")]);
    await press(root, "Delete");
    expect(doc.byId.c1.raw).toBe("CellChild");
    expect(doc.byId.c1.children).toEqual(["later", "g"]);
    expectJoinedInCell(root, "CellChild", 4);
    undo();
    await tick();
    expect(doc.byId.c1.raw).toBe("Cell");
    expect(doc.byId.c1.children).toEqual(["k", "later"]);
    expect(doc.byId.k.children).toEqual(["g"]);
  });

  it("Backspace at the cell text's start and at a later nested block's start keep develop's answers", async () => {
    const root = load("md", "Cell", [node("k", "Child", "c1"), node("later", "Later", "c1")]);
    setCellSel({ gridId: "grid", row: 0, col: 0 });
    keydown(window, "Enter");
    await tick();
    editor(root).select(0, 0);
    keydown(editor(root), "Backspace");
    await tick();
    expect(doc.byId.c1.raw).toBe("Cell");
    expect(doc.byId.c1.children).toEqual(["k", "later"]);
    // A later nested block merges with the block before it, as in the outline.
    editor(root).select(4, 4);
    keydown(editor(root), "ArrowDown");
    await tick();
    editor(root).select(5, 5);
    keydown(editor(root), "ArrowDown");
    await tick();
    expect(editingId()).toBe("later");
    editor(root).select(0, 0);
    keydown(editor(root), "Backspace");
    await tick();
    expect(doc.byId.k.raw).toBe("ChildLater");
    expect(doc.byId.c1.raw).toBe("Cell");
  });
});
