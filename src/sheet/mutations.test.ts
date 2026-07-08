import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import {
  blockProperty,
  blockIsGridView,
  blockSubtreeMarkdown,
  doc,
  loadSingle,
  pageToDto,
  resetStore,
  setDoc,
  undo,
} from "../store";
import type { BlockDto, PageDto } from "../types";
import {
  deleteColumn,
  deleteRow,
  copySheetSelection,
  appendSheetCellChild,
  clearSheetSelection,
  fillSheetSelection,
  insertColumn,
  insertRow,
  materializeCell,
  moveSheetSelection,
  pasteStructuralSheetSelection,
  pasteTextIntoSheetSelection,
  sheetSelectionText,
  structuralSheetPasteNode,
  deleteRows,
  deleteColumns,
} from "./mutations";
import { parseDelimitedText } from "./tsv";

let counter = 0;
function blk(raw: string, children: BlockDto[] = []): BlockDto {
  return { id: `m${counter++}`, raw, collapsed: false, children };
}

function loadGrid(raw = "Grid\ntine.view:: grid"): string {
  const grid = blk(raw, [
    blk("", [blk("A"), blk("B"), blk("C")]),
    blk("", [blk("D")]),
    blk("", []),
  ]);
  grid.properties = [["tine.view", "grid"]];
  const widths = /(?:^|\n)tine\.col-widths:: ?([^\n]*)/.exec(raw)?.[1];
  if (widths != null) grid.properties.push(["tine.col-widths", widths]);
  const aggregates = /(?:^|\n)tine\.col-aggregates:: ?([^\n]*)/.exec(raw)?.[1];
  if (aggregates != null) grid.properties.push(["tine.col-aggregates", aggregates]);
  const dto: PageDto = { name: "Sheet", kind: "page", title: "Sheet", pre_block: null, blocks: [grid] };
  loadSingle(dto);
  return grid.id;
}

function rows(gridId: string): string[] {
  return [...doc.byId[gridId].children];
}

function rowCells(rowId: string): string[] {
  return doc.byId[rowId].children.map((id) => doc.byId[id].raw);
}

function gridShape(gridId: string): string[][] {
  return rows(gridId).map((rowId) => rowCells(rowId));
}

function cellId(gridId: string, row: number, col: number): string | null {
  const rowId = rows(gridId)[row];
  return rowId ? (doc.byId[rowId].children[col] ?? null) : null;
}

function loadStructuralPasteDoc() {
  setDoc({
    byId: {
      src: { id: "src", raw: "Source\ntine.view:: grid", collapsed: false, parent: null, page: "Sheet", children: ["sr1", "sr2"] },
      sr1: { id: "sr1", raw: "", collapsed: false, parent: "src", page: "Sheet", children: ["s11", "s12"] },
      s11: { id: "s11", raw: "A\nprop:: one", collapsed: false, parent: "sr1", page: "Sheet", children: ["s11c"] },
      s11c: { id: "s11c", raw: "A child", collapsed: false, parent: "s11", page: "Sheet", children: [] },
      s12: { id: "s12", raw: "B", collapsed: false, parent: "sr1", page: "Sheet", children: [] },
      sr2: { id: "sr2", raw: "", collapsed: false, parent: "src", page: "Sheet", children: ["s21"] },
      s21: { id: "s21", raw: "C", collapsed: false, parent: "sr2", page: "Sheet", children: [] },
      dst: { id: "dst", raw: "Target grid\ntine.view:: grid", collapsed: false, parent: null, page: "Sheet", children: ["dr1"] },
      dr1: { id: "dr1", raw: "", collapsed: false, parent: "dst", page: "Sheet", children: ["target"] },
      target: {
        id: "target",
        raw: "Target\ntine.view:: grid\ntine.header:: true\ntine.col-widths:: 0=100\ntine.col-aggregates:: 0=sum",
        collapsed: false,
        parent: "dr1",
        page: "Sheet",
        children: ["er1"],
      },
      er1: { id: "er1", raw: "", collapsed: false, parent: "target", page: "Sheet", children: ["ec1"] },
      ec1: { id: "ec1", raw: "Existing", collapsed: false, parent: "er1", page: "Sheet", children: [] },
    },
    pages: [{ name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["src", "dst"], format: "md", readOnly: false }],
    feed: ["Sheet"],
    loaded: true,
  });
}

