import { existsSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { galleryThemeById } from "./index";

// GH #649: "Soft" = Medium Light + Medium Dark. The palette moves the WHOLE
// surface system together and keeps every text role at WCAG AA (4.5:1) on the
// surfaces it sits on. The ratios are computed from the shipped CSS, so editing
// a color in soft.css re-runs the check.

const soft = readFileSync("src/styles/themes/soft.css", "utf8");
const shim = readFileSync("src/styles/ls-shim.css", "utf8");

function block(css: string, mode: "light" | "dark"): Map<string, string> {
  const match = css.match(new RegExp(`html\\[data-theme="${mode}"\\]\\s*\\{([^}]*)\\}`));
  if (!match) throw new Error(`no ${mode} block`);
  const out = new Map<string, string>();
  for (const [, name, value] of match[1].matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/g)) out.set(name, value.trim());
  return out;
}

function resolve(vars: Map<string, string>, name: string, seen = new Set<string>()): string {
  const value = vars.get(name);
  if (value === undefined || seen.has(name)) throw new Error(`${name} is not defined`);
  const alias = value.match(/^var\((--[a-z0-9-]+)\)$/);
  return alias ? resolve(vars, alias[1], new Set(seen).add(name)) : value;
}

type Rgb = [number, number, number];

function parse(color: string): { rgb: Rgb; alpha: number } {
  const hex = color.match(/^#([0-9a-f]{6})$/i);
  if (hex) return { rgb: [0, 2, 4].map((i) => parseInt(hex[1].slice(i, i + 2), 16)) as Rgb, alpha: 1 };
  const fn = color.match(/^rgba\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*([\d.]+)\s*\)$/);
  if (fn) return { rgb: [Number(fn[1]), Number(fn[2]), Number(fn[3])], alpha: Number(fn[4]) };
  throw new Error(`${color} is not #rrggbb or rgba(r, g, b, a)`);
}
/** `top` painted over the opaque `under`, as the browser composites it. */
function over(top: string, under: Rgb): Rgb {
  const { rgb, alpha } = parse(top);
  return rgb.map((c, i) => c * alpha + under[i] * (1 - alpha)) as Rgb;
}
function channel(hex: string): [number, number, number] {
  return parse(hex).rgb.map((c) => c / 255) as [number, number, number];
}
function luminanceOf(rgb: Rgb): number {
  const [r, g, b] = rgb.map((v) => v / 255).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
function luminance(hex: string): number {
  return luminanceOf(parse(hex).rgb);
}
function ratio(a: Rgb, b: Rgb): number {
  const [hi, lo] = [luminanceOf(a), luminanceOf(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const SURFACES = [
  "--ls-primary-background-color",
  "--ls-secondary-background-color",
  "--ls-tertiary-background-color",
  "--ls-page-inline-code-bg-color",
  "--ls-block-highlight-color",
];
/** The page and the sidebars are what selection and match overlays are painted over. */
const UNDER_OVERLAY = ["--ls-primary-background-color", "--ls-secondary-background-color"];
const BODY_TEXT = [
  "--ls-title-text-color",
  "--ls-primary-text-color",
  "--ls-secondary-text-color",
  "--ls-link-text-color",
  "--ls-tag-text-color",
  "--text-muted",
];
/** Hover states (tab close, tab-overview handle) paint --bg-quaternary and switch to
    primary text (app.css .tab-close:hover, .tab-overview-*:hover): muted text never sits on it. */
const ON_QUATERNARY = ["--ls-title-text-color", "--ls-primary-text-color", "--ls-secondary-text-color"];

/** Foreground -> opaque backgrounds it is drawn on. */
const TEXT_ON: Record<string, string[]> = {
  ...Object.fromEntries(BODY_TEXT.map((name) => [name, [...SURFACES]])),
  // Task markers (CANCELED text, DONE text, unchecked checkbox border).
  "--marker-color": SURFACES,
  "--done-color": SURFACES,
  // Highlighted text, and a link inside highlighted text.
  "--ls-page-mark-color": ["--ls-page-mark-bg-color"],
};
for (const name of ON_QUATERNARY) TEXT_ON[name].push("--ls-quaternary-background-color");
TEXT_ON["--ls-link-text-color"].push("--ls-page-mark-bg-color");

/** Overlay background -> foregrounds drawn through it (alpha-blended over the page and sidebar). */
const OVERLAYS: Record<string, string[]> = {
  "--ls-selection-background-color": BODY_TEXT,
  "--ls-a-chosen-bg": BODY_TEXT,
  "--match-mark-bg": ["--ls-primary-text-color", "--ls-title-text-color", "--ls-link-text-color"],
};

/** Filled accent control: the label color against each fill it is drawn on. */
const ON_ACCENT_FILLS = ["--ls-active-primary-color", "--ls-link-text-color"];

describe("Soft palette (GH #649)", () => {
  for (const mode of ["light", "dark"] as const) {
    const vars = block(soft, mode);
    const at = (name: string) => parse(resolve(vars, name)).rgb;

    it(`${mode}: every text role is at least 4.5:1 on the surfaces it sits on`, () => {
      const failures: string[] = [];
      for (const [fg, backgrounds] of Object.entries(TEXT_ON)) {
        for (const bg of backgrounds) {
          const value = ratio(at(fg), at(bg));
          if (value < 4.5) failures.push(`${fg} on ${bg}: ${value.toFixed(2)}`);
        }
      }
      expect(failures).toEqual([]);
    });

    it(`${mode}: text stays at least 4.5:1 through selection, chosen-block and search-match overlays`, () => {
      const failures: string[] = [];
      for (const [overlay, foregrounds] of Object.entries(OVERLAYS)) {
        for (const under of UNDER_OVERLAY) {
          const blended = over(resolve(vars, overlay), at(under));
          for (const fg of foregrounds) {
            const value = ratio(at(fg), blended);
            if (value < 4.5) failures.push(`${fg} on ${overlay} over ${under}: ${value.toFixed(2)}`);
          }
        }
      }
      expect(failures).toEqual([]);
    });

    it(`${mode}: the label on a filled accent control is at least 4.5:1 on every fill it is drawn on`, () => {
      const failures: string[] = [];
      for (const fill of ON_ACCENT_FILLS) {
        const value = ratio(at("--on-accent"), at(fill));
        if (value < 4.5) failures.push(`--on-accent on ${fill}: ${value.toFixed(2)}`);
      }
      expect(failures).toEqual([]);
    });

    it(`${mode}: no filled accent control in the shipped CSS hard-codes a white label`, () => {
      // The label must follow the palette (--on-accent): white on a light accent
      // is 1.6:1.
      const offenders: string[] = [];
      for (const file of ["src/styles/app.css", "src/styles/query.css"]) {
        const css = readFileSync(file, "utf8");
        for (const [, selector, body] of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
          const filled = /background(?:-color)?:\s*var\(--(?:link-color|accent)\b/.test(body);
          if (filled && /(?:^|[;\s])color:\s*#(?:fff|ffffff)\s*;/.test(body)) offenders.push(`${file}: ${selector.trim()}`);
        }
      }
      expect(offenders).toEqual([]);
    });

    it(`${mode}: moves the whole surface system together, leaving no stock --ls-* color behind`, () => {
      const stock = [...block(shim, mode).keys()].filter((name) => name.startsWith("--ls-"));
      expect(stock.length).toBeGreaterThan(20);
      expect(stock.filter((name) => !vars.has(name))).toEqual([]);
    });

    it(`${mode}: sets every Tine color variable that --ls-* does not already drive`, () => {
      // theme.css's per-mode palette minus what ls-shim derives from --ls-*: those
      // are the colors a palette must set itself or they stay stock.
      const own = block(readFileSync("src/styles/theme.css", "utf8"), mode);
      const derived = block(shim, mode);
      const colorish = (value: string) => /^(#[0-9a-f]{6}|rgba?\()/i.test(value);
      const left = [...own]
        .filter(([name, value]) => colorish(value) && !derived.has(name) && !vars.has(name))
        .map(([name]) => name);
      expect(left).toEqual([]);
    });

    it(`${mode}: sits between the stock extremes (a medium surface, not white or near-black)`, () => {
      const page = luminance(resolve(vars, "--ls-primary-background-color"));
      const stockPage = luminance(resolve(block(shim, mode), "--ls-primary-background-color"));
      if (mode === "light") {
        expect(stockPage - page).toBeGreaterThan(0.1);
        expect(page).toBeGreaterThan(0.5);
      } else {
        expect(page / stockPage).toBeGreaterThan(2);
        expect(page).toBeLessThan(0.1);
      }
    });
  }

  it("light is warm-neutral (red >= green >= blue) and dark is neutral (channels within 8/255)", () => {
    const light = channel(resolve(block(soft, "light"), "--ls-primary-background-color"));
    expect(light[0]).toBeGreaterThanOrEqual(light[1]);
    expect(light[1]).toBeGreaterThanOrEqual(light[2]);
    const dark = channel(resolve(block(soft, "dark"), "--ls-primary-background-color"));
    expect((Math.max(...dark) - Math.min(...dark)) * 255).toBeLessThanOrEqual(8);
  });

  it("is registered in the gallery with a thumbnail on disk", () => {
    const theme = galleryThemeById("soft");
    expect(theme?.modes).toEqual(["light", "dark"]);
    expect(theme?.name).toBe("Soft");
    expect(existsSync(`public${theme?.thumbnail}`)).toBe(true);
  });
});
