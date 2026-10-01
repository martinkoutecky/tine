import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = (path: string) => readFileSync(path, "utf8");
describe("OG-DUPAL1 shared computation owners", () => {
  it("F07 I-12: reference collapse mutations belong to referenceGroupCollapse.ts", () => {
    for (const panel of ["LinkedReferences", "UnlinkedReferences"]) {
      const text = source(`src/components/${panel}.tsx`);
      expect(text).toContain("createReferenceGroupCollapse(");
      expect(text).not.toMatch(/setCollapsedGroupsFor|setCollapsedGroupsSignal|createReferenceFetcher/);
    }
  });
  it("F08 I-12: sheet focus, offset and menu primitives use sheet/selection.ts and sheet/interactions.ts", () => {
    for (const panel of ["SheetBoard", "SheetTable", "SheetGrid"]) {
      const text = source(`src/components/${panel}.tsx`);
      expect(text).toContain("cellIsSelected(");
      expect(text).toContain("sheetCellMenu(");
      expect(text).not.toMatch(/openSheetCellContextMenu|caretRangeFromPoint|Math\.ceil\(\(row \+ 1\)/);
    }
    expect(source("src/components/SheetGrid.tsx")).toContain("cellIsInLegacyRange(");
    const grid = source("src/components/SheetGrid.tsx");
    expect(grid.slice(grid.indexOf("function inSelectedRange"), grid.indexOf("function SheetGridCell"))).not.toMatch(/Math\.min\(sel\.anchor/);
  });
  it("F10 I-12: clipboard formatting and ancestry belong to edits/serialize.ts shared primitives", () => {
    const text = source("src/document/edits/serialize.ts");
    expect(text.match(/raw\.split\("\\n"\)/g)).toHaveLength(1);
    expect(text.match(/while \(parent !== null\)/g)).toHaveLength(1);
    expect(text.match(/markdownBlockLines\(/g)).toHaveLength(3);
    expect(text.match(/hasSelectedAncestor\(/g)).toHaveLength(3);
  });
});
