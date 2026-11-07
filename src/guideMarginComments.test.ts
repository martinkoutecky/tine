import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the Guide says a margin comment keeps a few words of context around its quote (ADR 0075, D-20261018-MC2)", () => {
  const tips = readFileSync("crates/tine-core/src/templates/tips.md", "utf8");
  expect(tips).toContain("with `tine.quote-prefix::` and `tine.quote-suffix::`, a few words on either side");
});

it("the Guide explains that body structure skips whole annotation threads (GH #891, #110)", () => {
  const tips = readFileSync("crates/tine-core/src/templates/tips.md", "utf8");
  for (const fact of [
    "Comments and all their replies are annotations",
    "arrow keys skip whole threads",
    "whose only children are comments creates a sibling",
    "cannot move a comment or nest a body block inside a thread",
    "body blocks cannot be dropped or pasted inside their threads",
    "Moving or deleting the commented body block takes its comments with it",
  ]) expect(tips).toContain(fact);
  expect(tips).not.toContain("Drag it to move the comment");
});
