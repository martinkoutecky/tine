// The Guide states the two user-visible draft-safety behaviours of og-draftrisk
// (REG-OG-DRAFTRISK-SWITCH, REG-OG-DRAFTRISK-EQUAL-BYTES) on its canonical page.
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const page = readFileSync("crates/tine-core/src/templates/files-external-edits-backups.md", "utf8");

it("explains where an edit held at a graph switch is found and how it is released", () => {
  expect(page).toMatch(/switch graphs while an edit could not be given a crash-safe copy, the panel keeps it[^\n]*\*\*Dismiss this draft\*\*/);
});

it("explains that an outside change equal to the unsaved text is not a conflict and the edit is still saved", () => {
  expect(page).toMatch(/exactly your unsaved text[^\n]*is not a conflict[^\n]*still saves your edit/);
});
