// GH #1000: a press on blank space inside a query's results (a result row's
// padding, the gap below a result, the gap between two page groups) bubbled to
// the query host's own mousedown handler, which opened the HOST in the editor
// with the caret at its end; the page then scrolled to that caret, far below the
// result that was clicked. A plain press inside the results belongs to the row
// drawn there, or to nobody, never to the block that holds the query; its header
// still opens it.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { setDoc } from "../document/model";
import { pageByName, resetStore } from "../document";
import { editingId, endEdit } from "../editorController";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import type { ParsedQuery } from "../editor/queryIr";
import { blockRunResult } from "../tests/queryReadingsTestkit";

beforeAll(initParser);
afterEach(() => {
  endEdit("blur");
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  document.body.replaceChildren();
});

describe("blank space inside a query's results (GH #1000)", () => {
  beforeEach(() => {
    const parsed: ParsedQuery = { query: { anchor: "block", filter: { kind: "and", items: [] }, source: { kind: "builder" } }, view: {} };
    vi.spyOn(backend(), "parseQuery").mockResolvedValue(parsed);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("@block");
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(false);
    vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult([
      { page: "Src", kind: "page", blocks: [{ id: "hit", raw: "TODO result block", collapsed: false, marker: "TODO",
        children: [{ id: "kid", raw: "Known so far", collapsed: false, children: [] }] }] },
      { page: "Other", kind: "page", blocks: [{ id: "hit2", raw: "TODO second result", collapsed: false, marker: "TODO", children: [] }] },
    ]));
    setDoc({
      byId: {
        query: { id: "query", raw: "{{tine-query @block}}", parent: null, children: [], page: "Host", collapsed: false },
        hit: { id: "hit", raw: "TODO result block", parent: null, children: ["kid"], page: "Src", collapsed: false },
        kid: { id: "kid", raw: "Known so far", parent: "hit", children: [], page: "Src", collapsed: false },
        hit2: { id: "hit2", raw: "TODO second result", parent: null, children: [], page: "Other", collapsed: false },
      },
      pages: [
        { name: "Host", title: "Host", kind: "page", roots: ["query"], preBlock: null, format: "md", readOnly: false, guide: false },
        { name: "Src", title: "Src", kind: "page", roots: ["hit"], preBlock: null, format: "md", readOnly: false, guide: false },
        { name: "Other", title: "Other", kind: "page", roots: ["hit2"], preBlock: null, format: "md", readOnly: false, guide: false },
      ],
      feed: ["Host"], loaded: true,
    });
  });

  const mountHost = () => {
    const root = document.createElement("div");
    document.body.append(root);
    return { root, dispose: render(() => <For each={pageByName("Host")?.roots ?? []}>{(id) => <Block id={id} />}</For>, root) };
  };
  const find = (root: HTMLElement, selector: string) => vi.waitFor(() => {
    const el = root.querySelector<HTMLElement>(selector);
    expect(el, selector).not.toBeNull();
    return el!;
  });
  const press = (el: Element) => {
    el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  };

  it.each([
    ["a result child's row padding, beside its text", '[data-block-id="query"] [data-block-id="kid"] > .block-main'],
    ["a result row's own box, left of its bullet", '[data-block-id="query"] [data-block-id="hit"]'],
    ["the blank below a page's results", '[data-block-id="query"] .live-ref-group'],
    ["the gap between two page groups", '[data-block-id="query"] .query-results-region'],
  ])("a press on %s never opens the query host in the editor", async (_label, selector) => {
    const { root, dispose } = mountHost();
    try {
      await find(root, '[data-block-id="query"] [data-block-id="hit2"] .block-content');
      press(await find(root, selector));
      await Promise.resolve();
      expect(editingId()).not.toBe("query");
    } finally { dispose(); }
  });

  it("a press on a result child's text still edits that child", async () => {
    const { root, dispose } = mountHost();
    try {
      press(await find(root, '[data-block-id="query"] [data-block-id="kid"] > .block-main .block-content'));
      await vi.waitFor(() => expect(editingId()).toBe("kid"));
    } finally { dispose(); }
  });

  it("a press on the empty-result text in the results does not open the query either", async () => {
    vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult([]));
    const { root, dispose } = mountHost();
    try {
      press(await find(root, '[data-block-id="query"] .query-empty'));
      await Promise.resolve();
      expect(editingId()).toBeNull();
    } finally { dispose(); }
  });

  it("a press on the query's own header still edits the query", async () => {
    const { root, dispose } = mountHost();
    try {
      await find(root, '[data-block-id="query"] [data-block-id="hit2"] .block-content');
      press(await find(root, '[data-block-id="query"] .query-header'));
      await vi.waitFor(() => expect(editingId()).toBe("query"));
    } finally { dispose(); }
  });
});
