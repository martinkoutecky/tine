import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the Guide teaches resolve, reveal, reply-to-reopen and immutable comment dates", () => {
  const tips = readFileSync("crates/tine-core/src/templates/tips.md", "utf8");
  for (const fact of ["**Resolve**", "**N resolved**", "**Reopen**", "Replying to a resolved thread reopens it", "tine.resolved:: true", "tine.date:: 2026-11-07T13:37Z", "Editing never updates it", "messages keep file order", "Ordinary blocks get no stored date"])
    expect(tips).toContain(fact);
});

it("the Guide explains width-only live popovers and phone sheets", () => {
  const tips = readFileSync("crates/tine-core/src/templates/tips.md", "utf8");
  for (const fact of ["slim passage rail", "live popover", "bottom sheet", "swipe between passages", "own open threads", "even when the block is collapsed", "no count badge"]) expect(tips).toContain(fact);
  expect(tips).not.toContain("comments stay in the outline as cards");
  expect(tips).toContain("revealing adds a separate **N resolved** entry beside the source block");
  expect(tips).toContain("Closed threads never regain passage markers, highlights or open counts until reopened");
});

it("the Guide explains unique context, ambiguous quotes, flat replies, thread counts and touch read-mode Comment (GH #891, #110)", () => {
  const tips = readFileSync("crates/tine-guide/src/templates/tips.md", "utf8");
  expect(tips).toContain("with `tine.quote-prefix::` and `tine.quote-suffix::`, a few words on either side");
  for (const text of ["grows until the occurrence is unique", "quoted text ambiguous", "at one level in file order", "direct children of the thread root", "counts threads, never replies", "select words in rendered text", "**Show/Hide comments**"]) expect(tips).toContain(text);
  for (const text of ["does not inherit from a parent", "stays hidden while editing", "author mark and chip stay visible", "**Remove author**", "original author property bytes", "**How should your comments be signed?**", "both see each other's unsigned comments as **you**"]) expect(tips).toContain(text);
  for (const text of ["each day's page owning its margin column", "split panes also use their own available width", "Comment and reply editors hide every `tine.*` property on all platforms", "original property bytes stay in the file"]) expect(tips).toContain(text);
});

it("the Guide explains sibling author runs and their independent visibility command (GH #892, #893, #896)", () => {
  const tips = readFileSync("crates/tine-core/src/templates/tips.md", "utf8");
  for (const fact of ["centred in the bullet-to-text gap", "chip only on the first block", "Hover or focus an individual mark", "**Show/Hide author marks**", "no default key", "shown by default", "Names stay available when the lines are hidden", "child outlines, embeds and comment threads break runs"])
    expect(tips).toContain(fact);
});

it("the Guide explains that body structure skips whole annotation threads (GH #891, #110)", () => {
  const tips = readFileSync("crates/tine-guide/src/templates/tips.md", "utf8");
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

it("the Guide explains multiline comments, grouped passages, subtree counts and the touch visibility control", () => {
  const tips = readFileSync("crates/tine-core/src/templates/tips.md", "utf8");
  for (const fact of ["**Enter** and **Shift+Enter** insert a line break", "**Ctrl/Cmd+Enter**", "**Send**", "**N below**", "**New thread**", "page actions menu", "share one card and one quote"]) expect(tips).toContain(fact);
  expect(tips).not.toContain("**Enter** in a comment's text adds a reply");
});
