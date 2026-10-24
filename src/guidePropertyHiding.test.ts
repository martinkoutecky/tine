import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
it("the Guide explains hidden metadata, preservation and the remaining edit surface", () => {
  const guide = readFileSync("crates/tine-core/src/templates/tips.md", "utf8");
  for (const text of ["**Hidden metadata**", ":block-hidden-properties", "clearing the visible text keeps the original hidden property lines", "Use **Properties…** to change or remove hidden values", "Logseq's `icon`, `title` and `filters` stay hidden there"]) expect(guide).toContain(text);
});
