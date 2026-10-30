import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("onboards the zoom commands and the Ctrl/Cmd+R and F5 reload change (GH #797, #799)", () => {
  const guide = readFileSync("crates/tine-core/src/templates/tips.md", "utf8");
  for (const text of ["**Zoom in on block**", "**Zoom out one level**", "start unbound", "does nothing on the page itself",
    "**Ctrl/Cmd+R** and **F5** no longer reload the app", "**Reload window**", "Comment on selected text"])
    expect(guide).toContain(text);
});
