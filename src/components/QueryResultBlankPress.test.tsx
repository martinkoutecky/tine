// GH #1000: a plain press inside a query's results (beside a result's text,
// between two results, between two pages' results, on "No results", in a table
// cell) reached the handler of the block that holds the query, which opened that
// block in the editor; the results were replaced by its text and the page jumped.
// The press now stays at that block's handler: its default is prevented (so no
// text range forms over the results) and nothing opens.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { pageByName, resetStore, undo } from "../document";
import { editingId, endEdit, startEditing } from "../editorController";
import { installKeybindings } from "../keybindings";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import type { ParsedQuery } from "../editor/queryIr";
import type { RefGroup } from "../types";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { editorIn } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";

beforeAll(initParser);
afterEach(() => {
  endEdit("blur");
  window.getSelection()?.removeAllRanges();
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  document.body.replaceChildren();
});

const QUERY = "{{query (task TODO)}}";
// A query whose text holds "nested" answers nothing; every other query answers `groups`.
let groups: RefGroup[] = [];
let legacyTable = false;
const ALPHA: RefGroup = { page: "Src", kind: "page", blocks: [{ id: "hit", raw: "Alpha result", collapsed: false,
  children: [{ id: "kid", raw: "Alpha child", collapsed: false, children: [] }] }] };
const BETA: RefGroup = { page: "Other", kind: "page", blocks: [{ id: "hit2", raw: "Beta result", collapsed: false, children: [] }] };

beforeEach(() => {
  groups = [ALPHA, BETA];
  legacyTable = false;
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (text) => ({
    query: { anchor: "block", filter: { kind: "raw", text, diagnostic_kind: "not_applicable" }, diagnostics: [],
      source: { kind: "og", original: text, og_options: "" } },
    view: {}, legacy_table: legacyTable,
  }) as ParsedQuery);
  vi.spyOn(backend(), "printQuery").mockResolvedValue("(task TODO)");
  vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
  vi.spyOn(backend(), "queryRun").mockImplementation(async (query) =>
    blockRunResult(JSON.stringify(query).includes("nested") ? [] : groups));
});

const page = (name: string, roots: string[], readOnly = false): FeedPage =>
  ({ name, title: name, kind: "page", roots, preBlock: null, format: "md", readOnly, guide: false });
const block = (id: string, raw: string, pageName: string, children: string[] = [], parent: string | null = null): StoreNode =>
  ({ id, raw, parent, children, page: pageName, collapsed: false });

/** The query block on page Host, its results' pages loaded (Src read-only when asked). */
function loadGraph(hostRaw: string, opts: { srcReadOnly?: boolean; hostReadOnly?: boolean; hitRaw?: string } = {}) {
  setDoc({
    byId: {
      query: block("query", hostRaw, "Host"),
      hit: block("hit", opts.hitRaw ?? "Alpha result", "Src", ["kid"]),
      kid: block("kid", "Alpha child", "Src", [], "hit"),
      hit2: block("hit2", "Beta result", "Other"),
    },
    pages: [page("Host", ["query"], opts.hostReadOnly), page("Src", ["hit"], opts.srcReadOnly), page("Other", ["hit2"])],
    feed: ["Host"], loaded: true,
  });
}

function mountHost() {
  const root = document.createElement("div");
  document.body.append(root);
  // Solid delegates mousedown, so what the page around the block sees is a delegated handler too.
  const seen: Event[] = [];
  const disposeKeys = installKeybindings();
  const disposeBlock = render(() => <div onMouseDown={e => seen.push(e)}>
    <For each={pageByName("Host")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  </div>, root);
  return { root, seen, dispose: () => { disposeKeys(); disposeBlock(); } };
}
const find = (root: ParentNode, selector: string) => vi.waitFor(() => {
  const el = root.querySelector<HTMLElement>(selector);
  expect(el, selector).not.toBeNull();
  return el!;
});
/** A plain press; answers whether its default survived (dispatchEvent's answer). */
function press(el: Element, init: MouseEventInit = {}): boolean {
  const kept = el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, ...init }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  return kept;
}
const settle = async () => { await Promise.resolve(); await new Promise(r => setTimeout(r, 0)); };
const Q = '[data-block-id="query"]';
const loaded = (root: ParentNode) => find(root, `${Q} [data-block-id="hit2"] .block-content`);
function keydown(key: string) {
  window.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
}
function selectText(el: Element, word: string, from: number, to: number) {
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  let text = walker.nextNode() as Text;
  while (text && !text.data.includes(word)) text = walker.nextNode() as Text;
  const range = document.createRange();
  range.setStart(text, from);
  range.setEnd(text, to);
  window.getSelection()!.removeAllRanges();
  window.getSelection()!.addRange(range);
}