beforeAll(() => initParser());

beforeEach(() => {
  counter = 0;
  resetStore();
});

describe("sheet structural mutations", () => {
  it("inserts an empty row and one undo fully reverts it", () => {
    const gridId = loadGrid();
    const inserted = insertRow(gridId, 1);

    expect(inserted).toBeTruthy();
    expect(rows(gridId)).toHaveLength(4);
    expect(doc.byId[inserted!].raw).toBe("");
    expect(doc.byId[inserted!].children).toEqual([]);
    expect(blockSubtreeMarkdown(gridId, 0, true)).toContain("\t-");

    undo();
    expect(gridShape(gridId)).toEqual([["A", "B", "C"], ["D"], []]);
  });

  it("deletes a row subtree and one undo restores it", () => {
    const gridId = loadGrid();
    const deletedRow = rows(gridId)[0];
    const deletedCell = doc.byId[deletedRow].children[1];

    deleteRow(gridId, 0);

    expect(doc.byId[deletedRow]).toBeUndefined();
    expect(doc.byId[deletedCell]).toBeUndefined();
    expect(gridShape(gridId)).toEqual([["D"], []]);

    undo();
    expect(gridShape(gridId)).toEqual([["A", "B", "C"], ["D"], []]);
  });

  it("inserts a column across ragged rows and shifts col-width keys in the same undo unit", () => {
    const gridId = loadGrid("Grid\ntine.view:: grid\ntine.col-widths:: 0=120;2=88\ntine.col-aggregates:: 0=sum;2=average");

    insertColumn(gridId, 1);

    expect(gridShape(gridId)).toEqual([["A", "", "B", "C"], ["D", ""], []]);
    expect(blockProperty(gridId, "tine.col-widths")).toBe("0=120;3=88");
    expect(blockProperty(gridId, "tine.col-aggregates")).toBe("0=sum;3=average");

    undo();
    expect(gridShape(gridId)).toEqual([["A", "B", "C"], ["D"], []]);
    expect(blockProperty(gridId, "tine.col-widths")).toBe("0=120;2=88");
    expect(blockProperty(gridId, "tine.col-aggregates")).toBe("0=sum;2=average");
  });

  it("deletes a column across ragged rows and rewrites col-width keys in the same undo unit", () => {
    const gridId = loadGrid("Grid\ntine.view:: grid\ntine.col-widths:: 0=120;1=77;2=88\ntine.col-aggregates:: 0=sum;1=max;2=average");

    deleteColumn(gridId, 1);

    expect(gridShape(gridId)).toEqual([["A", "C"], ["D"], []]);
    expect(blockProperty(gridId, "tine.col-widths")).toBe("0=120;1=88");
    expect(blockProperty(gridId, "tine.col-aggregates")).toBe("0=sum;1=average");

    undo();
    expect(gridShape(gridId)).toEqual([["A", "B", "C"], ["D"], []]);
    expect(blockProperty(gridId, "tine.col-widths")).toBe("0=120;1=77;2=88");
    expect(blockProperty(gridId, "tine.col-aggregates")).toBe("0=sum;1=max;2=average");
  });

  it("materializes a hole by appending exactly the missing cells", () => {
    const gridId = loadGrid();
    const rowId = rows(gridId)[1];

    const cellId = materializeCell(gridId, 1, 3);

    expect(cellId).toBe(doc.byId[rowId].children[3]);
    expect(rowCells(rowId)).toEqual(["D", "", "", ""]);
    expect(blockSubtreeMarkdown(gridId, 0, true)).toContain("\t\t- D");

    undo();
    expect(rowCells(rowId)).toEqual(["D"]);
  });

  it("no-ops cleanly on invalid coordinates", () => {
    const gridId = loadGrid();
    const before = pageToDto("Sheet");

    expect(insertRow(gridId, -1)).toBeNull();
    deleteRow(gridId, 99);
    insertColumn(gridId, 99);
    deleteColumn(gridId, -1);
    expect(materializeCell(gridId, 99, 0)).toBeNull();

    expect(pageToDto("Sheet")).toEqual(before);
  });

  it("moves a cell down into a trailing hole and one undo removes the materialized target", () => {
    const gridId = loadGrid();
    const before = pageToDto("Sheet");

    const next = moveSheetSelection({ kind: "cell", gridId, row: 0, col: 1 }, "down");

    expect(next).toEqual({ kind: "cell", gridId, row: 1, col: 1 });
    expect(gridShape(gridId)).toEqual([["A", "", "C"], ["D", "B"], []]);

    undo();
    expect(pageToDto("Sheet")).toEqual(before);
  });

  it("fills down from the top row raw, strips id::, and one undo removes target holes", () => {
    const gridId = loadGrid();
    const source = cellId(gridId, 0, 1)!;
    setDoc("byId", source, "raw", "B\nid:: hidden");
    const before = pageToDto("Sheet");

    fillSheetSelection(
      { kind: "range", gridId, anchor: { row: 0, col: 1 }, focus: { row: 1, col: 1 } },
      "down"
    );

    const target = cellId(gridId, 1, 1)!;
    expect(doc.byId[target].raw).toBe("B");
    expect(rowCells(rows(gridId)[1])).toEqual(["D", "B"]);

    undo();
    expect(pageToDto("Sheet")).toEqual(before);
  });

  it("pastes TSV from the anchor, grows rows/trailing cells, round-trips, and undoes fully", () => {
    const gridId = loadGrid();
    const before = pageToDto("Sheet");

    const next = pasteTextIntoSheetSelection({ kind: "cell", gridId, row: 2, col: 0 }, "X\tY\nZ\tW");

    expect(next).toEqual({
      kind: "range",
      gridId,
      anchor: { row: 2, col: 0 },
      focus: { row: 3, col: 1 },
    });
    expect(gridShape(gridId)).toEqual([["A", "B", "C"], ["D"], ["X", "Y"], ["Z", "W"]]);
    expect(pageToDto("Sheet")?.blocks[0].children[3].children.map((c) => c.raw)).toEqual(["Z", "W"]);

    undo();
    expect(pageToDto("Sheet")).toEqual(before);
  });

  it("pastes indented text as nested blocks under the anchor cell", () => {
    const gridId = loadGrid();
    const before = pageToDto("Sheet");

    const next = pasteTextIntoSheetSelection({ kind: "cell", gridId, row: 0, col: 0 }, "parent\n  child");

    expect(next).toEqual({ kind: "cell", gridId, row: 0, col: 0 });
    const anchor = cellId(gridId, 0, 0)!;
    const child = doc.byId[anchor].children[0];
    expect(doc.byId[child].raw).toBe("parent");
    expect(doc.byId[doc.byId[child].children[0]].raw).toBe("child");
    expect(blockSubtreeMarkdown(anchor, 0, true)).toContain("\t- child");

    undo();
    expect(pageToDto("Sheet")).toEqual(before);
  });

  it("copies a grid structurally and pastes it as a hosted child grid after auto-wrapping a compact target", async () => {
    loadStructuralPasteDoc();
    const before = pageToDto("Sheet");

    await copySheetSelection({
      kind: "range",
      gridId: "src",
      anchor: { row: 0, col: 0 },
      focus: { row: 1, col: 1 },
    });
    const next = pasteStructuralSheetSelection({ kind: "cell", gridId: "dst", row: 0, col: 0 }, "A\tB\nC\t");

    expect(next).toEqual({ kind: "cell", gridId: doc.byId.target.children[1], row: 0, col: 0 });
    expect(doc.byId.target.raw).toBe("Target");
    expect(doc.byId.target.children).toHaveLength(2);
    const [existingHost, pastedHost] = doc.byId.target.children;
    expect(doc.byId[existingHost].raw).toBe(
      "tine.view:: grid\ntine.header:: true\ntine.col-widths:: 0=100\ntine.col-aggregates:: 0=sum"
    );
    expect(doc.byId[existingHost].children).toEqual(["er1"]);
    expect(doc.byId.er1.parent).toBe(existingHost);
    expect(blockIsGridView(existingHost)).toBe(true);
    expect(blockIsGridView(pastedHost)).toBe(true);

    const [pr1, pr2] = doc.byId[pastedHost].children;
    expect(doc.byId[pr1].raw).toBe("");
    expect(doc.byId[pr1].children.map((id) => doc.byId[id].raw)).toEqual(["A\nprop:: one", "B"]);
    const copiedA = doc.byId[pr1].children[0];
    expect(doc.byId[doc.byId[copiedA].children[0]].raw).toBe("A child");
    expect(doc.byId[pr2].raw).toBe("");
    expect(doc.byId[pr2].children.map((id) => doc.byId[id].raw)).toEqual(["C", ""]);

    undo();
    expect(pageToDto("Sheet")).toEqual(before);
  });

  it("hands back a subgrid node for its own multi-cell copy, but not for other text or a single cell", async () => {
    loadStructuralPasteDoc();
    const sel = { kind: "range", gridId: "src", anchor: { row: 0, col: 0 }, focus: { row: 1, col: 1 } } as const;
    const { text } = sheetSelectionText(sel);
    await copySheetSelection(sel);

    const node = structuralSheetPasteNode(text);
    expect(node).not.toBeNull();
    expect(node!.raw).toBe("tine.view:: grid");
    expect(node!.children).toHaveLength(2); // one node per copied row
    expect(node!.children[0].children.map((c) => c.raw.split("\n")[0])).toEqual(["A", "B"]);

    // Not our clipboard → caller falls through to normal text paste.
    expect(structuralSheetPasteNode("unrelated\ttext")).toBeNull();

    // A single copied cell pastes as plain text, never a 1×1 "subgrid".
    const one = { kind: "cell", gridId: "src", row: 0, col: 0 } as const;
    const { text: oneText } = sheetSelectionText(one);
    await copySheetSelection(one);
    expect(structuralSheetPasteNode(oneText)).toBeNull();
  });

  it("deleteRows / deleteColumns remove full lines in one undo unit", () => {
    const gridId = loadGrid(); // rows: [A,B,C], [D], []
    const before = pageToDto("Sheet");
    deleteRows(gridId, 0, 0);
    expect(doc.byId[gridId].children).toHaveLength(2);
    undo();
    expect(pageToDto("Sheet")).toEqual(before);

    deleteColumns(gridId, 0, 0);
    // first row loses its first cell; row that had one cell is now empty
    expect(doc.byId[doc.byId[gridId].children[0]].children.map((id) => doc.byId[id].raw)).toEqual(["B", "C"]);
    undo();
    expect(pageToDto("Sheet")).toEqual(before);
  });

  it("leaves non-matching clipboard text on the TSV/plain paste path", async () => {
    loadStructuralPasteDoc();
    await copySheetSelection({ kind: "cell", gridId: "src", row: 0, col: 0 });

    expect(pasteStructuralSheetSelection({ kind: "cell", gridId: "dst", row: 0, col: 0 }, "X\tY")).toBeUndefined();
    const next = pasteTextIntoSheetSelection({ kind: "cell", gridId: "dst", row: 0, col: 0 }, "X\tY");

    expect(next).toEqual({ kind: "range", gridId: "dst", anchor: { row: 0, col: 0 }, focus: { row: 0, col: 1 } });
    // TSV paste replaces cell TEXT only: the target keeps its sheet config (and
    // thus its subgrid children stay a grid) instead of being half-destroyed.
    const [first, second] = doc.byId.dr1.children.map((id) => doc.byId[id].raw);
    expect(first.split("\n")[0]).toBe("X");
    expect(first).toContain("tine.view:: grid");
    expect(second).toBe("Y");
  });

  it("escapes tabbed and multiline cell bodies into one parseable TSV scalar", () => {
    const gridId = loadGrid();
    const id = cellId(gridId, 0, 0)!;
    setDoc("byId", id, "raw", "A\tB\nsecond line");

    const { text } = sheetSelectionText({ kind: "cell", gridId, row: 0, col: 0 });

    expect(text).toBe("A B second line");
    expect(parseDelimitedText(text, "tsv")).toEqual([["A B second line"]]);
  });

  it("adds a child bullet after wrapping a compact cell grid", () => {
    loadStructuralPasteDoc();
    const before = pageToDto("Sheet");

    const child = appendSheetCellChild("target");

    expect(child).toBeTruthy();
    expect(doc.byId.target.raw).toBe("Target");
    expect(doc.byId.target.children).toHaveLength(2);
    expect(doc.byId[doc.byId.target.children[0]].raw).toContain("tine.view:: grid");
    expect(doc.byId[doc.byId.target.children[1]].raw).toBe("");

    undo();
    expect(pageToDto("Sheet")).toEqual(before);
  });
});

