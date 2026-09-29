import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const css = readFileSync("src/styles/app.css", "utf8");

function rule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`).exec(css)?.[1] ?? "";
}

describe("page-selection UI chrome (GH #328)", () => {
  it.each([".page-trailing-block-target", ".references-header", ".references-header.clickable"])(
    "keeps %s out of native text selections",
    (selector) => {
      expect(rule(selector)).toMatch(/-webkit-user-select:\s*none/);
      expect(rule(selector)).toMatch(/(?:^|;)\s*user-select:\s*none/);
    },
  );
});

describe("page action controls stay out of native text selections (internal 2026-08-25)", () => {
  // Carry-over buttons, the tag-table toggle, the guide copy button and the
  // page-actions trigger all render text labels in the page title row; a dragged
  // selection crossing them must not pick up their label text. The control must
  // stay an ordinary activatable button: only selection is lost.
  it.each([".page-carry-actions", ".tag-table-toggle", ".guide-copy-btn", ".page-actions-trigger"])(
    "keeps %s unselectable without disabling activation",
    (selector) => {
      expect(rule(selector)).toMatch(/-webkit-user-select:\s*none/);
      expect(rule(selector)).toMatch(/(?:^|;)\s*user-select:\s*none/);
      expect(rule(selector)).not.toMatch(/pointer-events:\s*none/);
    },
  );
});
