import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("onboards keyboard graph switching and YouTube timestamp insertion and seeking", () => {
  const guide = (name: string) => readFileSync(`crates/tine-core/src/templates/${name}.md`, "utf8");
  for (const detail of ["**g Shift+G**", "**Up/Down**", "**Enter**", "sidebar shows the chosen graph's name", "leaves its files untouched"]) expect(guide("bring-existing-graph")).toContain(detail);
  for (const detail of ["**Ctrl/Cmd+Shift+Y**", "{{youtube-timestamp N}}", "click it to seek", "Without a ready player"]) expect(guide("tips")).toContain(detail);
});