describe("org-format sheet writes", () => {
  function orgNode(id: string, raw: string, parent: string | null, children: string[]) {
    return { id, raw, collapsed: false, parent, page: "Org", children };
  }

  function loadOrgDoc(byId: Record<string, ReturnType<typeof orgNode>>, roots: string[]) {
    setDoc({
      byId,
      pages: [{ name: "Org", kind: "page", title: "Org", preBlock: null, roots, format: "org", readOnly: false }],
      feed: ["Org"],
      loaded: true,
    });
  }

  it("clearing a cell keeps its :PROPERTIES: drawer (id survives, refs not orphaned)", () => {
    loadOrgDoc(
      {
        grid: orgNode("grid", "Grid\n:PROPERTIES:\n:tine.view: grid\n:END:", null, ["r1"]),
        r1: orgNode("r1", "", "grid", ["c1", "c2"]),
        c1: orgNode("c1", "Old\n:PROPERTIES:\n:id: keep-me\n:END:", "r1", []),
        c2: orgNode("c2", "B", "r1", []),
      },
      ["grid"]
    );

    expect(clearSheetSelection({ kind: "cell", gridId: "grid", row: 0, col: 0 })).toBe(true);

    expect(doc.byId.c1.raw).toBe(":PROPERTIES:\n:id: keep-me\n:END:");
  });

  it("pasting into a cell rebuilds the drawer at org's canonical spot", () => {
    loadOrgDoc(
      {
        grid: orgNode("grid", "Grid\n:PROPERTIES:\n:tine.view: grid\n:END:", null, ["r1"]),
        r1: orgNode("r1", "", "grid", ["c1"]),
        c1: orgNode("c1", "Old\n:PROPERTIES:\n:id: keep-me\n:END:", "r1", []),
      },
      ["grid"]
    );

    pasteTextIntoSheetSelection({ kind: "cell", gridId: "grid", row: 0, col: 0 }, "New");

    expect(doc.byId.c1.raw).toBe("New\n:PROPERTIES:\n:id: keep-me\n:END:");
  });

  it("wrapping a compact org cell grid moves the drawer config onto the host", () => {
    loadOrgDoc(
      {
        grid: orgNode("grid", "Grid\n:PROPERTIES:\n:tine.view: grid\n:END:", null, ["r1"]),
        r1: orgNode("r1", "", "grid", ["cell"]),
        cell: orgNode("cell", "Cell\n:PROPERTIES:\n:tine.view: grid\n:END:", "r1", ["ir1"]),
        ir1: orgNode("ir1", "", "cell", ["ic1"]),
        ic1: orgNode("ic1", "X", "ir1", []),
      },
      ["grid"]
    );

    const child = appendSheetCellChild("cell");

    expect(child).toBeTruthy();
    expect(doc.byId.cell.raw).toBe("Cell");
    expect(doc.byId.cell.children).toHaveLength(2);
    const host = doc.byId.cell.children[0];
    expect(doc.byId[host].raw).toBe(":PROPERTIES:\n:tine.view: grid\n:END:");
    expect(doc.byId[host].children).toEqual(["ir1"]);
  });
});

describe("fill preserves target hidden properties", () => {
  it("fill-right merges the source into the target's visible text, keeping the target id::", () => {
    setDoc({
      byId: {
        grid: { id: "grid", raw: "Grid\ntine.view:: grid", collapsed: false, parent: null, page: "Sheet", children: ["r1"] },
        r1: { id: "r1", raw: "", collapsed: false, parent: "grid", page: "Sheet", children: ["a", "b"] },
        a: { id: "a", raw: "A", collapsed: false, parent: "r1", page: "Sheet", children: [] },
        b: { id: "b", raw: "B\nid:: keep", collapsed: false, parent: "r1", page: "Sheet", children: [] },
      },
      pages: [{ name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["grid"], format: "md", readOnly: false }],
      feed: ["Sheet"],
      loaded: true,
    });

    const ok = fillSheetSelection(
      { kind: "range", gridId: "grid", anchor: { row: 0, col: 0 }, focus: { row: 0, col: 1 } },
      "right"
    );

    expect(ok).toBe(true);
    expect(doc.byId.b.raw).toBe("A\nid:: keep");
  });
});
