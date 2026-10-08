import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("I-12: sheet transfers use the shared hidden-key answerer, rename uses parser property spans, hydration never replaces occupied slots", () => {
  const read = (name: string) => readFileSync(new URL(name, import.meta.url), "utf8");
  const mutations = read("./mutations.ts");
  expect(mutations).toContain("splitProps(visible, isSheetCellHidden, fmt)");
  expect(mutations).toContain("splitProps(raw, isSheetCellHidden, formatForBlock(id))");
  expect(mutations).not.toContain("rawWithoutId");
  const rename = read("./renameField.ts");
  expect(rename).toContain("blockRegions(raw, format).properties");
  expect(rename).toContain("p.key_range[0]");
  expect(rename).toContain("p.value_range[0]");
  expect(rename).not.toMatch(/PROP_LINE|mdOccurrence|orgOccurrence/);
  expect(rename).not.toContain("transitionFence");
  expect(rename).not.toContain("[A-Za-z0-9_./-]");
  const hydration = read("./queryHydration.ts");
  expect(hydration).toContain("if (occupied) return;");
  expect(hydration).toContain("if (after) return;");
  expect(hydration).toContain("identitiesByName.get(group.page)?.size === 1");
});

it("I-12/I-22: native grid layout uses the same row cap as export inputs and charges bounded area", () => {
  const native = readFileSync(new URL("../../crates/tine-graph-features/src/render_sheets.rs", import.meta.url), "utf8");
  expect(native).toContain("owner.children.iter().take(MAX_INPUT_ROWS).enumerate()");
  expect(native).toContain("rows.min(MAX_INPUT_ROWS).saturating_mul(cols)");
  expect(native).toContain("sheets.admit_grid(owner.children.len(), *cols)");
});


it("I-12: query footers format backend statistics through querySummary and use the display writer; follow queryTableFooter.ts", () => {
  const table = readFileSync(new URL("../components/SheetTable.tsx", import.meta.url), "utf8");
  expect(table).toContain("queryTableFooter(props.queryDisplay, field)");
  const footer = readFileSync(new URL("./queryTableFooter.ts", import.meta.url), "utf8");
  expect(footer).toContain("querySummary({ statistics })");
  expect(table).toContain("values={props.queryDisplay ? [] : sortedRows()");
  expect(footer).toContain("control.apply({ ...control.view, aggregates: entries })");
  const macro = readFileSync(new URL("../components/Macro.tsx", import.meta.url), "utf8");
  expect(macro).toContain("statistics: displayed()?.statistics, statisticsView: runnable()?.view");
  const exporter = readFileSync(new URL("./staticExport.ts", import.meta.url), "utf8");
  const native = readFileSync(new URL("../../crates/tine-graph-features/src/render_sheets.rs", import.meta.url), "utf8");
  expect(exporter, "Export totals must use exported rows, never unrestricted query statistics").not.toContain("QueryStatistics");
  expect(native, "Unrestricted statistics must not cross the published-page boundary").not.toContain("QueryStatistics");
});
