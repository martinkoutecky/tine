import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// GH #430 / #432: the kinds of link are visibly different, and a link to a page
// that does not exist yet is readable. jsdom applies no cascade, so these read
// the stylesheet the app ships and evaluate the colors it computes.

const css = readFileSync("src/styles/app.css", "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
const theme = readFileSync("src/styles/theme.css", "utf8");

function declarations(selector: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [, sel, body] of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (sel.trim() !== selector) continue;
    for (const [, prop, val] of body.matchAll(/([a-z-]+)\s*:\s*([^;]+);?/g)) out[prop] = val.trim().replace(/\s+/g, " ");
  }
  return out;
}

describe("link kinds are visibly different (GH #430)", () => {
  it("a link out of the graph is underlined; a page link and a tag are not", () => {
    const external = declarations(".external-link");
    const underlined = /^var\(--tine-external-link-underline,\s*(?!none)[^)]+\)$/.test(external["border-bottom"] ?? "");
    expect(underlined, `.external-link must draw a solid underline by default (OG common.css .external-link): ${JSON.stringify(external)}`).toBe(true);
    const pageRef = declarations(".page-ref, .tag");
    expect(pageRef["text-decoration"]).toBe("none");
    expect(pageRef["border-bottom"]).toBeUndefined();
  });

  it("a block reference keeps its own underline in the text color", () => {
    expect(declarations(".block-ref")["border-bottom"]).toMatch(/solid/);
  });
});

// --- color math for the missing-page color (sRGB color-mix + WCAG contrast) ---
type RGB = [number, number, number];
const hex = (h: string): RGB => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16)) as RGB;
const mix = (a: RGB, pa: number, b: RGB): RGB => a.map((v, i) => v * pa + b[i] * (1 - pa)) as RGB;
const lum = (c: RGB) => {
  const [r, g, b] = c.map((v) => { const s = v / 255; return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4; });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const contrast = (a: RGB, b: RGB) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };

function palette(mode: "light" | "dark") {
  const block = theme.match(new RegExp(`html\\[data-theme="${mode}"\\]\\s*\\{([^}]*)\\}`))![1];
  const v = (name: string) => {
    const m = block.match(new RegExp(`${name}:\\s*(#[0-9a-fA-F]{6})`));
    if (!m) throw new Error(`${mode} palette has no literal ${name}`);
    return hex(m[1]);
  };
  const accent = v("--accent");
  return { bg: v("--bg-primary"), text: v("--text-title"), secondary: v("--text-secondary"), muted: v("--text-muted"), link: accent };
}

/** The color the missing-page rule resolves to when the token is unset. */
function missingColor(p: ReturnType<typeof palette>): RGB {
  const value = declarations("[data-missing-page]").color;
  const m = value.match(/^var\(--tine-missing-page-color,\s*color-mix\(in srgb,\s*var\(--link-color\)\s+(\d+)%,\s*var\(--text-secondary\)\)\)$/);
  if (!m) throw new Error(`unrecognised missing-page color: ${value}`);
  return mix(p.link, Number(m[1]) / 100, p.secondary);
}

describe("links to missing pages are readable (GH #432)", () => {
  for (const mode of ["light", "dark"] as const) {
    it(`${mode}: text contrast is at least 4.5:1, and the color is neither plain text nor a full link`, () => {
      const p = palette(mode);
      const c = missingColor(p);
      expect(contrast(c, p.bg), `${mode} missing-page contrast (the old muted grey was ${contrast(p.muted, p.bg).toFixed(2)})`).toBeGreaterThanOrEqual(4.5);
      const dist = (a: RGB, b: RGB) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
      expect(dist(c, p.text), "must differ from plain text").toBeGreaterThan(40);
      expect(dist(c, p.link), "must differ from a link to an existing page").toBeGreaterThan(25);
    });
  }

  it("the old muted grey would fail the same check", () => {
    expect(contrast(palette("light").muted, palette("light").bg)).toBeLessThan(4.5);
  });
});
