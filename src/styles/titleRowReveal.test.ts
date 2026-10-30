import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const css = readFileSync("src/styles/app.css", "utf8");

function rule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`).exec(css)?.[1] ?? "";
}

const FADING = [".page-carry-actions", ".fav-star", ".page-share-btn", ".page-actions-trigger", ".tag-table-toggle"];

// Martin's recording (2026-10-29, development on Linux; the flicker is gone with TINE_GPU=0): moving
// the pointer off a journal title row made the hover-revealed carry buttons (with the star, share and
// actions icons) fade out, come back at full opacity for a frame or two, and vanish again. With GPU
// compositing, the layer that a running opacity transition creates and removes flashes the old
// opacity as the fade ends. The fade stays (Martin's call); each fading element instead keeps a
// standing compositing layer (`will-change: opacity`), so no layer comes and goes. The layer belongs on
// the small fading element, never on the title row or the whole actions strip.
describe("title-row hover reveals keep their fade on a standing compositing layer (carry-button flicker)", () => {
  it.each(FADING)("%s fades and keeps an opacity compositing layer", (selector) => {
    const body = rule(selector);
    expect(body).not.toBe("");
    expect(body).toMatch(/(?:^|;|\s)will-change:\s*opacity\s*;/);
    expect(/(?:^|;|\s)transition:\s*([^;]*);/.exec(body)?.[1] ?? "").toMatch(/\bopacity\b/);
    expect(body).toMatch(/opacity:\s*0\s*;/);
  });

  it.each([".page-title-row", ".page-title-main", ".page-title-actions", ".page-section", ".page-title"])(
    "%s is not promoted to a layer",
    (selector) => {
      expect(rule(selector)).not.toMatch(/will-change/);
    },
  );
});
