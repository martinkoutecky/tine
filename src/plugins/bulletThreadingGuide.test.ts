import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the bundled Guide teaches how to configure and disable bullet threading without changing notes", () => {
  const guide = readFileSync("crates/tine-core/src/templates/plugins.md", "utf8");
  for (const outcome of ["All nested outlines", "0 hides the lines", "current theme accent", "Color by hierarchy level", "off by default", "without changing your notes"]) {
    expect(guide).toContain(outcome);
  }
});
