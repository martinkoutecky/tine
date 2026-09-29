// I-22 (og C3 L16): user `:macros` come from config.edn, which Syncthing or
// another device may deliver. The depth cap bounds nesting, not fan-out, so a
// branching template expanded ~10^12 times and hung the render thread. One
// budget (`expansionBudget.ts`) now bounds each top-level expansion tree in
// both the DOM renderer and the rendered-text walk; hostile shapes are paired
// with benign extremes that must still render in full.
import { describe, it, expect, beforeAll, afterEach } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { setGraphMeta } from "../graphSession";
import { renderedBlockText, type RenderedTextOptions } from "./renderedText";
import { MACRO_EXPANSION_LIMIT_LABEL, MAX_MACRO_EXPANSIONS } from "./expansionBudget";

beforeAll(async () => {
  await initParser();
});

afterEach(() => setGraphMeta(null));

function html(node: () => JSX.Element): string {
  const div = document.createElement("div");
  const dispose = render(() => node(), div);
  const out = div.innerHTML;
  dispose();
  return out;
}

const count = (text: string, needle: string) => text.split(needle).length - 1;

const TEXT: RenderedTextOptions = { typographicGlyphs: false, stripLinks: false, removeTags: false, removeProperties: false };

describe("user macro expansion budget (I-22)", () => {
  it("bounds a branching self-expansion and shows the limit marker", () => {
    setGraphMeta({ macros: { m: "{{m}}{{m}}" } } as never);
    const out = html(() => <AstBody raw="{{m}}" />);
    expect(out).toContain(MACRO_EXPANSION_LIMIT_LABEL);
    // 2^12 leaves without a budget; the budget keeps the tree near its size.
    expect(count(out, "{{m}}")).toBeLessThanOrEqual(2 * MAX_MACRO_EXPANSIONS + 2);
  });

  it("finishes a ten-way fan-out that would expand 10^12 times", () => {
    setGraphMeta({ macros: { m: "{{m}}".repeat(10) } } as never);
    const started = Date.now();
    const out = html(() => <AstBody raw="{{m}}" />);
    expect(out).toContain(MACRO_EXPANSION_LIMIT_LABEL);
    expect(Date.now() - started).toBeLessThan(20_000);
  });

  it("renders a deep but narrow chain in full (benign extreme)", () => {
    const macros: Record<string, string> = { a11: "DEEP-LEAF" };
    for (let i = 0; i < 11; i++) macros[`a${i}`] = `{{a${i + 1}}}`;
    setGraphMeta({ macros } as never);
    const out = html(() => <AstBody raw="{{a0}}" />);
    expect(out).toContain("DEEP-LEAF");
    expect(out).not.toContain(MACRO_EXPANSION_LIMIT_LABEL);
  });

  it("renders a wide but shallow expansion in full (benign extreme)", () => {
    setGraphMeta({ macros: { w: "{{x}} ".repeat(200), x: "X" } } as never);
    const out = html(() => <AstBody raw="{{w}}" />);
    expect(count(out, "X")).toBe(200);
    expect(out).not.toContain(MACRO_EXPANSION_LIMIT_LABEL);
  });

  it("gives every top-level macro its own budget", () => {
    setGraphMeta({ macros: { w: "{{x}} ".repeat(600), x: "X" } } as never);
    const out = html(() => <AstBody raw="{{w}} {{w}}" />);
    expect(count(out, "X")).toBe(1200);
    expect(out).not.toContain(MACRO_EXPANSION_LIMIT_LABEL);
  });
});

describe("rendered-text expansion budget (I-22)", () => {
  const macros = (table: Record<string, string>): RenderedTextOptions => ({
    ...TEXT,
    resolveMacro: (name) => (name in table ? { raw: table[name], format: "md" } : null),
  });

  it("bounds a branching macro and marks the cut", () => {
    const started = Date.now();
    const out = renderedBlockText("{{m}}", "md", macros({ m: "{{m}}".repeat(10) }));
    expect(out).toContain(MACRO_EXPANSION_LIMIT_LABEL);
    expect(Date.now() - started).toBeLessThan(20_000);
  });

  it("bounds branching block references", () => {
    const id = "00000000-0000-4000-8000-000000000001";
    const out = renderedBlockText(`((${id}))`, "md", {
      ...TEXT,
      resolveBlockRefsFully: true,
      resolveBlockRef: () => ({ raw: `((${id})) `.repeat(10), format: "md" }),
    });
    expect(out).toContain(MACRO_EXPANSION_LIMIT_LABEL);
  });

  it("expands deep-narrow and wide-shallow macros in full", () => {
    const chain: Record<string, string> = { a11: "DEEP-LEAF" };
    for (let i = 0; i < 11; i++) chain[`a${i}`] = `{{a${i + 1}}}`;
    expect(renderedBlockText("{{a0}}", "md", macros(chain))).toBe("DEEP-LEAF");
    const wide = renderedBlockText("{{w}}", "md", macros({ w: "{{x}} ".repeat(200).trim(), x: "X" }));
    expect(count(wide, "X")).toBe(200);
    expect(wide).not.toContain(MACRO_EXPANSION_LIMIT_LABEL);
  });
});
