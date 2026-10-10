// The Guide states the user-visible crash-draft behaviours on its canonical
// page: what the next start does with recovered text (STEP3 §9), what closing
// the last window does when a page still cannot be saved (B-QA), and the
// equal-bytes rule (REG-OG-DRAFTRISK-EQUAL-BYTES).
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const page = readFileSync("crates/tine-core/src/templates/files-external-edits-backups.md", "utf8");

it("explains that recovered text is put back and saved at the next start, named, and never written over a changed file", () => {
  expect(page).toMatch(/next start Tine puts it back on the page and saves it[^\n]*Recovered unsaved edits[^\n]*changed on disk[^\n]*nothing is written over unseen/);
});

it("explains that closing the last window over a page that cannot be saved reopens the graph and names its pages", () => {
  expect(page).toMatch(/close Tine's last window, Tine does not quit over it[^\n]*opens that graph again[^\n]*names each page whose edits are not on disk yet, and why/);
});

it("explains that an outside change equal to the unsaved text is not a conflict and the edit is still saved", () => {
  expect(page).toMatch(/exactly your unsaved text[^\n]*is not a conflict[^\n]*still saves your edit/);
});
