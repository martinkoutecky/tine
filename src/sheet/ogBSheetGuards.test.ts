import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("I-12: sheet transfers use the shared hidden-key answerer, rename uses parser regions and editor grammar, hydration never replaces occupied slots", () => {
  const read = (name: string) => readFileSync(new URL(name, import.meta.url), "utf8");
  const mutations = read("./mutations.ts");
  expect(mutations).toContain("splitProps(visible, isSheetCellHidden, fmt)");
  expect(mutations).toContain("splitProps(raw, isSheetCellHidden, formatForBlock(id))");
  expect(mutations).not.toContain("rawWithoutId");
  const rename = read("./renameField.ts");
  expect(rename).toContain("parseBody(raw, format)");
  expect(rename).toContain('block.kind !== "properties"');
  expect(rename).toContain("PROP_LINE.exec(line.text)");
  expect(rename).toContain("pagePropertyEntries(");
  expect(rename).not.toContain("transitionFence");
  expect(rename).not.toContain("[A-Za-z0-9_./-]");
  const hydration = read("./queryHydration.ts");
  expect(hydration).toContain("if (occupied) return;");
  expect(hydration).toContain("if (after) return;");
  expect(hydration).toContain("identitiesByName.get(group.page)?.size === 1");
});
