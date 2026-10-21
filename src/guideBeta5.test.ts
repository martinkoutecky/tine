import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const guide = (page: string) => readFileSync(`crates/tine-core/src/templates/${page}.md`, "utf8");

it("the Guide locates live unsupported journal pattern notes in Settings even with a sections page (GH #317)", () => {
  const journals = guide("journals-tasks-scheduling");
  for (const detail of ["notes also appear in Settings → **Journals**, beside **Journal date format**", "even when a Today's sections page replaces", "notes update when the format changes"]) expect(journals).toContain(detail);
});

it("the Guide describes the phone navigation row, its Appearance setting and how it returns after the keyboard closes (GH #716)", () => {
  const mobile = guide("platforms-and-mobile");
  for (const detail of [
    "**Sidebar, Search, Back and Forward are in the top bar**",
    "**Navigation buttons at the bottom**",
    "off by default, this device only",
    "the editing toolbar, which then carries the same four buttons at its left end",
    "tapping one saves the block and leaves the edit before it acts",
    "close the keyboard with Back or the keyboard's hide key",
    "tap the block to bring the keyboard and the editing toolbar back",
  ]) expect(mobile).toContain(detail);
});

// GH #705 said phones and tablets had no tab strip; D-20261018-W1 gates it on width instead, so the Guide
// now says where a new tab goes on a narrow window.
it("the Guide says where Open in new tab goes on a narrow window (GH #705, D-20261018-W1)", () => {
  expect(guide("pages-links-references-search")).toContain("it is listed under **Tabs** in the left sidebar");
  expect(guide("tips")).toContain("on a narrow window the tab is listed under **Tabs** in the left sidebar");
  expect(guide("platforms-and-mobile")).toContain("The **tab strip** appears whenever the window is 640 px or wider, on every platform");
});

it("the Guide says folding a query block folds its results and that Unlinked References mark every mention (GH #619, #200)", () => {
  expect(guide("queries")).toContain("Fold the query block's bullet to hide its results");
  const references = guide("pages-links-references-search");
  for (const detail of ["Each mention is highlighted inside its excerpt", "also gets a numbered jump row"]) expect(references).toContain(detail);
});

it("the Guide ties the All pages journal-date column to Show journals and ranks Settings search name matches first (GH #706)", () => {
  const where = guide("where-things-are");
  for (const detail of [
    "adds the **Journal date** column",
    "Names that match come before descriptions that match",
  ]) expect(where).toContain(detail);
  // Beta 6 made one character match descriptions again; src/guideBeta6.test.ts pins the new sentence.
});
