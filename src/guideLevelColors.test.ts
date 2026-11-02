import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { KNOBS, knobOutput } from "./styleKnobs";

// GH #856: indent guides colored by nesting level. jsdom has no cascade for custom
// properties, so the rotation is checked on the stylesheet text: the declarations
// are read from app.css and run down a tree of nested `.block-children`. The pixels
// are covered by scripts/shot-guide-level-colors.mjs (computed colors per level,
// the knob off, a zoomed block and an embed).

const appCss = readFileSync("src/styles/app.css", "utf8");
const levelsCss = readFileSync("src/styles/guideLevels.css", "utf8");
const themeCss = readFileSync("src/styles/theme.css", "utf8");

/** The `--name: value` declarations of the rules whose selector is exactly `selector`. */
function declarations(selector: string): Record<string, string> {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const rules = [...levelsCss.matchAll(new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`, "g"))];
  expect(rules.length, `guideLevels.css must have a rule for ${selector}`).toBeGreaterThan(0);
  const out: Record<string, string> = {};
  for (const rule of rules) for (const [, name, value] of rule[1].matchAll(/(--[\w-]+):\s*([^;]+);/g)) out[name] = value.trim();
  return out;
}

describe("level-colored indent guides (GH #856)", () => {
  it("the knob is on by default: it emits nothing untouched, and only 'single' adds the off class", () => {
    const knob = KNOBS.find((entry) => entry.id === "guide-colors");
    expect(knob?.type).toBe("class-select");
    expect(knobOutput(KNOBS, {})).toEqual({ css: "", classes: [] });
    expect(knobOutput(KNOBS, { "guide-colors": "level" })).toEqual({ css: "", classes: [] });
    expect(knobOutput(KNOBS, { "guide-colors": "single" })).toEqual({ css: "", classes: ["tine-guide-colors-single"] });
    expect(knobOutput(KNOBS, { "guide-colors": "rainbow" })).toEqual({ css: "", classes: [] });
  });

  it("both color schemes define six hues and a strength; the finished level colors derive from the theme's own guide color", () => {
    // Once in the light block, once in the dark block.
    for (let n = 1; n <= 6; n++) expect(themeCss.match(new RegExp(`--guide-hue-${n}: #[0-9a-f]{6};`, "g")), `--guide-hue-${n}`).toHaveLength(2);
    expect(themeCss.match(/--guide-level-strength: \d+%;/g)).toHaveLength(2);
    const root = declarations(":root");
    for (let n = 1; n <= 6; n++) {
      expect(root[`--guide-level-${n}`]).toBe(`color-mix(in srgb, var(--guide-hue-${n}) var(--guide-level-strength, 48%), var(--guide-color))`);
    }
  });

  it("the six colors rotate one step per nesting level, and cycle after six", () => {
    const start = declarations(":root, .live-ref-group, .embed-block");
    const container = declarations(".block-children-container");
    const children = declarations(".block-children");
    // Run the cascade: the container reads the colors handed down, the children paint the first and hand them on.
    let handed: Record<string, string> = { ...start };
    const painted: string[] = [];
    for (let depth = 1; depth <= 14; depth++) {
      const read: Record<string, string> = {};
      for (const [name, value] of Object.entries(container)) read[name] = handed[/var\((--[\w-]+)\)/.exec(value)![1]];
      const own = (value: string) => read[/var\((--[\w-]+)\)/.exec(value)![1]];
      painted.push(own(children["--guide-line"]));
      const next: Record<string, string> = {};
      for (const [name, value] of Object.entries(children)) if (name.startsWith("--gl-c")) next[name] = own(value);
      handed = { ...handed, ...next };
    }
    const level = (n: number) => `var(--guide-level-${n})`;
    expect(painted.slice(0, 7)).toEqual([1, 2, 3, 4, 5, 6, 1].map(level));
    painted.forEach((color, i) => expect(color).toBe(level((i % 6) + 1)));
  });

  it("a foreign outline (embed, Linked References, query result, block references) starts again at level 1", () => {
    // `.live-ref-group` and `.embed-block` re-declare the starting colors; a leaf block declares nothing.
    expect(declarations(":root, .live-ref-group, .embed-block")["--gl-c1"]).toBe("var(--guide-level-1)");
    expect(levelsCss).not.toMatch(/\.block-main\s*\{[^}]*--gl-/);
    expect(levelsCss).not.toMatch(/\.ls-block\s*\{[^}]*--gl-/);
  });

  it("the base rule still reads one variable, so every existing override of the guide keeps winning", () => {
    expect(appCss).toMatch(/\.block-children \{\s*border-inline-start: 1px solid var\(--guide-line, var\(--guide-color\)\);/);
    // The off class swaps the variable back to the single guide color, nothing else.
    expect(levelsCss).toMatch(/html\.tine-guide-colors-single \.block-children \{\s*--guide-line: var\(--guide-color\);\s*\}/);
    // No selector with more specificity than the base rule paints the level color.
    expect(levelsCss).not.toMatch(/border-inline-start-color: var\(--gl-/);
  });
});
