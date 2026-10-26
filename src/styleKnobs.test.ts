import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { SETTING_SEARCH, settingMatches } from "./components/settingsSearch";
import { renderEnglish } from "./i18n";
import {
  KNOBS,
  decodeKnobValues,
  knobIsSet,
  knobOutput,
  validKnobValue,
  validateKnobTable,
  type StyleKnob,
} from "./styleKnobs";

// Style knobs (GH #610 item 3, KNOBS-DESIGN gates K1-K5). The pure half: the
// table schema, what may reach CSS, and the guards. The DOM half is
// styleKnobs.render.test.tsx.

/** One knob of every type, as a theme or a later cleanup row would declare them. */
const EVERY_TYPE: StyleKnob[] = [
  { id: "group", type: "heading", level: 2, title: "Group" },
  { id: "flag", type: "class-toggle", title: "Flag" },
  { id: "mode", type: "class-select", title: "Mode", default: "plain", options: [{ label: "Plain", value: "plain" }, { label: "Fancy", value: "fancy" }] },
  { id: "gap", type: "variable-number-slider", title: "Gap", default: 4, min: 0, max: 20, step: 1, format: "px" },
  { id: "line", type: "variable-select", title: "Line", default: "1px solid", options: [{ label: "Solid", value: "1px solid" }, { label: "None", value: "none" }] },
  { id: "tint", type: "variable-color", title: "Tint", default: "#336699", opacity: true },
];
const knob = (id: string) => EVERY_TYPE.find((k) => k.id === id)!;

