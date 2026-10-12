// GH #494: drag a block out of a query result. OG renders block query results
// as ordinary blocks whose bullet drags the RESULT block itself and moves it
// to the drop target (og-1.0.0 frontend/components/block.cljs
// bullet-drag-start / block-drop -> frontend/handler/dnd.cljs move-blocks).
// The query host block must neither move, nor enter its editor, nor be copied.
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { doc, setDoc } from "../document/model";
import { pageByName, resetStore } from "../document";
import { editingId } from "../editorController";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { clearTransientLayersForTest } from "../transientLayers";
import type { ParsedQuery } from "../editor/queryIr";
import { blockRunResult } from "../tests/queryReadingsTestkit";

beforeAll(initParser);
beforeEach(() => {
  const parsed: ParsedQuery = { query: { anchor: "block", filter: { kind: "and", items: [] }, source: { kind: "builder" } }, view: {} };
  vi.spyOn(backend(), "parseQuery").mockResolvedValue(parsed);
  vi.spyOn(backend(), "printQuery").mockResolvedValue("@block");
  vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(false);
  vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult([{ page: "Src", kind: "page", blocks: [
    { id: "hit", raw: "TODO result block", collapsed: false, children: [], marker: "TODO" },
  ] }]));
});
afterEach(() => {
  clearTransientLayersForTest();
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  document.body.replaceChildren();
});

it("dragging a list-view query result's bullet moves that block, not the query", async () => {
  setDoc({
    byId: {
      query: { id: "query", raw: "{{tine-query @block}}", parent: null, children: [], page: "Host", collapsed: false },
      target: { id: "target", raw: "Drop here", parent: null, children: [], page: "Host", collapsed: false },
      before: { id: "before", raw: "Sibling", parent: null, children: [], page: "Src", collapsed: false },
      hit: { id: "hit", raw: "TODO result block", parent: null, children: [], page: "Src", collapsed: false },
    },
    pages: [
      { name: "Host", title: "Host", kind: "page", roots: ["query", "target"], preBlock: null, format: "md", readOnly: false, guide: false },
      { name: "Src", title: "Src", kind: "page", roots: ["before", "hit"], preBlock: null, format: "md", readOnly: false, guide: false },
    ],
    feed: ["Host"], loaded: true,
  });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <For each={pageByName("Host")?.roots ?? []}>{(id) => <Block id={id} />}</For>, host);
  try {
    const bullet = await vi.waitFor(() => {
      const el = host.querySelector<HTMLElement>('[data-block-id="query"] [data-block-id="hit"] .bullet-container');
      expect(el).not.toBeNull();
      return el!;
    });
    bullet.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, clientX: 0, clientY: 0 }));
    expect(editingId()).toBeNull();
    const target = host.querySelector<HTMLElement>(':scope > [data-block-id="target"], [data-block-id="target"]')!;
    Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => target });
    vi.spyOn(target, "getBoundingClientRect").mockReturnValue({ top: 0, bottom: 20, left: 0, right: 400, height: 20, width: 400 } as DOMRect);
    vi.spyOn(target.querySelector<HTMLElement>(".block-main")!, "getBoundingClientRect")
      .mockReturnValue({ top: 0, bottom: 20, left: 0, right: 400, height: 20, width: 400 } as DOMRect);
    document.dispatchEvent(new MouseEvent("mousemove", { clientX: 10, clientY: 18 }));
    document.dispatchEvent(new MouseEvent("mouseup"));
    await vi.waitFor(() => expect(doc.byId.hit?.page).toBe("Host"));
    expect(pageByName("Host")!.roots.filter((id) => id !== "query" && id !== "target").concat(doc.byId.target.children)).toContain("hit");
    expect(pageByName("Src")!.roots).toEqual(["before"]);
    expect(doc.byId.query.raw).toBe("{{tine-query @block}}");
    expect(editingId()).toBeNull();
  } finally {
    dispose();
  }
});
