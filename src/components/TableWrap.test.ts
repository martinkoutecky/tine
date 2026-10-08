import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { relative } from "node:path";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = (file: string) => process.env.QBG_TEST_BASE
  ? execFileSync("git", ["show", `${process.env.QBG_TEST_BASE}:${relative(process.cwd(), fileURLToPath(new URL(file, import.meta.url)))}`], { encoding: "utf8" })
  : readFileSync(new URL(file, import.meta.url), "utf8");

describe("table layout onboarding", () => {
  it("explains when tables bleed and when they scroll", () => {
    const guide = source("../../crates/tine-core/src/templates/sheets.md");
    expect(guide).toContain("scroll horizontally only when wider than the pane");
    expect(guide).toContain("tables inside a sheet cell stay within that cell");
  });
});

describe("pane table layout ownership (I-12)", () => {
  it("routes Markdown/Org and query tables through the shared TableWrap viewport", () => {
    for (const file of ["../render/body.tsx", "./QueryLegacyTable.tsx", "./QueryResultParts.tsx", "./Macro.tsx"]) {
      const text = source(file);
      expect(text, `I-12: ${file} must use TableWrap.tsx, the pane table layout owner`).toContain("<TableWrap>");
      expect(text).not.toContain('<div class="md-table-wrap">');
    }
  });
  it("contains tables nested inside sheet cells", () => {
    expect(source("./TableWrap.tsx")).toContain('el.closest(".sheet-cell")');
    expect(source("./SheetContainer.tsx")).toContain('!nested');
  });
  it("shares pane geometry with sheets and enables it by default", () => {
    const text = source("./SheetContainer.tsx");
    expect(text).toContain("tableBleedGeometry(");
    expect(text).toContain("props.allowBreakout !== false && !nested");
  });
});