describe("the knob table (K1)", () => {
  it("the shipped table is valid", () => {
    expect(validateKnobTable(KNOBS)).toEqual([]);
    expect(validateKnobTable(EVERY_TYPE)).toEqual([]);
  });

  it("every shipped variable default is what the stylesheet already does (K3: an untouched knob changes nothing)", () => {
    const css = readFileSync("src/styles/app.css", "utf8").replace(/\/\*[\s\S]*?\*\//g, "").replace(/\s+/g, " ");
    for (const k of KNOBS) {
      if (k.type === "variable-select") {
        expect(css, `app.css must read var(--tine-${k.id}, <default>) with the knob's default as the fallback`).toContain(`var(--tine-${k.id}, ${k.default})`);
      }
      if (k.type === "class-select") {
        for (const option of k.options.filter((o) => o.value !== k.default)) {
          expect(css, `app.css has no rule for the class a non-default option adds: html.tine-${k.id}-${option.value}`).toContain(`.tine-${k.id}-${option.value}`);
        }
      }
    }
  });

  it.each([
    ["a duplicate id", [{ ...knob("flag") }, { ...knob("flag") }], "duplicate id"],
    ["an id that is not a bare name", [{ ...knob("flag"), id: "Flag; x" }], "id must match"],
    ["a select default that is not an option", [{ ...knob("line"), default: "2px solid" }], "not one of the options"],
    ["a declared option value that ends the declaration", [{ ...knob("line"), options: [{ label: "A", value: "1px solid" }, { label: "B", value: "none; } body { display: none" }] }], "option value"],
    ["a declared option value that loads a resource", [{ ...knob("line"), options: [{ label: "A", value: "1px solid" }, { label: "B", value: "url(https://example.com/x)" }] }], "option value"],
    ["unbalanced parentheses in a declared value", [{ ...knob("line"), options: [{ label: "A", value: "1px solid" }, { label: "B", value: "calc(1px" }] }], "option value"],
    ["a class option that is not a plain word", [{ ...knob("mode"), options: [{ label: "A", value: "plain" }, { label: "B", value: "x y" }] }], "option value"],
    ["a slider default outside its range", [{ ...knob("gap"), default: 99 }], "default within"],
    ["a slider unit that is not allowed", [{ ...knob("gap"), format: "px; x" }], "unit"],
    ["a colour default that is not hex", [{ ...knob("tint"), default: "red" }], "hex colour"],
  ] as [string, StyleKnob[], string][])("rejects %s", (_name, table, fragment) => {
    const problems = validateKnobTable(table);
    expect(problems.length).toBeGreaterThan(0);
    expect(problems.join("\n")).toContain(fragment);
  });

  it("rejects a class named twice across knobs", () => {
    const problems = validateKnobTable([
      { id: "a-b", type: "class-toggle", title: "AB" },
      { id: "a", type: "class-select", title: "A", default: "x", options: [{ label: "x", value: "x" }, { label: "b", value: "b" }] },
    ]);
    expect(problems.join("\n")).toContain("tine-a-b is claimed twice");
  });
});

describe("what reaches CSS (K2, K3)", () => {
  it("an untouched table emits nothing", () => {
    expect(knobOutput(KNOBS, {})).toEqual({ css: "", classes: [] });
    expect(knobOutput(EVERY_TYPE, {})).toEqual({ css: "", classes: [] });
  });

  it("a value equal to the default emits nothing and is not a 'set' knob", () => {
    const values = { flag: false, mode: "plain", gap: 4, line: "1px solid", tint: "#336699" };
    expect(knobOutput(EVERY_TYPE, values)).toEqual({ css: "", classes: [] });
    for (const k of EVERY_TYPE) expect(knobIsSet(k, values)).toBe(false);
  });

  it("each type emits its declared piece", () => {
    const out = knobOutput(EVERY_TYPE, { flag: true, mode: "fancy", gap: 12, line: "none", tint: "#AABBCC80" });
    expect(out.classes).toEqual(["tine-flag", "tine-mode-fancy"]);
    expect(out.css).toBe(":root { --tine-gap: 12px; --tine-line: none; --tine-tint: #aabbcc80; }");
  });

  it("a stored string that is not a declared option emits nothing, however it is shaped", () => {
    const attack = "1px; } body { display:none";
    for (const bad of [attack, "1px solid; x", "none ", "NONE", "url(https://example.com/x)", "@import 'x'"]) {
      expect(knobOutput(EVERY_TYPE, { line: bad, mode: bad })).toEqual({ css: "", classes: [] });
    }
  });

  it("a slider takes only an in-range finite number", () => {
    for (const bad of ["12px", "1px; } body { display:none", -1, 21, NaN, Infinity, true, null, {}]) {
      expect(knobOutput(EVERY_TYPE, { gap: bad as never }).css, String(bad)).toBe("");
    }
    expect(knobOutput(EVERY_TYPE, { gap: 0 }).css).toBe(":root { --tine-gap: 0px; }");
    expect(knobOutput(EVERY_TYPE, { gap: 20 }).css).toBe(":root { --tine-gap: 20px; }");
  });

  it("a colour takes only a hex colour that also passes the theme manifest grammar", () => {
    for (const bad of ["red", "rgb(0,0,0)", "#12345", "#12", "#336699; } x { y: z", "url(x)", "#33669900ff", 7, true]) {
      expect(knobOutput(EVERY_TYPE, { tint: bad as never }).css, String(bad)).toBe("");
    }
    // The alpha pair needs `opacity`; a knob without it takes only 3 or 6 digits.
    const plain: StyleKnob[] = [{ id: "ink", type: "variable-color", title: "Ink", default: "#000000" }];
    expect(knobOutput(plain, { ink: "#11223344" }).css).toBe("");
    expect(knobOutput(plain, { ink: "#123" }).css).toBe(":root { --tine-ink: #123; }");
  });

  it("a toggle takes only a boolean", () => {
    for (const bad of ["true", 1, "tine-flag", null]) expect(knobOutput(EVERY_TYPE, { flag: bad as never }).classes).toEqual([]);
    expect(knobOutput(EVERY_TYPE, { flag: true }).classes).toEqual(["tine-flag"]);
  });

  it("whatever is stored, the output is only declared --tine- variables on :root and declared classes", () => {
    const hostile = ["}", "a{b:c}", "1;2", "\\", "/*", "*/", "<style>", "\n", "url(x)", '"', "'", "@import"];
    const values: Record<string, string> = {};
    for (const k of EVERY_TYPE) for (const [i, h] of hostile.entries()) values[`${k.id}${i}`] = h;
    for (const k of EVERY_TYPE) values[k.id] = hostile.join(" ");
    const out = knobOutput(EVERY_TYPE, values);
    expect(out).toEqual({ css: "", classes: [] });
  });

  it("an unknown id is ignored on output", () => {
    expect(knobOutput(EVERY_TYPE, { retired: "x", "retired-2": true })).toEqual({ css: "", classes: [] });
  });

  it("a number reads back as the same validated value", () => {
    expect(validKnobValue(knob("gap"), 7)).toBe(7);
    expect(validKnobValue(knob("gap"), "7")).toBeNull();
    expect(validKnobValue(knob("group"), "anything")).toBeNull();
  });
});

describe("stored values (N4)", () => {
  it("keeps scalar entries, including ids this build does not know", () => {
    expect(decodeKnobValues('{"gap":7,"flag":true,"newer-knob":"x"}')).toEqual({ gap: 7, flag: true, "newer-knob": "x" });
  });

  it("drops nested values and survives garbage", () => {
    expect(decodeKnobValues('{"a":{"b":1},"c":[1],"d":null,"e":"ok"}')).toEqual({ e: "ok" });
    for (const junk of ["", "not json", "[1,2]", "7", "null", '"x"']) expect(decodeKnobValues(junk)).toEqual({});
  });
});

describe("Settings search", () => {
  it("every knob with a control is searchable by its title; headings are not", () => {
    for (const k of KNOBS.filter((entry) => entry.type !== "heading")) {
      const entry = SETTING_SEARCH.find((e) => e.label === k.title);
      expect(entry?.tab, `Settings search has no entry for the knob ${k.title}`).toBe("appearance");
      expect(settingMatches(entry!, typeof k.title === "string" ? k.title : renderEnglish(k.title))).toBe(true);
    }
    expect(SETTING_SEARCH.some((e) => KNOBS.some((k) => k.type === "heading" && k.title === e.label))).toBe(false);
  });
});

// --- guards -----------------------------------------------------------------

function walk(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const full = path.join(dir, name);
    if (statSync(full).isDirectory()) walk(full, out);
    else if (/\.(ts|tsx)$/.test(full) && !/\.test\.tsx?$/.test(full)) out.push(full);
  }
  return out;
}

