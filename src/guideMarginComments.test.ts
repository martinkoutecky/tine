import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the Guide explains unique context, ambiguous quotes, flat replies, thread counts and touch read-mode Comment (GH #891, #110)", () => {
  const tips = readFileSync("crates/tine-core/src/templates/tips.md", "utf8");
  expect(tips).toContain("with `tine.quote-prefix::` and `tine.quote-suffix::`, a few words on either side");
  for (const text of ["grows until the occurrence is unique", "quoted text ambiguous", "at one level in file order", "direct children of the thread root", "counts threads, never replies", "select words in rendered text", "**Show/Hide comments**"]) expect(tips).toContain(text);
  for (const text of ["does not inherit from a parent", "stays hidden while editing", "author mark and chip stay visible", "**Remove author**", "original author property bytes", "**How should your comments be signed?**", "both see each other's unsigned comments as **you**"]) expect(tips).toContain(text);
  for (const text of ["each day's page owning its margin column", "split panes also use their own available width", "Comment and reply editors hide every `tine.*` property on all platforms", "original property bytes stay in the file"]) expect(tips).toContain(text);
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
