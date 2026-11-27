// GH #874: in a grid cell, Backspace at the start of the cell's first child and Delete at the
// end of the cell's own text remove that child when it is empty (whitespace-only text, no
// children), in both editors and both formats. Nothing else is removed or merged: every other
// first child is refused, the page unchanged and nothing queued for saving.
import { maybeEditorIn, type EditorDriver } from "../tests/editorDriver";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { isDirty, resetStore, undo } from "../document";
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

/** A grid whose top-left cell `c1` holds `cellRaw`; its first child `k` holds `kRaw`, then `later`. */
function load(format: Format, cellRaw: string, kRaw: string, opts: { kChildren?: boolean; later?: boolean } = {}) {
  const page: FeedPage = { name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["grid"], format, readOnly: false, guide: false };
  const byId: Record<string, StoreNode> = {
    grid: node("grid", `Grid\n${prop(format, "tine.view", "grid")}`, null, ["r1"]),
    r1: node("r1", "", "grid", ["c1", "c2"]),
    c1: node("c1", cellRaw, "r1", opts.later ? ["k", "later"] : ["k"]),
    c2: node("c2", "Beta", "r1"),
    k: node("k", kRaw, "c1", opts.kChildren ? ["g"] : []),
  };
  if (opts.kChildren) byId.g = node("g", "Grand", "k");
  if (opts.later) byId.later = node("later", "Later", "c1");
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

/** Edit the first child `k`, caret at its start (Down from the cell text's end). */
async function editFirstChildStart(root: HTMLElement): Promise<EditorDriver> {
  const ed = await editCellEnd(root);
  keydown(ed, "ArrowDown");
  await tick();
  expect(editingId()).toBe("k");
  const child = editor(root);
  child.select(0, 0);
  return child;
}

/** The page as the serializer sees it: every block's text, children and fold. */
const snapshot = () => JSON.stringify(Object.keys(doc.byId).sort().map(id => [id, doc.byId[id].raw, doc.byId[id].children, doc.byId[id].collapsed]));

const keys = ["Backspace", "Delete"] as const;
type Key = typeof keys[number];

async function press(root: HTMLElement, key: Key): Promise<void> {
  const ed = key === "Delete" ? await editCellEnd(root) : await editFirstChildStart(root);
  keydown(ed, key);
  await tick();
}

function expectInCell(root: HTMLElement, visible: string, caret: number) {
  expect(editingId()).toBe("c1");
  expect(editingOwner()).toMatch(/^sheet:/);
  expect(root.querySelector('.sheet-cell[data-row="0"][data-col="0"]')!.contains(editor(root).element)).toBe(true);
  expect(editor(root).value()).toBe(visible);
  expect(editor(root).selection()).toEqual([caret, caret]);
}

const formats = ["md", "org"] as const;
const cases = keys.flatMap(key => formats.map(format => [key, format] as const));

describe.each(cases)("GH874 %s at a grid cell's edge (%s)", (key, format) => {
  it.each([["empty", ""], ["whitespace-only", "  "]])("removes an %s first child; the cell's text stays as typed", async (_, kRaw) => {
    const root = load(format, "Cell", kRaw);
    await press(root, key);
    expect(doc.byId.k).toBeUndefined();
    expect(doc.byId.c1.children).toEqual([]);
    expect(doc.byId.c1.raw).toBe("Cell");
    expect(doc.byId.r1.children).toEqual(["c1", "c2"]);
    expectInCell(root, "Cell", 4);
    expect(isDirty("Sheet")).toBe(true);
  });

  it("with a hidden sheet setting on the cell, the caret ends at the cell's visible text and the setting is untouched", async () => {
    const setting = prop(format, "tine.col-widths", "0=120");
    const root = load(format, `Cell\n${setting}`, "");
    await press(root, key);
    expect(doc.byId.k).toBeUndefined();
    expect(doc.byId.c1.raw).toBe(`Cell\n${setting}`);
    expectInCell(root, "Cell", 4);
  });

  it("one Undo brings the empty child back", async () => {
    const root = load(format, "Cell", "", { later: true });
    await press(root, key);
    expect(doc.byId.c1.children).toEqual(["later"]);
    undo();
    await tick();
    expect(doc.byId.c1.children).toEqual(["k", "later"]);
    expect(doc.byId.k.raw).toBe("");
    expect(doc.byId.c1.raw).toBe("Cell");
  });
});

/** Inputs whose first child is not empty: [label, format, cell raw, first child raw]. */
const refused: [string, Format, string, string][] = [
  ["hidden tine.col-widths on the cell", "md", "Cell\ntine.col-widths:: 0=120", "Child"],
  ["hidden tine.col-widths on the cell", "org", "Cell\n:PROPERTIES:\n:tine.col-widths: 0=120\n:END:", "Child"],
  ["owner:: Alice on the cell", "md", "Cell\nowner:: Alice", "Child"],
  ["a child `:: 0=20`", "md", "Cell", ":: 0=20"],
  ["a tine.view child", "md", "Cell", "Hosted\ntine.view:: table"],
  ["a tine.view child", "org", "Cell", "Hosted\n:PROPERTIES:\n:tine.view: board\n:END:"],
  ["a drawer-only child", "org", "Cell", ":PROPERTIES:\n:owner: Alice\n:END:"],
  ["a configured-hidden key", "md", "Cell", "secret:: x"],
  ["a calculator child", "md", "Cell", "```calc\n1+1\n```"],
  ["a calculator child after a hidden property", "md", "Cell", "tine.col-widths:: 0=20\n```calc\n1+1\n```"],
  ["a calculator cell", "md", "```calc\n1+1\n```", "Child"],
  ["a comment child", "md", "Cell", "Note\nls-type:: annotation"],
  ["seam property tine.view + `:: grid`", "md", "Cell\ntine.view", ":: grid"],
  ["seam property tine.col-widths + `:: 0=20`", "md", "Cell\ntine.col-widths", ":: 0=20"],
  ["Org drawer seam", "org", "Cell\n:PROPER", "TIES:\n:owner: Alice\n:END:"],
  ["a LOGBOOK child", "org", "Cell", ":LOGBOOK:\nCLOCK: [2026-11-27 Fri 10:00]--[2026-11-27 Fri 11:00] =>  1:00\n:END:"],
  ["a LOGBOOK child", "md", "Cell", ":LOGBOOK:\nCLOCK: [2026-11-27 Fri 10:00]--[2026-11-27 Fri 11:00] =>  1:00\n:END:"],
  ["a CLOSED child", "org", "Cell", "CLOSED: [2026-11-27 Fri]"],
  ["a SCHEDULED child", "md", "Cell", "SCHEDULED: <2026-11-27 Fri>"],
  ["a DEADLINE child", "org", "Cell", "DEADLINE: <2026-11-27 Fri>"],
  ["a TODO child", "md", "Cell", "TODO Child"],
  ["a heading child", "md", "Cell", "## Child"],
  ["#+BEGIN_ + QUOTE", "org", "#+BEGIN_", "QUOTE"],
  ["#+BEGIN_ + QUOTE", "md", "#+BEGIN_", "QUOTE"],
  ["an HTML comment seam", "md", "<!-", "-Child-->"],
  ["#+CAP + TION", "org", "#+CAP", "TION: Child"],
  ["#one + two", "md", "#one", "two"],
  ["an Org tag on the cell", "org", "Cell :tag:", "Child"],
  ["> + >", "org", ">", ">"],
  ["Cell + Child", "md", "Cell", "Child"],
  ["Cell + Child", "org", "Cell", "Child"],
  ["a child with trailing spaces", "md", "Cell", "Child  "],
  ["a hidden collapsed:: child", "md", "Cell", "collapsed:: true"],
  ["a hidden id:: child", "md", "Cell", "id:: 6566c5d3-1b2c-4d5e-8f90-123456789abc"],
  ["a hidden tine.view:: grid child", "md", "Cell", "tine.view:: grid"],
  ["a hidden id child", "org", "Cell", ":PROPERTIES:\n:id: 6566c5d3-1b2c-4d5e-8f90-123456789abc\n:END:"],
];

describe.each(keys)("GH874 %s refuses a first child that is not empty", (key) => {
  it.each(refused)("%s (%s): nothing changes and nothing is queued", async (_, format, cellRaw, kRaw) => {
    const root = load(format, cellRaw, kRaw);
    const before = snapshot();
    await press(root, key);
    expect(snapshot()).toBe(before);
    expect(isDirty("Sheet")).toBe(false);
    expect(root.querySelector('.sheet-cell[data-row="0"][data-col="0"]')!.contains(editor(root).element)).toBe(true);
  });

  it.each(formats)("an empty child with children (%s)", async (format) => {
    const root = load(format, "Cell", "", { kChildren: true });
    const before = snapshot();
    await press(root, key);
    expect(snapshot()).toBe(before);
    expect(isDirty("Sheet")).toBe(false);
  });
});

/** A cell that hosts a compact grid, table or board: its children are that sheet's rows or
 * cards, not the cell's outline, so Delete at the cell text's end removes none of them
 * (reviewer's 12 combinations: view x editor run x format). */
const hostedViews = ["grid", "table", "board"] as const;
describe.each(hostedViews.flatMap(view => formats.map(format => [view, format] as const)))("GH874 Delete on a cell hosting a %s (%s)", (view, format) => {
  it.each([["a blank first row", ""], ["a whitespace-only first row", "  "]])("keeps %s and queues nothing", async (_, kRaw) => {
    const root = load(format, `Cell\n${prop(format, "tine.view", view)}`, kRaw, { later: true });
    const before = snapshot();
    keydown(await editCellEnd(root), "Delete");
    await tick();
    expect(snapshot()).toBe(before);
    expect(doc.byId.k).toBeDefined();
    expect(isDirty("Sheet")).toBe(false);
    expect(editingId()).toBe("c1");
  });
});

describe("GH874 Delete on a folded cell", () => {
  it.each(formats)("leaves its hidden empty child alone (%s)", async (format) => {
    const root = load(format, `Cell\n${prop(format, "collapsed", "true")}`, "");
    setDoc("byId", "c1", "collapsed", true);
    const before = snapshot();
    keydown(await editCellEnd(root), "Delete");
    await tick();
    expect(snapshot()).toBe(before);
    expect(isDirty("Sheet")).toBe(false);
    expect(editingId()).toBe("c1");
  });
});

describe("GH874 controls: develop's answers elsewhere in the cell", () => {
  it.each(formats)("Backspace at the cell text's start does nothing (%s)", async (format) => {
    const root = load(format, "Cell", "");
    const before = snapshot();
    const ed = await editCellEnd(root);
    ed.select(0, 0);
    keydown(ed, "Backspace");
    await tick();
    expect(snapshot()).toBe(before);
    expect(isDirty("Sheet")).toBe(false);
  });

  it.each(formats)("Backspace at a later nested block's start merges it into the block before (%s)", async (format) => {
    const root = load(format, "Cell", "Child", { later: true });
    const ed = await editFirstChildStart(root);
    ed.select(5, 5);
    keydown(ed, "ArrowDown");
    await tick();
    expect(editingId()).toBe("later");
    editor(root).select(0, 0);
    keydown(editor(root), "Backspace");
    await tick();
    expect(doc.byId.k.raw).toBe("ChildLater");
    expect(doc.byId.c1.children).toEqual(["k"]);
    expect(doc.byId.c1.raw).toBe("Cell");
  });

  it.each(formats)("Backspace at an empty later nested block's start merges it away, as in the outline (%s)", async (format) => {
    const root = load(format, "Cell", "Child", { later: true });
    setDoc("byId", "later", "raw", "");
    setDoc("byId", "c1", "children", ["k", "later", "last"]);
    setDoc("byId", "last", node("last", "Last", "c1"));
    const ed = await editFirstChildStart(root);
    ed.select(5, 5);
    keydown(ed, "ArrowDown");
    await tick();
    expect(editingId()).toBe("later");
    keydown(editor(root), "Backspace");
    await tick();
    expect(doc.byId.later).toBeUndefined();
    expect(doc.byId.k.raw).toBe("Child");
    expect(doc.byId.c1.children).toEqual(["k", "last"]);
  });

  it.each(formats)("Delete at the end of a nested block merges the next one into it (%s)", async (format) => {
    const root = load(format, "Cell", "Child", { later: true });
    const ed = await editFirstChildStart(root);
    ed.select(5, 5);
    keydown(ed, "Delete");
    await tick();
    expect(doc.byId.k.raw).toBe("ChildLater");
    expect(doc.byId.c1.raw).toBe("Cell");
  });
});
