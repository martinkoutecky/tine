import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("onboards page/subtree folding and macOS mouse/keyboard history (GH #780, #785)", () => {
  const guide = readFileSync("crates/tine-core/src/templates/tips.md", "utf8");
  for (const text of ["**t o**", "Expand all blocks on current page", "Collapse all blocks on current page",
    "block's subtree", "one Undo step", "Mouse side buttons", "**Cmd+[**", "side-button drag"])
    expect(guide).toContain(text);
});
