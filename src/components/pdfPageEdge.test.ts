import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

// PDF9-5 (Martin 2026-10-30, D-20261030-HL): a PDF page is set off by a hairline
// edge, not a blurred box-shadow. Without GPU compositing WebKit repaints the
// scroller on every scroll frame, and the page-sized blur cost ~15-20 ms of a
// ~45 ms frame at DPR 2 (and ~1-2 s of fit-width fast-scroll blank, since pdf.js
// renders one slice per animation frame).

const css = readFileSync("src/styles/pdf-workspace.css", "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

/** Declarations of every top-level rule whose selector list contains `selector` exactly. */
function declarations(selector: string): string[] {
  const out: string[] = [];
  for (const m of css.matchAll(/([^{}@;]+)\{([^{}]*)\}/g)) {
    if (m[1].split(",").some((part) => part.trim() === selector)) out.push(m[2]);
  }
  return out;
}

describe("PDF page edge", () => {
  it("is not a blurred box-shadow on the page", () => {
    const rules = [".pdf-page", ".pdf-viewer .pdf-page", ".pdf-scroll .pdf-page"].flatMap(declarations);
    expect(rules.length).toBeGreaterThan(0);
    for (const body of rules) {
      for (const shadow of body.matchAll(/box-shadow\s*:\s*([^;]+)/g)) {
        const lengths = shadow[1].match(/-?\d*\.?\d+px|(?<![\w.])0(?![\w.])/g) ?? [];
        const blur = lengths[2] ?? "0";
        expect(shadow[1].trim() === "none" || parseFloat(blur) === 0, `blurred page shadow: ${shadow[0]}`).toBe(true);
      }
    }
  });

  it("is a 1px outline in the theme's edge colour, which takes no layout space", () => {
    const [body] = declarations(".pdf-page");
    expect(body).toMatch(/outline:\s*1px solid var\(--pdf-page-edge\)/);
    for (const theme of [".pdf-viewer", '.pdf-viewer[data-theme="light"]', '.pdf-viewer[data-theme="warm"]', '.pdf-viewer[data-theme="dark"]']) {
      expect(declarations(theme).join(";"), theme).toMatch(/--pdf-page-edge:\s*rgba\(/);
    }
  });
});
