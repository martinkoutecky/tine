import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { doc, type Node, type FeedPage, setDoc } from "../document/model";
import { openJournals } from "../router";
import { resetCellSelectionForTests } from "../sheet/selection";
import { editingId } from "../editorController";

beforeAll(async () => { await initParser(); });
afterEach(() => {
  resetCellSelectionForTests();
  resetStore();
  openJournals({ inPlace: true });
  document.body.innerHTML = "";
});

const PAGE = "Sheet";
const n = (id: string, raw: string, parent: string | null, children: string[] = []): Node =>
  ({ id, raw, collapsed: false, parent, page: PAGE, children });

function load(byId: Record<string, Node>) {
  setDoc({
    byId,
    pages: [{ name: PAGE, kind: "page", title: PAGE, preBlock: null, roots: ["grid"], format: "md", readOnly: false, guide: false } as FeedPage],
    feed: [PAGE], loaded: true,
  });
}

function mount() {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <Block id="grid" />, root);
  return { root, dispose };
}

const cell = (root: HTMLElement, id: string) => root.querySelector<HTMLElement>(`.sheet-cell[data-block-id="${id}"]`)!;

// GH #782: a grid cell holds ordinary blocks (ADR 0031), so a block inside a cell must look and behave as it
// does on a page: task marker and checkbox, priority, SCHEDULED, properties, any depth, a hosted sub-grid.
function taskCell() {
  load({
    grid: n("grid", "Grid\ntine.view:: grid", null, ["row"]),
    row: n("row", "", "grid", ["t", "plain"]),
    plain: n("plain", "Plain cell", "row"),
    t: n("t", "TODO Task cell\nSCHEDULED: <2027-06-01 Tue>\nnote:: kept", "row", ["child", "host"]),
    child: n("child", "TODO [#A] Child task\nSCHEDULED: <2027-06-02 Wed>\nweight:: heavy", "t", ["grand"]),
    grand: n("grand", "DONE Grandchild task", "child", ["deep"]),
    deep: n("deep", "Deepest line", "grand"),
    host: n("host", "Inner grid\ntine.view:: grid", "t", ["hrow"]),
    hrow: n("hrow", "", "host", ["ha", "hb"]),
    ha: n("ha", "inner A", "hrow"),
    hb: n("hb", "inner B", "hrow"),
  });
}

describe("a block inside a grid cell is drawn as the page draws it (GH #782)", () => {
  it("shows task markers, checkboxes, priority, planning dates and properties at every level", () => {
    taskCell();
    const { root, dispose } = mount();
    try {
      const c = cell(root, "t");
      expect([...c.querySelectorAll(".block-marker")].map((m) => m.textContent)).toEqual(["TODO", "TODO", "DONE"]);
      const boxes = [...c.querySelectorAll(".block-task-checkbox")];
      expect(boxes.map((b) => b.classList.contains("checked"))).toEqual([false, false, true]);
      expect([...c.querySelectorAll(".block-priority")].map((p) => p.textContent)).toEqual(["[#A]"]);
      expect(c.querySelectorAll(".date-chip.scheduled")).toHaveLength(2);
      const props = [...c.querySelectorAll(".block-property")].map((p) => p.textContent);
      expect(props.some((p) => /note.*kept/.test(p ?? ""))).toBe(true);
      expect(props.some((p) => /weight.*heavy/.test(p ?? ""))).toBe(true);
      // The marker words are chips, not repeated inside the body text.
      expect(c.querySelector('.block-content[data-rendered-block="child"]')?.textContent).toContain("Child task");
      // Four levels deep, each a child line of the one above, each with the page's bullet.
      expect(c.querySelectorAll(".sheet-nested-line")).toHaveLength(4); // child, grand, deep, host
      expect(c.querySelector('[data-rendered-block="deep"]')?.closest(".sheet-nested-lines")?.parentElement?.closest(".sheet-nested-lines")).not.toBeNull();
      expect(c.querySelectorAll(".sheet-nested-bullet .bullet")).toHaveLength(4);
      // A hosted sub-grid still renders inside the cell.
      expect(c.querySelector('.sheet-grid[data-sheet-grid-id="host"]')?.textContent).toContain("inner B");
      // tine.* properties of the grid host stay hidden, as on a page.
      expect(c.textContent).not.toContain("tine.view");
    } finally { dispose(); }
  });

  it("ticks a task inside a cell from its checkbox without entering the editor", () => {
    taskCell();
    const { root, dispose } = mount();
    try {
      const box = cell(root, "t").querySelectorAll<HTMLElement>(".block-task-checkbox")[1];
      box.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
      expect(doc.byId.child.raw.startsWith("DONE [#A] Child task")).toBe(true);
      expect(editingId()).toBeNull();
    } finally { dispose(); }
  });

  it("keeps a cell's own click behavior: a press on the body does not start editing", () => {
    taskCell();
    const { root, dispose } = mount();
    try {
      const body = cell(root, "plain").querySelector<HTMLElement>(".block-content")!;
      body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, clientX: 4, clientY: 4 }));
      expect(editingId()).toBeNull();
    } finally { dispose(); }
  });

  it("keeps the render cost of a 50-row sheet bounded (parse calls and nodes)", () => {
    const byId: Record<string, Node> = { grid: n("grid", "Grid\ntine.view:: grid", null, []) };
    for (let r = 0; r < 50; r++) {
      const rowId = `r${r}`;
      byId.grid.children.push(rowId);
      byId[rowId] = n(rowId, "", "grid", [0, 1, 2].map((c) => `c${r}_${c}`));
      for (let c = 0; c < 3; c++) {
        const raw = c === 0 && r % 5 === 0 ? `TODO Task ${r}\nnote:: v${r}` : `Cell ${r}/${c}`;
        byId[`c${r}_${c}`] = n(`c${r}_${c}`, raw, rowId);
      }
    }
    load(byId);
    const w = window as unknown as { __tineParseStats?: { calls: number; hits: number; misses: number } };
    delete w.__tineParseStats;
    const { root, dispose } = mount();
    try {
      expect(root.querySelectorAll(".sheet-cell .sheet-cell-body").length).toBe(150);
      const stats = w.__tineParseStats ?? { calls: 0, hits: 0, misses: 0 };
      // Measured on this fixture (150 cells, 10 of them tasks with a property). Before the page's body
      // renderer: 360 calls, 152 cold parses, 622 nodes. After: 600 calls, 162 cold parses, 992 nodes; the
      // extra calls are warm cache hits (each cell asks once for its property rows), the extra cold parses
      // are the 10 task cells (their full text and their stripped body). A new full parse per cell would
      // show here as ~300 cold parses.
      console.log(`cells-render 50x3 parse stats ${JSON.stringify(stats)} nodes ${root.querySelectorAll("*").length}`);
      expect(stats.misses).toBeLessThanOrEqual(170);
      expect(stats.calls).toBeLessThanOrEqual(640);
      expect(root.querySelectorAll("*").length).toBeLessThanOrEqual(1100);
    } finally { dispose(); }
  });
});
