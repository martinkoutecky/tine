import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const guide = (page: string) => readFileSync(`crates/tine-core/src/templates/${page}.md`, "utf8");

it("the Guide explains the page and block copy submenu and reference click zoom", () => {
  const references = guide("pages-links-references-search");
  for (const detail of ["Page and block menus", "**Copy as ▸**", "**Click a block reference to zoom in**", "including on another page"]) expect(references).toContain(detail);
});

it("the Guide describes Soft's middle-range light and dark choices", () => {
  const appearance = guide("customize-look");
  for (const detail of ["**Soft**", "medium-brightness palette", "warm gray in light mode", "lifted slate gray in dark mode", "warm highlights"]) expect(appearance).toContain(detail);
});

it("the Guide retains source-safe Alt dragging and Search selection guidance", () => {
  const queries = guide("queries");
  for (const detail of ["same Alt gesture works on an ordinary outline bullet", "only after the source's ID has been saved", "**Select results** again or **Done**", "Selection actions stay above the result rows"]) expect(queries).toContain(detail);
});

it("the Guide explains how versioned Betas reach the Beta updater", () => {
  const recovery = guide("troubleshooting-recovery");
  for (const detail of ["tine.page/updates/beta/latest.json", "`vX.Y.Z-beta.N`", "older Betas"]) expect(recovery).toContain(detail);
});
