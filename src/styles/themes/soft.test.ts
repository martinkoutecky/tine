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

function channel(hex: string): [number, number, number] {
  const m = hex.match(/^#([0-9a-f]{6})$/i);
  if (!m) throw new Error(`${hex} is not #rrggbb`);
  return [0, 2, 4].map((i) => parseInt(m[1].slice(i, i + 2), 16) / 255) as [number, number, number];
}
function luminance(hex: string): number {
  const [r, g, b] = channel(hex).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** Text role -> the surfaces it is drawn on. */
const TEXT_ON: Record<string, string[]> = {
  "--ls-title-text-color": ["--ls-primary-background-color", "--ls-secondary-background-color", "--ls-tertiary-background-color", "--ls-page-inline-code-bg-color", "--ls-block-highlight-color"],
  "--ls-primary-text-color": ["--ls-primary-background-color", "--ls-secondary-background-color", "--ls-tertiary-background-color", "--ls-page-inline-code-bg-color", "--ls-block-highlight-color"],
  "--ls-secondary-text-color": ["--ls-primary-background-color", "--ls-secondary-background-color", "--ls-tertiary-background-color", "--ls-page-inline-code-bg-color", "--ls-block-highlight-color"],
  "--ls-link-text-color": ["--ls-primary-background-color", "--ls-secondary-background-color", "--ls-tertiary-background-color", "--ls-page-inline-code-bg-color", "--ls-block-highlight-color"],
  "--ls-tag-text-color": ["--ls-primary-background-color", "--ls-secondary-background-color", "--ls-tertiary-background-color", "--ls-page-inline-code-bg-color", "--ls-block-highlight-color"],
  "--text-muted": ["--ls-primary-background-color", "--ls-secondary-background-color", "--ls-page-inline-code-bg-color", "--ls-block-highlight-color"],
  "--ls-page-mark-color": ["--ls-page-mark-bg-color"],
};

describe("Soft palette (GH #649)", () => {
  for (const mode of ["light", "dark"] as const) {
    const vars = block(soft, mode);
    it(`${mode}: every text role is at least 4.5:1 on the surfaces it sits on`, () => {
      const failures: string[] = [];
      for (const [fg, backgrounds] of Object.entries(TEXT_ON)) {
        for (const bg of backgrounds) {
          const ratio = contrast(resolve(vars, fg), resolve(vars, bg));
          if (ratio < 4.5) failures.push(`${fg} on ${bg}: ${ratio.toFixed(2)}`);
        }
      }
      expect(failures).toEqual([]);
    });

    it(`${mode}: moves the whole surface system together, leaving no stock --ls-* color behind`, () => {
      const stock = [...block(shim, mode).keys()].filter((name) => name.startsWith("--ls-"));
      expect(stock.length).toBeGreaterThan(20);
      expect(stock.filter((name) => !vars.has(name))).toEqual([]);
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