describe("guards", () => {
  const files = walk("src");

  it("src/styleKnobs.ts is the only writer of #tine-knobs, the knob classes and the stored key (K1, K2)", () => {
    const offenders = files.filter((f) => f !== path.join("src", "styleKnobs.ts") &&
      /["'`]tine-knobs["'`]|STYLE_KNOBS_STYLE_ID|style\.knobs\.v1|STYLE_KNOBS_KEY/.test(readFileSync(f, "utf8")));
    expect(offenders, "only src/styleKnobs.ts may name the #tine-knobs element or the style.knobs.v1 key; go through setKnob / initStyleKnobs").toEqual([]);
  });

  it("the Settings control is rendered from the table, with no per-knob code (K1)", () => {
    const source = readFileSync("src/components/StyleKnobsSettings.tsx", "utf8");
    for (const k of KNOBS) {
      expect(source, `StyleKnobsSettings.tsx must not name the knob ${k.id}; add a row to KNOBS instead`).not.toContain(`"${k.id}"`);
    }
  });

  it("the Guide page names the Fine-tuning section and the device preference key (K5)", () => {
    const guide = readFileSync("crates/tine-core/src/templates/customize-look.md", "utf8");
    expect(existsSync("crates/tine-core/src/templates/customize-look.md")).toBe(true);
    expect(guide).toContain("**Fine-tuning**");
    expect(guide).toContain("style.knobs.v1");
    for (const k of KNOBS.filter((entry) => entry.type !== "heading")) {
      const title = typeof k.title === "string" ? k.title : renderEnglish(k.title);
      expect(guide, `the Guide must name the Settings control "${title}"`).toContain(`**${title}**`);
    }
  });
});
