import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the onboarding Guide explains conversion refusals and concurrent checkouts in CLI runs", () => {
  const guide = readFileSync(new URL("../crates/tine-core/src/templates/command-line.md", import.meta.url), "utf8");
  for (const attribute of ["filter", "ident", "working-tree-encoding"]) {
    expect(guide).toContain(`\`${attribute}\``);
  }
  expect(guide).toContain("Tine never runs external filters");
  expect(guide).toContain("warning says it was not committed");
  expect(guide).toContain("both branches and the new branch's index alone");
});