describe("a plain press inside a query's results (GH #1000)", () => {
  // (a)
  it.each([
    ["a result row's padding beside its text", `${Q} [data-block-id="kid"] > .block-main`],
    ["a result row's own box, left of its bullet", `${Q} [data-block-id="hit"]`],
    ["a page group's box below its results", `${Q} .live-ref-group`],
    ["the gap under a page group's heading", `${Q} .query-group`],
    ["the results' shell between two page groups", `${Q} .query-results-region`],
  ])("on %s opens nothing and goes no further", async (_label, selector) => {
    loadGraph(QUERY);
    const { root, seen, dispose } = mountHost();
    try {
      await loaded(root);
      const target = await find(root, selector);
      expect(press(target)).toBe(false);
      expect(seen).toHaveLength(0);
      await settle();
      expect(editingId()).toBeNull();
      expect(root.querySelector(`${Q} [data-block-id="hit2"] .block-content`)).not.toBeNull();
    } finally { dispose(); }
  });

  it("on \"No results\" opens nothing and goes no further", async () => {
    groups = [];
    loadGraph(QUERY);
    const { root, seen, dispose } = mountHost();
    try {
      expect(press(await find(root, `${Q} .query-empty`))).toBe(false);
      expect(seen).toHaveLength(0);
      await settle();
      expect(editingId()).toBeNull();
    } finally { dispose(); }
  });

  // (b) the hosts where a range over the output once reached the query's own source
  const HOSTS: [string, string][] = [
    ["a heading", `# ${QUERY}`],
    ["a heading:: property", `${QUERY}\nheading:: 2`],
    ["a heading on a later line", `intro\n# ${QUERY}`],
    ["a quote", `> ${QUERY}`],
    ["a callout", `> [!NOTE]\n> ${QUERY}`],
  ];
  it.each(HOSTS)("in %s: a press on \"No results\" or a table cell is default-prevented and opens nothing", async (_name, raw) => {
    for (const table of [false, true]) {
      groups = table ? [ALPHA] : [];
      legacyTable = table;
      loadGraph(raw);
      const { root, dispose } = mountHost();
      try {
        const target = await find(root, table ? `${Q} .query-table tbody td:not(.qt-page)` : `${Q} .query-empty`);
        expect(press(target), table ? "table cell" : "No results").toBe(false);
        await settle();
        expect(editingId()).toBeNull();
        expect(doc.byId.query.raw).toBe(raw);
      } finally { dispose(); resetSharedQueryResultsForTests(); }
    }
  });

  // (c)
  it("a press directly on a loaded writable result's content wrapper edits that result, not the query", async () => {
    loadGraph(QUERY);
    const { root, dispose } = mountHost();
    try {
      press(await find(root, `${Q} [data-block-id="hit2"] > .block-main .block-content-wrapper`));
      await settle();
      expect(editingId()).toBe("hit2");
    } finally { dispose(); }
  });

  // (d)
  it("a press in a nested query's results beside an open code fence does not reach the fence", async () => {
    const fenced = "```js\ncode\n```\n" + QUERY;
    loadGraph(fenced, { hitRaw: "{{query nested}}" });
    startEditing("query", 7);
    const { root, dispose } = mountHost();
    try {
      const after = await find(root, `${Q} .mixed-code-after`);
      const editor = editorIn(root, "query");
      // The fence's handler, had it seen the press, would leave the code body for the source after it.
      expect(editor.value()).toBe("code");
      press(await find(after, '[data-block-id="hit"] .query-empty'));
      await settle();
      expect(editingId()).toBe("query");
      expect(editor.value()).toBe("code");
    } finally { dispose(); }
  });

  // (f) a range left in generated output survives the press; Delete over it does nothing
  it("a range in a table cell, then a press on another cell, then Delete: the query's text stays", async () => {
    groups = [ALPHA, BETA];
    legacyTable = true;
    const raw = `# ${QUERY}`;
    loadGraph(raw);
    const { root, dispose } = mountHost();
    try {
      await find(root, `${Q} .query-table tbody tr:nth-child(2) td:not(.qt-page)`);
      const cells = root.querySelectorAll(`${Q} .query-table tbody td:not(.qt-page)`);
      selectText(cells[0], "Alpha", 0, 3);
      expect(press(cells[1])).toBe(false);
      keydown("Delete");
      await settle();
      expect(doc.byId.query.raw).toBe(raw);
      expect(editingId()).toBeNull();
    } finally { dispose(); }
  });

  // (g)
  it("a press on a read-only result's content wrapper opens nothing", async () => {
    loadGraph(QUERY, { srcReadOnly: true });
    const { root, dispose } = mountHost();
    try {
      press(await find(root, `${Q} [data-block-id="hit"] > .block-main .block-content-wrapper`));
      await settle();
      expect(editingId()).toBeNull();
    } finally { dispose(); }
  });

  it("a press on a result still drawn from the index, its page not loaded, opens nothing", async () => {
    groups = [{ page: "Cold", kind: "page", blocks: [{ id: "cold", raw: "Gamma result", collapsed: false, children: [] }] }];
    vi.spyOn(backend(), "getPage").mockReturnValue(new Promise(() => {}));
    loadGraph(QUERY);
    const { root, dispose } = mountHost();
    try {
      const cold = await find(root, `${Q} .ref-block`);
      press(cold.querySelector(".block-content-wrapper") ?? cold);
      await settle();
      expect(editingId()).toBeNull();
    } finally { dispose(); }
  });

  // (h) a range in a live result's own text is that result's: Delete edits only it
  it("a range in a result's own text, a press on its group, then Delete: only that text goes, one undo restores it", async () => {
    loadGraph(QUERY);
    const { root, dispose } = mountHost();
    try {
      const content = await find(root, `${Q} [data-block-id="hit2"] > .block-main .block-content`);
      selectText(content, "Beta", 0, 5);
      expect(press(await find(root, `${Q} .query-group:last-child`))).toBe(false);
      keydown("Delete");
      await vi.waitFor(() => expect(doc.byId.hit2.raw).toBe("result"));
      expect(doc.byId.query.raw).toBe(QUERY);
      undo();
      expect(doc.byId.hit2.raw).toBe("Beta result");
      expect(doc.byId.query.raw).toBe(QUERY);
    } finally { dispose(); }
  });

  // Unchanged guards
  it("a press on a result's text still edits that result", async () => {
    loadGraph(QUERY);
    const { root, dispose } = mountHost();
    try {
      press(await find(root, `${Q} [data-block-id="kid"] > .block-main .block-content`));
      await vi.waitFor(() => expect(editingId()).toBe("kid"));
    } finally { dispose(); }
  });

  it.each([
    ["the query's own header", `${Q} .query-header`],
    ["the query block's edge under the results", `${Q} > .block-main .block-content-wrapper`],
  ])("a press on %s still edits the query", async (_label, selector) => {
    loadGraph(QUERY);
    const { root, dispose } = mountHost();
    try {
      await loaded(root);
      press(await find(root, selector));
      await vi.waitFor(() => expect(editingId()).toBe("query"));
    } finally { dispose(); }
  });

  it("a Shift-press in the results is left to the outline selection and opens no result", async () => {
    loadGraph(QUERY);
    const { root, dispose } = mountHost();
    try {
      await loaded(root);
      press(await find(root, `${Q} .query-results-region`), { shiftKey: true });
      await settle();
      expect(editingId()).not.toBe("hit2");
    } finally { dispose(); }
  });

  it.each([
    ["a page group heading", `${Q} .query-page`],
    ["a result's bullet", `${Q} [data-block-id="hit2"] .block-controls`],
  ])("a press on %s keeps its own handler and opens no editor", async (_label, selector) => {
    loadGraph(QUERY);
    const { root, dispose } = mountHost();
    try {
      await loaded(root);
      press(await find(root, selector));
      await settle();
      expect(editingId()).toBeNull();
    } finally { dispose(); }
  });

  it("on a read-only page a press in the results opens nothing", async () => {
    loadGraph(QUERY, { hostReadOnly: true });
    const { root, dispose } = mountHost();
    try {
      await loaded(root);
      press(await find(root, `${Q} .query-results-region`));
      await settle();
      expect(editingId()).toBeNull();
    } finally { dispose(); }
  });
});

// (e) a control inside a live result that handles the press without stopping it
describe.each(["textarea", "cm6"] as const)("a press on the formatting toolbar of a result being edited (%s)", (kind) => {
  useEditorKind(kind);
  it("keeps that result in the editor, its selection kept, and does not open the query", async () => {
    loadGraph(QUERY);
    const { root, dispose } = mountHost();
    try {
      await loaded(root);
      press(await find(root, `${Q} [data-block-id="hit2"] > .block-main .block-content`));
      const editor = await vi.waitFor(() => editorIn(root, "hit2"));
      editor.focus();
      editor.selectByUser(0, 4);
      const toolbar = await find(root, `${Q} [data-block-id="hit2"] .sel-toolbar`);
      press(toolbar);
      await settle();
      expect(editingId()).toBe("hit2");
      expect(editor.selection().slice(0, 2)).toEqual([0, 4]);
    } finally { dispose(); }
  });
});
