import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const guide = (page: string) => readFileSync(`crates/tine-core/src/templates/${page}.md`, "utf8");

it("the Guide explains keyboard folding and the lifetime of the Hierarchy choice (GH #738)", () => {
  const where = guide("where-things-are");
  expect(where).toContain("**Hierarchy** section below its outline");
  expect(where).toContain("press **Enter** or **Space**");
  expect(where).toContain("starts expanded and remembers your choice for that page until you switch graphs");
});

it("the Guide describes one-character Settings search, the matched description and the jump highlight (GH #706)", () => {
  const where = guide("where-things-are");
  for (const detail of [
    "the setting is highlighted for about two seconds",
    "a result found only through its description shows that description with the match highlighted",
    "A single character matches only the text shown in your interface language",
  ]) expect(where).toContain(detail);
  expect(where).not.toContain("A single character matches only a setting's name and aliases, not its description");
});

it("the Guide says the unreadable-files notice names the files and copies the full list (GitHub discussion #727)", () => {
  const recovery = guide("troubleshooting-recovery");
  for (const detail of [
    "**Some page files couldn't be read**",
    "the notice names those files (the first five, then how many more)",
    "**Copy** on the notice copies the full list",
  ]) expect(recovery).toContain(detail);
});
