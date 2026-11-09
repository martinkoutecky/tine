import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("onboards safe app-wide quit, remapping and sidebar-header window gestures (GH #805, #815)", () => {
  const tips = readFileSync("crates/tine-guide/src/templates/tips.md", "utf8");
  for (const text of ["**Ctrl+Q**", "**Quit Tine**", "every graph window after saving pending edits", "`app/quit`", "**Settings → Keyboard shortcuts**", "**Cmd+Q**", "If a page cannot be saved"])
    expect(tips).toContain(text);
  const where = readFileSync("crates/tine-guide/src/templates/where-things-are.md", "utf8");
  for (const text of ["drag the empty space beside the workspace switcher", "Double-click", "maximize or restore", "stay clickable", "Mobile drawers and system title bars"])
    expect(where).toContain(text);
});
