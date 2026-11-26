import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { galleryThemes } from "./themes";
import { BLOCK_COLOR_NAMES } from "../blockColors";
import { highlightMarkOpen } from "../editor/format";
import { colorOnlyStyles } from "../render/htmlSanitize";

// Text in a coloured highlight (the selection toolbar's colour row) stays readable, at least
// 4.5:1 (WCAG AA), on every shipped theme in both modes: the default theme and each gallery
// palette over it. The colours and the text colour are read from the shipped CSS, so changing
// either re-runs the check.

const shim = readFileSync("src/styles/ls-shim.css", "utf8");
const app = readFileSync("src/styles/app.css", "utf8");
// Each gallery palette's shipped CSS (the `?inline` import is empty under vitest), read from disk.
const palettes = galleryThemes.map((theme) => ({ id: theme.id, css: readFileSync(`src/styles/themes/${theme.id}.css`, "utf8") }));

function block(css: string, mode: "light" | "dark"): Map<string, string> {
  const match = css.match(new RegExp(`html\\[data-theme="${mode}"\\]\\s*\\{([^}]*)\\}`));
  const out = new Map<string, string>();
  if (!match) return out;
  for (const [, name, value] of match[1].matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/g)) out.set(name, value.trim());
  return out;
}

function resolve(vars: Map<string, string>, name: string, seen = new Set<string>()): string {
  const value = vars.get(name);
  if (value === undefined || seen.has(name)) throw new Error(`${name} is not defined`);
  const alias = value.match(/^var\((--[a-z0-9-]+)(?:,\s*([^)]+))?\)$/);
  if (!alias) return value;
  return vars.has(alias[1]) ? resolve(vars, alias[1], new Set(seen).add(name)) : alias[2]!.trim();
}

type Rgb = [number, number, number];
function rgb(color: string): Rgb {
  const hex = color.match(/^#([0-9a-f]{6})$/i);
  if (hex) return [0, 2, 4].map((i) => parseInt(hex[1].slice(i, i + 2), 16)) as Rgb;
  const hsl = color.match(/^hsl\(\s*([\d.]+)\s*,\s*([\d.]+)%\s*,\s*([\d.]+)%\s*\)$/);
  if (!hsl) throw new Error(`${color} is not #rrggbb or hsl()`);
  const [h, s, l] = [Number(hsl[1]) % 360, Number(hsl[2]) / 100, Number(hsl[3]) / 100];
  const f = (n: number) => {
    const k = (n + h / 30) % 12;
    return 255 * (l - s * Math.min(l, 1 - l) * Math.max(-1, Math.min(k - 3, 9 - k, 1)));
  };
  return [f(0), f(8), f(4)];
}
function luminance([r, g, b]: Rgb): number {
  const [R, G, B] = [r, g, b].map((v) => v / 255).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * R + 0.7152 * G + 0.0722 * B;
}
function ratio(a: Rgb, b: Rgb): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

describe("coloured highlight text contrast", () => {
  // The rule app.css draws the colour row's marks with: a substring of the style attribute, and
  // the variable their text is drawn in.
  const rule = app.match(/\nmark\[style\*="([^"]+)"\]\s*\{\s*color:\s*var\((--[a-z0-9-]+)\);/);
  const textVariable = rule![2];
  // The style attribute a mark renders with: what the sanitizer keeps.
  const rendered = (style: string) => colorOnlyStyles([style])[0];

  it("draws the colour row's marks, and only marks with a theme highlight background, in a theme variable", () => {
    expect(rule).toBeTruthy();
    const row = BLOCK_COLOR_NAMES.map((name) => rendered(highlightMarkOpen(name).match(/style="([^"]*)"/)![1]));
    expect(row.every((style) => style.includes(rule![1]))).toBe(true);
    // Merge review of 75e6ab532, finding 2: a mark written with any other background keeps the
    // mark text colour it had (on Solarized light, #073642 text on #073642 otherwise).
    const authored = ["background-color: #073642", "background-color: black", "background-color: rgb(0, 0, 0)",
      "background-color: hsl(0, 0%, 0%)", "background: #000", "color: var(--ls-highlight-color-red)"];
    expect(authored.map(rendered).filter((style) => style?.includes(rule![1]))).toEqual([]);
  });

  it("keeps every other mark's text in the theme's mark text colour", () => {
    expect(app.match(/\nmark \{[^}]*\scolor:\s*var\((--[a-z0-9-]+)\);/)![1]).toBe("--mark-text");
  });

  it("a gallery palette declares a highlight colour only to change it", () => {
    expect(palettes.every((theme) => theme.css.includes("--ls-primary-text-color"))).toBe(true);
    const restated = palettes.flatMap((theme) => (["light", "dark"] as const).flatMap((mode) => {
      const own = block(theme.css, mode), stock = block(shim, mode);
      return BLOCK_COLOR_NAMES.map((name) => `--ls-highlight-color-${name}`)
        // The same colour, written as hex or hsl: every channel within rounding.
        .filter((name) => own.has(name) && rgb(own.get(name)!).every((c, i) => Math.abs(c - rgb(stock.get(name)!)[i]) <= 1))
        .map((name) => `${theme.id} ${mode} ${name}`);
    }));
    expect(restated).toEqual([]);
  });

  const themes: [string, string][] = [["default", ""], ...palettes.map((theme): [string, string] => [theme.id, theme.css])];
  for (const [id, css] of themes) for (const mode of ["light", "dark"] as const) {
    it(`${id} ${mode}: every highlight colour keeps its text at 4.5:1`, () => {
      const vars = new Map([...block(shim, mode), ...block(css, mode)]);
      const text = rgb(resolve(vars, textVariable));
      const low = BLOCK_COLOR_NAMES
        .map((name) => ({ name, contrast: ratio(text, rgb(resolve(vars, `--ls-highlight-color-${name}`))) }))
        .filter(({ contrast }) => contrast < 4.5);
      expect(low).toEqual([]);
    });
  }
});
