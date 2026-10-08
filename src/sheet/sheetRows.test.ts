import { afterEach, describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resetStore } from "../document";
import { setDoc, type Node } from "../document/model";
import { sheetSourceRows } from "./sheetRows";

afterEach(() => resetStore());

describe("sheet source rows", () => {
  it("lists an owner's children, or the blocks of the query groups with their DTOs", () => {
    const node = (id: string, children: string[]): Node => ({ id, raw: id, page: "P", parent: null, children, collapsed: false, loaded: true } as unknown as Node);
    setDoc({ byId: { o: node("o", ["c1", "c2"]), c1: node("c1", []), c2: node("c2", []) }, pages: [], feed: [], loaded: true });
    expect(sheetSourceRows("children", "o", undefined)).toEqual([{ id: "c1", page: "P" }, { id: "c2", page: "P" }]);
    const block = { id: "b", raw: "b", collapsed: false, children: [] };
    expect(sheetSourceRows("query", "o", [{ page: "Q", kind: "journal", blocks: [block] }]))
      .toEqual([{ id: "b", page: "Q", kind: "journal", dto: block }]);
    expect(sheetSourceRows("query", "o", undefined)).toEqual([]);
  });

  it("is the only place the table and the board build their source rows (I-12)", () => {
    for (const file of ["src/components/SheetTable.tsx", "src/components/SheetBoard.tsx"]) {
      const source = readFileSync(file, "utf8");
      expect(source, file).toContain("sheetSourceRows(");
      expect(source, file).not.toMatch(/g\.blocks\.map\(/);
    }
  });
});
