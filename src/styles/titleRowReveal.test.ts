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
    out.push({ selectors: m[1].split(",").map((s) => s.trim().replace(/\s+/g, " ")), body: m[2] });
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

  // Every way these controls are revealed still reveals them (end states unchanged).
  const REVEALS: Array<[string, string]> = [
    [".page-title-row:hover .page-carry-actions", "pointer over the title row"],
    [".page-title-row:focus-within .page-carry-actions", "keyboard focus in the title row"],
    [".page-carry-actions", "touch (hover: none)"],
    [".page-title-row:hover .fav-star", "pointer over the title row"],
    [".fav-star.active", "starred page"],
    [".page-title-row:hover .page-share-btn", "pointer over the title row"],
    [".page-share-btn:focus-visible", "keyboard focus"],
    [".page-share-btn", "coarse pointer"],
    [".page-title-row:hover .page-actions-trigger", "pointer over the title row"],
    [".page-actions-trigger:focus-visible", "keyboard focus"],
    [`.page-actions-trigger[aria-expanded="true"]`, "menu open"],
    [".page-actions-trigger", "coarse pointer"],
    [".page-title-row:hover .tag-table-toggle", "pointer over the title row"],
    [".tag-table-toggle.active", "table shown"],
  ];
  it.each(REVEALS)("%s sets --reveal: 1 (%s)", (selector) => {
    const shown = RULES.filter((r) => r.selectors.includes(selector) && declaration(r.body, "--reveal") === "1");
    expect(shown.length).toBeGreaterThan(0);
  });

  it("the touch cases live inside the matching media queries", () => {
    expect(css).toMatch(/@media \(hover: none\)\s*\{\s*\.page-carry-actions\s*\{\s*--reveal:\s*1;/);
    expect(css).toMatch(/@media \(pointer: coarse\)\s*\{\s*\.page-share-btn\s*\{[^}]*--reveal:\s*1;/);
    expect(css).toMatch(/@media \(pointer: coarse\)\s*\{\s*\.page-actions-trigger\s*\{[^}]*--reveal:\s*1;/);
  });

  it.each([".page-title-row", ".page-title-main", ".page-title-actions", ".page-section", ".page-title"])(
    "%s is not promoted to a layer",
    (selector) => {
      expect(baseRule(selector)).not.toMatch(/will-change/);
    },
  );
});
