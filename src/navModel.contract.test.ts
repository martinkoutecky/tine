// ADR 0034 — the SHARED nav-model contract. Sheets (cell selection) and split
// view (pane-select mode) are two implementations of one spatial navigation
// model: regions separated by boundaries, arrows alternating region → boundary
// → region, lateral sliding along a boundary, Enter/typing on a boundary
// materializing a new region, Escape descending a rung. This suite drives BOTH
// key handlers with the same key sequences in an equivalent 1×2 world and
// asserts the same invariants — if either surface drifts from the model, a
// test here fails naming the divergent surface.
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { initParser } from "./render/parse";
import { resetStore, setDoc, type FeedPage, type Node as StoreNode } from "./store";
import { endEdit } from "./editorController";
import { cellSel, handleCellSelectionKey, matrixForGrid, resetCellSelectionForTests, setCellSel } from "./sheet/selection";
import { handlePaneSelectKey } from "./keybindings";
import { enterPaneSelect, exitPaneSelect, paneSel } from "./paneSelect";
import { focusPane, layoutPaneIds, layoutRoot, resetPaneLayoutToSingle, splitRootAtEdge } from "./panes";
import { closeSwitcher, switcherOpen } from "./ui";
import type { PaneSnapshot } from "./router";

function ev(init: Partial<KeyboardEvent>): KeyboardEvent {
  return {
    key: "",
    code: "",
    shiftKey: false,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    isComposing: false,
    ...init,
  } as KeyboardEvent;
}

interface NavHarness {
  name: string;
  /** Build a 1×2 world — two regions side by side — and select the LEFT one. */
  setup(): void;
  teardown(): void;
  press(key: string): void;
  /** The current target, normalized to the shared model's vocabulary.
   *  Boundaries carry their line orientation so the lateral-slide invariant
   *  can tell "moved ALONG the same line" from "jumped to a crossing one". */
  selected(): { kind: "region" | "boundary"; key: string; orientation?: "h" | "v" } | null;
  /** For a selected boundary: the region it belongs to (was reached from /
   *  anchors on), or null for whole-line boundaries with no single owner. */
  boundaryOwner(): string | null;
  regionCount(): number;
}

const sheetHarness: NavHarness = {
  name: "sheet grid",
  setup() {
    const page: FeedPage = {
      name: "Sheet",
      kind: "page",
      title: "Sheet",
      preBlock: null,
      roots: ["grid"],
      format: "md",
      readOnly: false,
    };
    const node = (id: string, raw: string, parent: string | null, children: string[] = []): StoreNode => ({
      id,
      raw,
      collapsed: false,
      parent,
      page: "Sheet",
      children,
    });
    setDoc({
      byId: {
        grid: node("grid", "Grid\ntine.view:: grid", null, ["r1"]),
        r1: node("r1", "", "grid", ["c1", "c2"]),
        c1: node("c1", "A", "r1"),
        c2: node("c2", "B", "r1"),
      },
      pages: [page],
      feed: ["Sheet"],
      loaded: true,
    });
    setCellSel({ kind: "cell", gridId: "grid", row: 0, col: 0 });
  },
  teardown() {
    endEdit("blur");
    resetCellSelectionForTests();
    resetStore();
  },
  press(key) {
    handleCellSelectionKey(ev({ key }));
  },
  selected() {
    const sel = cellSel();
    if (!sel) return null;
    if (sel.kind === "row-seam") return { kind: "boundary", key: `row-seam:${sel.at}:${sel.anchor.col}`, orientation: "h" };
    if (sel.kind === "col-seam") return { kind: "boundary", key: `col-seam:${sel.at}:${sel.anchor.row}`, orientation: "v" };
    if (sel.kind === "range") return { kind: "region", key: `range:${sel.focus.row}:${sel.focus.col}` };
    return { kind: "region", key: `cell:${sel.row}:${sel.col}` };
  },
  boundaryOwner() {
    const sel = cellSel();
    // A seam's anchor records the cell it was reached from — that's the
    // boundary's owning region in the shared model.
    if (sel?.kind === "row-seam") return `cell:${sel.at}:${sel.anchor.col}`;
    if (sel?.kind === "col-seam") return `cell:${sel.anchor.row}:${sel.at}`;
    return null;
  },
  regionCount() {
    const matrix = matrixForGrid("grid");
    return matrix.rows === 0 ? 0 : matrix.rows * matrix.cols;
  },
};

