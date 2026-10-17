import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { AstBody } from "./body";
import { initParser } from "./parse";
import * as extents from "../editor/queryMacro";
import * as reconciliation from "./queryMacroRender";

vi.mock("../components/Macro", () => ({ QueryMacro: () => <span>Query fixture</span> }));
beforeAll(initParser);
afterEach(() => vi.restoreAllMocks());
it("does no query scanning or reconciliation for 1000 ordinary headings with an unknown macro", () => {
  const scan = vi.spyOn(extents, "queryMacroExtents");
  const reconcile = vi.spyOn(reconciliation, "queryMacroRenderRun");
  const raw = ["Introduction {{unknown x}}", ...Array.from({ length: 1000 }, (_, n) => `## Paragraph ${n} with some ordinary prose.`)].join("\n");
  const root = document.createElement("div");
  const dispose = render(() => <AstBody raw={raw} />, root);
  try {
    expect(root.textContent).toContain("Paragraph 999");
    expect(scan.mock.calls.length).toBe(0);
    expect(reconcile.mock.calls.length).toBe(0);
  } finally { dispose(); }
});
it("scans a raw value once and reconciles only parser-confirmed query flows", () => {
  const scan = vi.spyOn(extents, "queryMacroExtents");
  const reconcile = vi.spyOn(reconciliation, "queryMacroRenderRun");
  const raw = ['{{query (task TODO) {:title "One"}}}', ...Array.from({ length: 1000 }, (_, n) => `## Paragraph ${n} with some ordinary prose.`), '## {{query (task DONE) {:title "Two"}}}'].join("\n");
  const root = document.createElement("div");
  const dispose = render(() => <AstBody raw={raw} />, root);
  try {
    expect(root.textContent?.split("Query fixture")).toHaveLength(3);
    expect(scan.mock.calls.length).toBe(1);
    expect(reconcile.mock.calls.length).toBe(2);
  } finally { dispose(); }
});
