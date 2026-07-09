import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { inlineText, parseBody } from "../render/facets";
import { pageToDto, resetStore, setDoc, undo, doc, type FeedPage, type Node as StoreNode } from "../store";
import { setToasts, toasts } from "../ui";
import {
  canConvertPipeTableToGrid,
  convertGridToPipeTable,
  convertPipeTableToGrid,
  escapedPipeCellsRoundTrip,
  gridVisibleMatrix,
  insertMatrixGridAfter,
} from "./conversions";

beforeAll(() => initParser());

beforeEach(() => {
  resetStore();
  setToasts([]);
});

function page(roots: string[]): FeedPage {
  return {
    name: "Sheet",
    kind: "page",
    title: "Sheet",
    preBlock: null,
    roots,
    format: "md",
    readOnly: false,
    guide: false,
  };
}

function node(id: string, raw: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

function load(byId: Record<string, StoreNode>, roots: string[]) {
  setDoc({ byId, pages: [page(roots)], feed: ["Sheet"], loaded: true });
}

function grid(raw = "Grid\ntine.view:: grid", matrix: readonly (readonly string[])[] = [["A", "B"], ["C"]]) {
  const byId: Record<string, StoreNode> = { grid: node("grid", raw, null, []) };
  matrix.forEach((row, r) => {
    const rowId = `r${r}`;
    byId[rowId] = node(rowId, "", "grid", []);
    byId.grid.children.push(rowId);
    row.forEach((cell, c) => {
      const cellId = `c${r}-${c}`;
      byId[cellId] = node(cellId, cell, rowId);
      byId[rowId].children.push(cellId);
    });
  });
  load(byId, ["grid"]);
}

function rawMatrix(id: string): string[][] {
  return (doc.byId[id]?.children ?? []).map((rowId) =>
    (doc.byId[rowId]?.children ?? []).map((cellId) => doc.byId[cellId]?.raw ?? ""),
  );
}

function lastToast(): string {
  return toasts()[toasts().length - 1]?.message ?? "";
}

describe("sheet pipe-table/grid conversions", () => {
  it("shows pipe-table conversion only for a markdown block ending in one parsed table", () => {
    load({ b: node("b", "Title\n| A | B |\n| --- | --- |\n| 1 | 2 |", null) }, ["b"]);
    expect(canConvertPipeTableToGrid("b")).toBe(true);

    load({ b: node("b", "Title\n| A | B |\n| --- | --- |\n| 1 | 2 |\nAfter", null) }, ["b"]);
    expect(canConvertPipeTableToGrid("b")).toBe(false);

    load({ b: node("b", "Title\n| A | B |\n| --- | --- |\n| 1 | 2 |\n\n| C | D |\n| --- | --- |", null) }, ["b"]);
    expect(canConvertPipeTableToGrid("b")).toBe(false);
  });

  it("converts a pipe table to grid rows/cells, preserving source cell markup and undoing as one unit", () => {
    load(
      {
        b: node(
          "b",
          "Title\nowner:: Ada\n\nIntro\n| A | B |\n| --- | --- |\n| **bold** | [[Page]] |\n|  | tail |",
          null,
        ),
      },
      ["b"],
    );
    const before = pageToDto("Sheet");

    expect(convertPipeTableToGrid("b")).toBe(true);

    expect(doc.byId.b.raw).toBe("Title\nowner:: Ada\ntine.view:: grid\ntine.header:: true\n\nIntro");
    expect(rawMatrix("b")).toEqual([
      ["A", "B"],
      ["**bold**", "[[Page]]"],
      ["", "tail"],
    ]);

    undo();
    expect(pageToDto("Sheet")).toEqual(before);
  });

  it("refuses pipe-table conversion when existing children would collide with grid rows", () => {
    load(
      {
        b: node("b", "Title\n| A |\n| --- |\n| 1 |", null, ["child"]),
        child: node("child", "Existing", "b"),
      },
      ["b"],
    );
    const before = pageToDto("Sheet");

    expect(canConvertPipeTableToGrid("b")).toBe(true);
    expect(convertPipeTableToGrid("b")).toBe(false);

    expect(pageToDto("Sheet")).toEqual(before);
    expect(lastToast()).toContain("children would collide");
  });

  it("converts a grid to a canonical pipe table, drops sheet config, deletes children, and undoes as one unit", () => {
    grid("Title\nowner:: Ada\ntine.view:: grid\ntine.header:: true\ntine.col-widths:: 0=120", [
      ["A", "B"],
      ["**bold**", "[[Page]]"],
      ["short"],
    ]);
    const before = pageToDto("Sheet");

    expect(convertGridToPipeTable("grid")).toBe(true);

    expect(doc.byId.grid.raw).toBe("Title\nowner:: Ada\n| A | B |\n| --- | --- |\n| **bold** | [[Page]] |\n| short |");
    expect(doc.byId.grid.children).toEqual([]);
    expect(doc.byId["r0"]).toBeUndefined();
    expect(doc.byId["c1-1"]).toBeUndefined();

    undo();
    expect(pageToDto("Sheet")).toEqual(before);
  });

  it("keeps non-property body lines of the host verbatim when converting grid to table", () => {
    // regression: a rebuild-from-facets inverse silently dropped body lines
    grid("Title\na note under the title\ntine.view:: grid", [["A"]]);

    expect(convertGridToPipeTable("grid")).toBe(true);
    expect(doc.byId.grid.raw).toBe("Title\na note under the title\n| A |");
  });

  it("refuses grid-to-table when the host has no title line (tine.* on line 0)", () => {
    grid("tine.view:: grid", [["A"]]);
    const before = doc.byId.grid.raw;

    expect(convertGridToPipeTable("grid")).toBe(false);
    expect(doc.byId.grid.raw).toBe(before);
    expect(toasts().length).toBe(1);
  });

  it("round-trips canonical pipe table bytes through grid and back", () => {
    const raw = "Title\nowner:: Ada\n| A | B |\n| --- | --- |\n| **bold** | [[Page]] |";
    load({ b: node("b", raw, null) }, ["b"]);

    expect(convertPipeTableToGrid("b")).toBe(true);
    expect(convertGridToPipeTable("b")).toBe(true);

    expect(doc.byId.b.raw).toBe(raw);
  });

  it("preserves the cell matrix through grid to table to grid", () => {
    grid("Title\ntine.view:: grid", [["A", "B"], ["C"]]);

    expect(convertGridToPipeTable("grid")).toBe(true);
    expect(doc.byId.grid.raw).toBe("Title\n| A | B |\n| C |");
    expect(convertPipeTableToGrid("grid")).toBe(true);

    expect(rawMatrix("grid")).toEqual([["A", "B"], ["C"]]);
    expect(gridVisibleMatrix("grid")).toEqual([["A", "B"], ["C"]]);
  });

  it("verifies escaped pipes do not currently round-trip through lsdoc table cells", () => {
    expect(escapedPipeCellsRoundTrip()).toBe(false);
    const table = parseBody("T\n| a\\|b | c |\n| --- | --- |\n| x | y |", "md").find((b) => b.kind === "table");
    if (!table || table.kind !== "table") throw new Error("expected table");
    expect(table.header?.map((cell) => inlineText(cell))).not.toEqual(["a|b", "c"]);
  });

  it("refuses grid-to-table cells containing pipe while escaped pipes do not round-trip", () => {
    grid("Title\ntine.view:: grid", [["A|B"]]);
    const before = pageToDto("Sheet");

    expect(convertGridToPipeTable("grid")).toBe(false);

    expect(pageToDto("Sheet")).toEqual(before);
    expect(lastToast()).toContain("does not round-trip \\|");
  });

  it("refuses grid-to-table rows or cells that would lose non-table structure", () => {
    const cases: Array<[string, Record<string, StoreNode>, string]> = [
      [
        "row text",
        {
          grid: node("grid", "Grid\ntine.view:: grid", null, ["r0"]),
          r0: node("r0", "not empty", "grid", ["c0"]),
          c0: node("c0", "A", "r0"),
        },
        "row blocks must be empty",
      ],
      [
        "cell children",
        {
          grid: node("grid", "Grid\ntine.view:: grid", null, ["r0"]),
          r0: node("r0", "", "grid", ["c0"]),
          c0: node("c0", "A", "r0", ["nested"]),
          nested: node("nested", "child", "c0"),
        },
        "cells with children",
      ],
      [
        "multiline cell",
        {
          grid: node("grid", "Grid\ntine.view:: grid", null, ["r0"]),
          r0: node("r0", "", "grid", ["c0"]),
          c0: node("c0", "A\nB", "r0"),
        },
        "single-line",
      ],
      [
        "hidden cell property",
        {
          grid: node("grid", "Grid\ntine.view:: grid", null, ["r0"]),
          r0: node("r0", "", "grid", ["c0"]),
          c0: node("c0", "A\nid:: abc", "r0"),
        },
        "cell hidden properties",
      ],
      [
        "hidden row property",
        {
          grid: node("grid", "Grid\ntine.view:: grid", null, ["r0"]),
          r0: node("r0", "id:: row", "grid", ["c0"]),
          c0: node("c0", "A", "r0"),
        },
        "row hidden properties",
      ],
    ];

    for (const [_name, byId, reason] of cases) {
      resetStore();
      setToasts([]);
      load(byId, ["grid"]);
      const before = pageToDto("Sheet");

      expect(convertGridToPipeTable("grid")).toBe(false);

      expect(pageToDto("Sheet")).toEqual(before);
      expect(lastToast()).toContain(reason);
    }
  });

  it("refuses grid-to-table oversize grids", () => {
    grid("Title\ntine.view:: grid", [Array.from({ length: 31 }, (_, i) => `c${i}`)]);
    const before = pageToDto("Sheet");

    expect(convertGridToPipeTable("grid")).toBe(false);

    expect(pageToDto("Sheet")).toEqual(before);
    expect(lastToast()).toContain("30 columns by 200 rows");
  });

  it("inserts a fake delimited matrix as a grid block with one undo unit", () => {
    load({ anchor: node("anchor", "After me", null) }, ["anchor"]);
    const before = pageToDto("Sheet");

    const inserted = insertMatrixGridAfter("anchor", "People", [["Name", "Role"], ["Ada", "[[Math]]"]]);

    expect(inserted).toBeTruthy();
    expect(doc.byId[inserted!].raw).toBe("People\ntine.view:: grid\ntine.header:: true");
    expect(rawMatrix(inserted!)).toEqual([["Name", "Role"], ["Ada", "[[Math]]"]]);

    undo();
    expect(pageToDto("Sheet")).toEqual(before);
  });
});
