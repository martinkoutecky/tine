import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const template = (name: string) => readFileSync(`crates/tine-guide/src/templates/${name}.md`, "utf8");

it("the Guide explains one history step for each repeated mouse press (GH #948)", () => {
  expect(template("tips")).toContain("Each press steps once, including repeated presses before the page finishes displaying, just like **Alt+Left** and **Alt+Right**");
});

it("the Guide explains link completion inside a complete link (GH #888)", () => {
  expect(template("pages-links-references-search")).toContain("the list searches the whole link text, not only the part before the caret, so the linked page comes first");
});

it("the Guide says no completion menu opens inside a fenced code block (CM6-D9)", () => {
  expect(template("tips")).toContain("Inside a fenced code block no menu opens");
});

it("the Guide names the Remove author command and how to run it (GH #918)", () => {
  const tips = template("tips");
  expect(tips).toContain("**Remove author** is also a command");
  expect(tips).toContain("with the block selected, or assign it a key under **Settings → Shortcuts**");
  expect(tips).toContain("With several blocks selected, it removes authors from the whole selection, including folded descendants, in one undo step");
  expect(tips).toContain("Blocks without an author and read-only pages are skipped");
});
