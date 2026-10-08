import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
it("the Guide teaches committing complete planning drafts and configured journal date links", () => {
  const guide = readFileSync("crates/tine-core/src/templates/journals-tasks-scheduling.md", "utf8");
  for (const outcome of ["Click a day to select it", "**Done** or clicking outside applies the chosen date, time, and repeat together", "**Escape** cancels all pending changes", "**/Date picker**", "graph's journal title format", "**/Tomorrow**", "**/Yesterday**"]) expect(guide).toContain(outcome);
});
