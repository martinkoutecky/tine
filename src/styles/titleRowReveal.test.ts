import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const rawCss = readFileSync("src/styles/app.css", "utf8");
const css = rawCss.replace(/\/\*[\s\S]*?\*\//g, "");
const themeCss = readFileSync("src/styles/themePresentation.css", "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

type Rule = { selectors: string[]; body: string };

// Flat rule scan: `a, b { decls }`, also for rules nested in @media/@supports (the at-rule
// prelude is dropped, the inner rule is kept). Good enough for this stylesheet's plain CSS.
function rulesOf(src: string): Rule[] {
  const out: Rule[] = [];
  for (const m of src.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    // Split the list at top-level commas only, so `a > :is(b, c)` stays one selector.
    out.push({ selectors: m[1].split(/,(?![^(]*\))/).map((s) => s.trim().replace(/\s+/g, " ")), body: m[2] });
  }
  return out;
}

const RULES = [...rulesOf(css), ...rulesOf(themeCss)];

function declaration(body: string, prop: string): string | null {
  const m = new RegExp(`(?:^|[;\\s])${prop.replace(/[-]/g, "\\$&")}\\s*:\\s*([^;]*?)\\s*(?:;|$)`).exec(body);
  return m ? m[1] : null;
}

/** The last compound selector, i.e. the element the rule styles. */
const subject = (selector: string) => selector.split(/[\s>+~]+/).filter(Boolean).pop() ?? "";

function baseRule(cls: string): string {
  const r = RULES.find((x) => x.selectors.includes(cls));
  return r?.body ?? "";
}

const KEPT: Array<[string, string]> = [
  [".fav-star.active", "starred page"],
  [`.page-actions-trigger[aria-expanded="true"]`, "menu open"],
  [".tag-table-toggle.active", "table shown"],
];
const FADING = [".page-carry-actions", ".fav-star", ".page-share-btn", ".page-actions-trigger", ".tag-table-toggle"];

// Martin's recording (2026-10-29, development on Linux; the flicker is gone with TINE_GPU=0): moving
// the pointer off a journal title row made the hover-revealed carry buttons (with the star, share and
// actions icons) fade out, come back at full opacity for a frame or two, and vanish again. Round 1
// (`will-change: opacity` on each element) did not cure it on his GPU. The fade now runs on the main
// thread: the elements transition a registered custom property (`--reveal`) and read it as
// `opacity: var(--reveal, 0)`, so no accelerated (compositor) opacity animation and no layer exists.
describe("title-row hover reveals fade through --reveal, not a transition on opacity (carry-button flicker)", () => {
  it("registers --reveal as a non-inherited number starting at 0", () => {
    const block = /@property\s+--reveal\s*\{([^}]*)\}/.exec(css)?.[1] ?? "";
    expect(declaration(block, "syntax")).toMatch(/^["']<number>["']$/);
    expect(declaration(block, "inherits")).toBe("false");
    expect(declaration(block, "initial-value")).toBe("0");
  });

  it.each(FADING)("%s reads opacity from --reveal, hidden when --reveal is absent, and fades --reveal in 0.1s", (selector) => {
    const body = baseRule(selector);
    expect(body).not.toBe("");
    expect(declaration(body, "opacity")).toBe("var(--reveal, 0)");
    const transition = declaration(body, "transition") ?? "";
    expect(transition).toMatch(/(?:^|,\s*)--reveal 0\.1s/);
    expect(transition).not.toMatch(/\bopacity\b/);
    expect(body).not.toMatch(/will-change/);
  });

  it("no rule for a fading element sets opacity directly (only --reveal)", () => {
    for (const rule of RULES) {
      for (const sel of rule.selectors) {
        if (!FADING.some((f) => subject(sel).includes(f))) continue;
        const op = declaration(rule.body, "opacity");
        if (op !== null) expect(op, `${sel} sets opacity directly`).toBe("var(--reveal, 0)");
        expect(declaration(rule.body, "will-change") ?? "", `${sel} promotes a layer`).not.toMatch(/opacity/);
        const tr = declaration(rule.body, "transition") ?? "";
        expect(tr, `${sel} transitions opacity`).not.toMatch(/\bopacity\b/);
      }
    }
  });

  // Every way these controls are revealed still reveals them (end states unchanged): the group's one rule
  // (below) for pointer, focus and touch, and the states that keep one action in view.
  it.each(KEPT)("%s sets --reveal: 1 (%s)", (selector) => {
    const shown = RULES.filter((r) => r.selectors.includes(selector) && declaration(r.body, "--reveal") === "1");
    expect(shown.length).toBeGreaterThan(0);
  });
});

// GH #1025: on a phone the star and the Tables chip appeared only after a tap synthesized hover, while Share and
// the menu showed, because each action decided alone when to show (five rules, two touch conditions).
describe("one rule decides when the page title's actions show (GH #1025)", () => {
  const SHOWN = "--title-actions-shown";
  it("the group shows its actions while the pointer or focus is in the title row, and always without hover", () => {
    const deciders = RULES.filter((r) => declaration(r.body, SHOWN) !== null);
    expect(deciders.flatMap((r) => r.selectors).sort()).toEqual([
      ".page-title-actions",
      ".page-title-row:focus-within .page-title-actions",
      ".page-title-row:hover .page-title-actions",
    ]);
    expect(deciders.every((r) => declaration(r.body, SHOWN) === "1")).toBe(true);
    expect(css).toMatch(/@media \(hover: none\), \(pointer: coarse\)\s*\{\s*\.page-title-actions\s*\{\s*--title-actions-shown:\s*1;\s*\}\s*\}/);
  });

  it("every action takes the group's answer, and no action reveals itself on hover, focus or touch", () => {
    const take = RULES.filter((r) => declaration(r.body, "--reveal") === `var(${SHOWN}, 0)`);
    expect(take).toHaveLength(1);
    const selector = take[0].selectors.join(", ");
    expect(take[0].selectors).toHaveLength(1);
    for (const action of FADING) expect(selector, action).toContain(action);
    expect(selector.startsWith(".page-title-actions > :is(")).toBe(true);
    const kept = new Set(KEPT.map(([selector]) => selector));
    for (const rule of RULES) {
      if (declaration(rule.body, "--reveal") !== "1") continue;
      for (const sel of rule.selectors) {
        if (FADING.some((f) => subject(sel).includes(f))) expect(kept.has(sel), sel).toBe(true);
      }
    }
  });

  it("the actions are one wrapping flex group, not layout-neutral", () => {
    expect(declaration(baseRule(".page-title-actions"), "display")).toBe("flex");
    expect(declaration(baseRule(".page-title-actions"), "flex-wrap")).toBe("wrap");
    // The group is exactly as wide as its buttons would be as items of the row, so it leaves the title's row at
    // the width where the first of them would have.
    for (const rule of RULES.filter((r) => r.selectors.some((sel) => subject(sel) === ".page-title-actions")))
      expect(declaration(rule.body, "gap") ?? "inherit").toBe("inherit");
    for (const rule of RULES.filter((r) => r.selectors.some((sel) => subject(sel) === ".page-title-actions")))
      expect(declaration(rule.body, "display") ?? "flex").toBe("flex");
  });

  // One rule places a journal's group: the row's free space goes after the item before the group (the title, or
  // Today's task summary). So the group ends a row it shares and starts a row of its own, and the summary stays
  // beside the title. A margin of `auto` on the group would push it to the end of its own row too, and spreading
  // the row's items would move the summary to the middle of a wide row.
  it("the group of a journal with carry buttons ends a row it shares and starts a row of its own", () => {
    const journalRow = ".page-title-row:has(> .page-title-actions > .page-carry-actions)";
    const freeSpace = RULES.filter((r) => declaration(r.body, "margin-inline-end") === "auto" && r.selectors.some((s) => s.includes(".page-title-main")));
    expect(freeSpace.map((r) => r.selectors)).toEqual([[`${journalRow} > .page-title-main > :last-child`]]);
    // The editorial Today header is a centred column, not a row: it takes no part in that.
    const editorial = 'html[data-theme-journal-header="editorial"] .page-title-row.journal-today .page-title-main > :last-child';
    expect(RULES.filter((r) => r.selectors.includes(editorial)).map((r) => declaration(r.body, "margin-inline-end"))).toEqual(["0"]);
    for (const r of RULES) {
      if (r.selectors.some((s) => /^\.page-title-row\b/.test(s) && !s.includes(" "))) expect(declaration(r.body, "justify-content"), r.selectors.join(", ")).toBeNull();
      if (!r.selectors.some((s) => /^\.page-(title|carry)-actions\b/.test(subject(s)))) continue;
      expect(declaration(r.body, "margin-inline-start"), r.selectors.join(", ")).not.toBe("auto");
      expect(declaration(r.body, "margin-left"), r.selectors.join(", ")).not.toBe("auto");
    }
  });
});

describe("title-row layers", () => {
  it.each([".page-title-row", ".page-title-main", ".page-title-actions", ".page-section", ".page-title"])(
    "%s is not promoted to a layer",
    (selector) => {
      expect(baseRule(selector)).not.toMatch(/will-change/);
    },
  );
});
