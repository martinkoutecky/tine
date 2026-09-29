// I-12 (family 7): the static-export sheet code computes nothing itself. It calls the
// SAME functions the live SheetTable / SheetBoard / SheetGrid call, so what the app shows
// and what a published site shows cannot drift. A second implementation of grouping,
// formulas or aggregates in the export path is the failure this guard names.
// Exemplar: src/sheet/staticExport.ts (imports its semantics, defines none).
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const uncommented = (source: string) => source.split("\n").filter((line) => !line.trim().startsWith("//")).join("\n");
const code = uncommented(readFileSync(new URL("./sheet/staticExport.ts", import.meta.url), "utf8"));
const ownFunctions = (source: string) =>
  [...source.matchAll(/(?:function\s+|const\s+)(\w+)\s*(?:=\s*(?:\([^)]*\)|\w+)\s*=>|\()/g)].map((m) => m[1]);
const SEMANTIC_NAME = /^(aggregate\w*|group\w*|sort\w*|compare\w*|evaluate\w*|eval\w*|sum|avg|count\w*)$/i;

describe("static-export sheets have no second implementation (I-12)", () => {
  it("imports every sheet semantic from the shared sheet modules", () => {
    for (const [name, module] of [
      ["childrenSheetConfig", "./childrenSheet"],
      ["filterFormulaRows", "./formulaEval"],
      ["computeFormulaResults", "./formulaEval"],
      ["buildBoardColumns", "./boardColumns"],
      ["cellView", "./cellPresentation"],
      ["aggregate", "./aggregate"],
      ["collectAggregateColumns", "./aggregate"],
    ] as const) {
      expect(code, `I-12: staticExport.ts must call the shared ${name}; exemplar tableBody`)
        .toMatch(new RegExp(`import[^;]*\\b${name}\\b[^;]*from "${module.replace(".", "\\.")}"`));
    }
  });

  it("defines no aggregate, grouping, comparison or formula function of its own", () => {
    const own = ownFunctions(code);
    for (const name of own) {
      expect(name, `I-12: ${name} looks like sheet semantics; put it in src/sheet/ and share it with the live views`)
        .not.toMatch(SEMANTIC_NAME);
    }
    expect(own).toEqual(expect.arrayContaining(["computeSheetExport", "computeSheetExports"]));
  });

  it("the check fails on a planted second implementation", () => {
    const planted = uncommented("export function aggregateColumn(values: number[]) { return values.reduce((a, b) => a + b, 0); }\nconst groupRows = (rows: unknown[]) => rows;");
    expect(ownFunctions(planted).filter((n) => SEMANTIC_NAME.test(n))).toEqual(["aggregateColumn", "groupRows"]);
  });
});
