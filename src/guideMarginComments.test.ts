import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the Guide says a margin comment keeps a few words of context around its quote (ADR 0075, D-20261018-MC2)", () => {
  const tips = readFileSync("crates/tine-core/src/templates/tips.md", "utf8");
  expect(tips).toContain("with `quote-prefix::` and `quote-suffix::`, a few words on either side");
});
