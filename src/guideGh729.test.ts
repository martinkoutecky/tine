import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const guide = (page: string) => readFileSync(`crates/tine-core/src/templates/${page}.md`, "utf8");

it("the Guide says a page can be a template through its own property header (discussion #729)", () => {
  const journals = guide("journals-tasks-scheduling");
  for (const detail of [
    "A page can be a template too",
    "`template:: Name` among the page's own property lines at its top",
    "inserts those property lines (without `template::`) as one block",
  ]) expect(journals).toContain(detail);
});
