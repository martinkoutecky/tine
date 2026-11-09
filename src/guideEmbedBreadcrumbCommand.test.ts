import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the Guide explains the breadcrumb command, its unassigned shortcut and shared device setting (GH #860)", () => {
  const guide = readFileSync("crates/tine-guide/src/templates/customize-look.md", "utf8");
  expect(guide).toContain("**Toggle embed breadcrumbs**");
  expect(guide).toContain("no default key");
  expect(guide).toContain("**Settings → Keyboard shortcuts**");
  expect(guide).toContain("same device setting");
});
