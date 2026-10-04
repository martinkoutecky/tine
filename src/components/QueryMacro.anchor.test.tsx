// GH #615 current-base verification. The shared Rust printer's anchor round-trip
// is pinned by an_anchor_the_og_form_would_lose_is_not_og_expressible.
// These boundary fixtures exercise the actual builder/save/render interaction;
// jsdom's dev-preview backend does not implement the native query engine.
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { doc, setDoc } from "../document/model";
import { resetStore } from "../document";
import { editingId, startEditing } from "../editorController";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { clearTransientLayersForTest } from "../transientLayers";
import { resetQueryTextOpenForTests } from "../navSettings";
import type { ParsedQuery } from "../editor/queryIr";
import { blockRunResult } from "../tests/queryReadingsTestkit";

beforeAll(initParser);
beforeEach(() => {
  resetQueryTextOpenForTests(false);
  // Exact empty-query fixtures, rather than a second parser or printer.
  const empty = (anchor: "block" | "page"): ParsedQuery => ({
    query: { anchor, filter: { kind: "and", items: [] }, source: { kind: "builder" } }, view: {},
  });
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (text) => {
    if (text.trim() === "@block") return empty("block");
    if (text.trim() === "" || text.trim() === "@page") return empty("page");
    throw new Error(`Unexpected empty-query fixture: ${text}`);
  });
  vi.spyOn(backend(), "printQuery").mockImplementation(async (query, _view, dialect) => {
    if (dialect === "og" && query.anchor === "page") return "";
    return query.anchor === "block" ? "@block" : "@page";
  });
  vi.spyOn(backend(), "queryOgExpressible").mockImplementation(async (query) => query.anchor === "page");
  vi.spyOn(backend(), "queryRun").mockImplementation(async (query) => query.anchor === "block"
    ? blockRunResult([{ page: "Sheet", kind: "page", blocks: [
      { id: "witness", raw: "Block result witness", collapsed: false, children: [] },
    ] }])
    : { anchor: "page", pages: [{ name: "Page result witness", kind: "page", path: "pages/Witness.md", properties: [] }],
      total: 1, exceeded: false, diagnostics: [], report: { supported: true, ran: [], ignored: [] } });
});
afterEach(() => {
  clearTransientLayersForTest();
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  document.body.replaceChildren();
});

for (const entry of ["slash command", "existing OG query", "existing TQL query"] as const) {
  it(`GH #615: ${entry} switches pages to blocks and back, saving and rerendering the chosen unit`, async () => {
    setDoc({ byId: {
      query: { id: "query", raw: entry === "slash command" ? "/query" : entry === "existing OG query" ? "{{query }}" : "{{tine-query @page}}",
        parent: null, children: [], page: "Sheet", collapsed: false },
    }, pages: [{ name: "Sheet", title: "Sheet", kind: "page", roots: ["query"], preBlock: null,
      format: "md", readOnly: false, guide: false }], feed: ["Sheet"], loaded: true });
    if (entry === "slash command") startEditing("query", 6);
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <Block id="query" />, host);
    try {
      if (entry === "slash command") {
        const editor = host.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
        editor.focus();
        editor.setSelectionRange(6, 6);
        editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "y" }));
        const command = await vi.waitFor(() => {
          const item = [...document.querySelectorAll<HTMLElement>(".autocomplete .ac-item")]
            .find(item => item.querySelector(".ac-label")?.textContent === "Query");
          expect(item).toBeDefined();
          return item!;
        });
        command.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
        await vi.waitFor(() => expect(editingId()).toBeNull());
      } else {
        await vi.waitFor(() => expect(host.querySelector(".qs-gear")).not.toBeNull());
        host.querySelector<HTMLButtonElement>(".qs-gear")!.click();
      }
      await vi.waitFor(() => expect(document.querySelector(".qs-anchor-button")?.textContent).toContain("pages"));
      for (const [unit, witness] of [["blocks", "Block result witness"], ["pages", "Page result witness"]] as const) {
        document.querySelector<HTMLButtonElement>(".qs-anchor-button")!.click();
        const option = [...document.querySelectorAll<HTMLElement>(".qs-option")]
          .find(item => item.textContent?.trim().startsWith(unit) && !item.textContent?.includes("pages and blocks"));
        expect(option).toBeDefined();
        option!.click();
        await vi.waitFor(() => expect(doc.byId.query.raw).toBe(unit === "blocks" ? "{{tine-query @block}}" : "{{tine-query @page}}"));
        await vi.waitFor(() => expect(host.querySelector(".qs-sentence")?.textContent?.toLowerCase()).toContain(unit));
        await vi.waitFor(() => expect(host.querySelector(unit === "pages" ? ".query-page-link" : ".query-group")?.textContent).toContain(witness));
      }
    } finally { dispose(); }
  });
}
