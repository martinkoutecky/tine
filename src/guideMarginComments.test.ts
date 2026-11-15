import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the Guide teaches resolve, reveal, reply-to-reopen and immutable comment dates", () => {
  const tips = readFileSync("crates/tine-guide/src/templates/tips.md", "utf8");
  for (const fact of ["**Resolve**", "**N resolved**", "**Reopen**", "Replying to a resolved thread reopens it", "tine.resolved:: true", "tine.date:: 2026-11-07T13:37Z", "Editing never updates it", "messages keep file order", "Ordinary blocks get no stored date", "**Message outlines and folding.**", "**Collapse all comments on this block**", "collapsed:: true"])
    expect(tips).toContain(fact);
});

it("the Guide explains width-only live popovers and phone sheets", () => {
  const tips = readFileSync("crates/tine-guide/src/templates/tips.md", "utf8");
  for (const fact of ["live popover", "bottom sheet", "swipe between passages", "Own threads stay reachable", "even when the block is collapsed", "shows no count of its own comments"]) expect(tips).toContain(fact);
  expect(tips).not.toContain("comments stay in the outline as cards");
  expect(tips).toContain("revealing adds a separate **N resolved** entry beside the source block");
  expect(tips).toContain("Closed threads never regain highlights or markers until reopened");
  // D-20261108-MC7/MC8: hover opens, a click places the caret, touch taps; no rail or counts.
  for (const fact of ["hover the highlighted words to open their thread in a live popover", "just like a page link's hover preview", "stays open while you type in it",
    "a click on the words still places the caret", "On a touch tablet, tap the highlighted words", "small comment marker that opens it",
    "choose **In the margin** (the default)", "**In a popup** works the same at every width, including Wide mode"]) expect(tips).toContain(fact);
  for (const gone of ["slim passage rail", "number chip", "counts threads", "On click"]) expect(tips).not.toContain(gone);
});

it("the Guide explains unique context, ambiguous quotes, flat replies and touch read-mode Comment (GH #891, #110)", () => {
  const tips = readFileSync("crates/tine-guide/src/templates/tips.md", "utf8");
  expect(tips).toContain("with `tine.quote-prefix::` and `tine.quote-suffix::`, a few words on either side");
  for (const text of ["grows until the occurrence is unique", "quoted text ambiguous", "at one level in file order", "direct children of the thread root", "select words in rendered text", "**Show/Hide comments**"]) expect(tips).toContain(text);
  for (const text of ["does not inherit from a parent", "stays hidden while editing", "author tint and chip stay visible", "**Remove author**", "original author property bytes", "**How should your comments be signed?**", "both see each other's unsigned comments as **you**"]) expect(tips).toContain(text);
  for (const text of ["each day's page owning its margin column", "split panes also use their own available width", "Comment and reply editors hide every `tine.*` property on all platforms", "original property bytes stay in the file"]) expect(tips).toContain(text);
});

it("the Guide explains sibling author runs and their independent visibility command (GH #892, #893, #896)", () => {
  const tips = readFileSync("crates/tine-guide/src/templates/tips.md", "utf8");
  for (const fact of ["faint background in the author's colour", "never moves the text", "share one card", "chip only on the first block", "Hover a tinted block to see its author's name", "**Show/Hide author marks**", "no default key", "shown by default", "Author chips stay when the tints are hidden", "child outlines, embeds and comment threads break runs"])
    expect(tips).toContain(fact);
  for (const gone of ["gutter lines", "centred in the bullet-to-text gap", "continuous line"]) expect(tips).not.toContain(gone);
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
  const tips = readFileSync("crates/tine-guide/src/templates/tips.md", "utf8");
  for (const fact of ["**Enter** and **Shift+Enter** insert a line break", "**Ctrl/Cmd+Enter**", "**Send**", "**N below**", "**New thread**", "page actions menu", "share one card and one quote"]) expect(tips).toContain(fact);
  expect(tips).not.toContain("**Enter** in a comment's text adds a reply");
});
