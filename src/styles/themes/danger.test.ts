import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { galleryThemes } from "./index";

const css = (name: string) => readFileSync(`src/styles/${name}.css`, "utf8");
const theme = css("theme");
const shim = css("ls-shim");
const consumers = ["query", "capture", "props-panel", "app", "pdf-workspace"];

function variables(source: string, mode: string): Map<string, string> {
  const vars = new Map<string, string>();
  for (const [, selector, body] of source.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (![":root", ":where(:root)", `html[data-theme="${mode}"]`, `:where(html[data-theme="${mode}"])`].includes(selector.trim())) continue;
    for (const [, name, value] of body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) vars.set(name, value.trim());
  }
  return vars;
}
function resolve(value: string, vars: Map<string, string>): string {
  const seen = new Set<string>();
  while (value.includes("var(")) {
    if (seen.has(value)) throw new Error(`unresolvable color: ${value}`);
    seen.add(value);
    value = value.replace(/var\((--[\w-]+)(?:,\s*([^()]+))?\)/g, (_, name, fallback) => {
      const assigned = vars.get(name);
      if (!assigned || assigned === "initial") {
        if (fallback) return fallback;
        throw new Error(`undefined ${name}`);
      }
      return assigned;
    });
  }
  return value;
}
type Rgb = number[];
function rgb(color: string): Rgb {
  const hex = color.match(/^#([0-9a-f]{3}|[0-9a-f]{6})$/i)?.[1];
  if (!hex) throw new Error(`not a hex color: ${color}`);
  const full = hex.length === 3 ? [...hex].map((c) => c + c).join("") : hex;
  return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16));
}
function luminance(color: Rgb): number {
  const [r, g, b] = color.map((v) => v / 255).map((v) => v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  return r * 0.2126 + g * 0.7152 + b * 0.0722;
}
function contrast(a: Rgb, b: Rgb): number {
  const [lo, hi] = [luminance(a), luminance(b)].sort((x, y) => x - y);
  return (hi + 0.05) / (lo + 0.05);
}
function declaration(file: string, selector: string, property: string): string {
  for (const [, selectors, body] of css(file).replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (!selectors.split(",").map((s) => s.trim()).includes(selector)) continue;
    const value = body.match(new RegExp(`(?:^|[;\\s])${property}:\\s*([^;]+)`))?.[1];
    if (value) return value.trim();
  }
  throw new Error(`missing ${file} ${selector} ${property}`);
}

describe("error and danger colors (UI-DANGER-THEME-CONTRAST)", () => {
  // Vitest can stub CSS imports to an empty string. Read the registered files
  // directly, as Soft's contrast tests do, so this checks the shipped palettes.
  const palettes = galleryThemes.map((entry) => ({ ...entry, css: css(`themes/${entry.id}`) }));
  for (const palette of [{ id: "default", css: "", modes: ["light", "dark"] }, ...palettes]) {
    for (const mode of palette.modes) {
      const vars = variables(`${theme}\n${shim}\n${palette.css}`, mode);
      const at = (value: string) => rgb(resolve(value, vars));
      it(`${palette.id} ${mode}: diagnostic text and borders contrast on every host surface`, () => {
        const foreground = at(declaration("query", ".query-diagnostics", "color"));
        const border = at(declaration("query", ".query-text-pane-diagnostic", "border-left").replace(/^2px solid /, ""));
        const failures: string[] = [];
        for (const surface of ["--bg-primary", "--bg-secondary", "--bg-tertiary", "--bg-quaternary", "--code-bg", "--block-highlight"]) {
          const background = at(`var(${surface})`);
          const backgrounds = [background];
          // These actual consumers tint their host surface; composite that tint.
          for (const [file, selector] of [["app", ".plugin-finding-severity.severity-critical"], ["pdf-workspace", ".pdf-load-error"]]) {
            const tint = declaration(file, selector, "background").match(/^rgba\((\d+),\s*(\d+),\s*(\d+),\s*([\d.]+)\)$/);
            if (!tint) throw new Error(`unhandled tint: ${selector}`);
            backgrounds.push(background.map((c, i) => Number(tint[i + 1]) * Number(tint[4]) + c * (1 - Number(tint[4]))));
          }
          for (const bg of backgrounds) {
            const textRatio = contrast(foreground, bg);
            const borderRatio = contrast(border, bg);
            if (textRatio < 4.5 || borderRatio < 3) failures.push(`${surface}: text ${textRatio.toFixed(2)}, border ${borderRatio.toFixed(2)}`);
          }
        }
        expect(failures).toEqual([]);
        // The semantic cue remains recognizably red, without pinning a palette.
        expect(foreground[0]).toBeGreaterThan(foreground[1]);
        expect(foreground[0]).toBeGreaterThan(foreground[2]);
        // This sibling paints the semantic color as a fill. Its icon must
        // follow the mode too: white on the dark-mode pale red is unreadable.
        const trash = ".asset-action-btn.asset-action-trash:hover";
        expect(contrast(at(declaration("app", trash, "color")), at(declaration("app", trash, "background")))).toBeGreaterThanOrEqual(4.5);
      });
    }
  }
  it("all formerly undefined error/danger consumers share the public override", () => {
    const source = consumers.map(css).join("\n") + readFileSync("src/components/PluginsTab.tsx", "utf8");
    expect(source).not.toMatch(/var\(--(?:danger-color|danger|error)\b/);
    for (const file of consumers) expect(css(file)).toContain("var(--tine-danger-color,");
    expect(readFileSync("src/components/PluginsTab.tsx", "utf8")).toContain("var(--tine-danger-color,");
  });
  it("the Guide explains the public danger override and its theme default", () => {
    const guide = readFileSync("crates/tine-core/src/templates/customize-look.md", "utf8");
    expect(guide).toContain("`--tine-danger-color`");
    expect(guide).toContain("Error and destructive-action text and error borders");
  });
});
