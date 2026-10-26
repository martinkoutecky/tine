import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const guide = (page: string) => readFileSync(`crates/tine-core/src/templates/${page}.md`, "utf8");

it("the Guide describes the phone navigation row, its Appearance setting and how it returns after the keyboard closes (GH #716)", () => {
  const mobile = guide("platforms-and-mobile");
  for (const detail of [
    "**Sidebar, Search, Back and Forward are in the top bar**",
    "**Navigation buttons at the bottom**",
    "off by default, this device only",
    "close the keyboard with Back or the keyboard's hide key",
    "tap the block to bring the keyboard and the editing toolbar back",
  ]) expect(mobile).toContain(detail);
});

it("the Guide says phones and tablets have no Open in new tab (GH #705)", () => {
  expect(guide("pages-links-references-search")).toContain("Phones and tablets have no tab strip");
  expect(guide("tips")).toContain("phones and tablets have no tab strip and offer **Open in sidebar** instead");
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