const paneSnapshot = (name: string): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

const paneHarness: NavHarness = {
  name: "split view",
  setup() {
    resetPaneLayoutToSingle(paneSnapshot("Source"));
    splitRootAtEdge("right", "main"); // row [main, extra]
    focusPane("main");
    enterPaneSelect("main");
  },
  teardown() {
    exitPaneSelect();
    if (switcherOpen()) closeSwitcher();
    resetPaneLayoutToSingle(paneSnapshot("Source"));
  },
  press(key) {
    handlePaneSelectKey(ev({ key }));
  },
  selected() {
    const sel = paneSel();
    if (!sel) return null;
    const sideOrientation = (side: string): "h" | "v" => (side === "top" || side === "bottom" ? "h" : "v");
    if (sel.kind === "pane") return { kind: "region", key: `pane:${sel.paneId}` };
    if (sel.kind === "seam") {
      // A row split's seam is a vertical line, a col split's horizontal.
      let node = layoutRoot();
      for (const idx of sel.path) {
        if (node.kind !== "split") break;
        node = node.children[idx];
      }
      const orientation = node.kind === "split" && node.dir === "row" ? "v" : "h";
      return { kind: "boundary", key: `seam:${sel.path.join(".")}`, orientation };
    }
    if (sel.kind === "pane-edge")
      return { kind: "boundary", key: `pane-edge:${sel.paneId}:${sel.side}`, orientation: sideOrientation(sel.side) };
    return { kind: "boundary", key: `edge:${sel.side}`, orientation: sideOrientation(sel.side) };
  },
  boundaryOwner() {
    const sel = paneSel();
    return sel?.kind === "pane-edge" ? `pane:${sel.paneId}` : null;
  },
  regionCount() {
    return layoutPaneIds().length;
  },
};

const harnesses = [sheetHarness, paneHarness];

beforeAll(() => initParser());

describe.each(harnesses)("nav-model contract — $name", (h) => {
  afterEach(() => h.teardown());

  it("an arrow selects the boundary first, the region beyond it second", () => {
    h.setup();
    const start = h.selected();
    expect(start?.kind).toBe("region");

    h.press("ArrowRight");
    expect(h.selected()?.kind).toBe("boundary");

    h.press("ArrowRight");
    const landed = h.selected();
    expect(landed?.kind).toBe("region");
    expect(landed?.key).not.toBe(start?.key);
  });

  it("a perpendicular arrow on a boundary slides ALONG it to the NEIGHBOR's boundary", () => {
    h.setup();
    const left = h.selected();
    h.press("ArrowRight");
    h.press("ArrowRight"); // land on the right region
    const right = h.selected();
    expect(right?.kind).toBe("region");
    expect(right?.key).not.toBe(left?.key);

    h.press("ArrowUp"); // the right region's top boundary
    const before = h.selected();
    expect(before?.kind).toBe("boundary");
    expect(h.boundaryOwner()).toBe(right?.key);

    h.press("ArrowLeft"); // slide along the top line to the LEFT region's boundary
    const after = h.selected();
    expect(after?.kind).toBe("boundary");
    expect(after?.orientation).toBe(before?.orientation); // same line family
    expect(h.boundaryOwner()).toBe(left?.key); // owned by the neighbor we slid to
  });

  it("Enter on a boundary materializes a new region there", () => {
    h.setup();
    h.press("ArrowUp");
    expect(h.selected()?.kind).toBe("boundary");
    const before = h.regionCount();

    h.press("Enter");
    expect(h.regionCount()).toBeGreaterThan(before);
  });

  it("typing on a boundary also materializes a new region (create-and-type)", () => {
    h.setup();
    h.press("ArrowUp");
    expect(h.selected()?.kind).toBe("boundary");
    const before = h.regionCount();

    h.press("z");
    expect(h.regionCount()).toBeGreaterThan(before);
  });

  it("Escape descends out of the selection surface", () => {
    h.setup();
    h.press("Escape");
    expect(h.selected()).toBeNull();
  });
});
