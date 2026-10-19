// GH #726: right-clicking a block inside a query's results opened the block
// menu of the QUERY HOST (the nested row's handler ran, then the host row's
// handler overrode it), so Cut / Delete removed the query block from its page.
// OG resolves the menu target as the NEAREST bullet (og-1.0.0
// frontend/components/content.cljs hiccup-content: `(.closest target
// ".bullet-container[blockid]")` -> block-context-menu-content -> cut-block! /
// delete-block-aux! on that block), i.e. the result's own source block.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { doc, setDoc } from "../document/model";
import { pageByName, resetStore, selectedIds, undo } from "../document";
import { editingId } from "../editorController";
import { installKeybindings } from "../keybindings";
import { loadSingle } from "../document/workingSet";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { clearTransientLayersForTest } from "../transientLayers";
import { clearClipboardPayload } from "../clipboard";
import type { ParsedQuery } from "../editor/queryIr";
import type { BlockDto, RefGroup } from "../types";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { closeContextMenu, contextMenu } from "../ui";

class AllNearObserver implements IntersectionObserver {
  readonly root = null;
  readonly rootMargin = "0px";
  readonly thresholds = [0];
  constructor(private readonly callback: IntersectionObserverCallback) {}
  disconnect(): void {}
  takeRecords(): IntersectionObserverEntry[] { return []; }
  unobserve(): void {}
  observe(target: Element): void {
    this.callback([{ isIntersecting: true, target } as IntersectionObserverEntry], this);
  }
}

beforeAll(initParser);
afterEach(() => {
  closeContextMenu();
  clearClipboardPayload();
  clearTransientLayersForTest();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  resetSharedQueryResultsForTests();
  resetStore();
  document.body.replaceChildren();
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.append(root);
  return { root, dispose: render(node, root) };
}

function rightClick(element: Element): MouseEvent {
  const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 5, clientY: 5 });
  element.dispatchEvent(event);
  return event;
}

function clickMenu(label: string): void {
  const item = [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((el) => el.textContent?.trim() === label);
  expect(item, label).toBeDefined();
  item!.click();
}

describe("query results (GH #726)", () => {
  beforeEach(() => {
    const parsed: ParsedQuery = { query: { anchor: "block", filter: { kind: "and", items: [] }, source: { kind: "builder" } }, view: {} };
    vi.spyOn(backend(), "parseQuery").mockResolvedValue(parsed);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("@block");
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(false);
    vi.spyOn(backend(), "queryRun").mockImplementation(async () => blockRunResult([{ page: "Src", kind: "page", blocks: [
      { id: "hit", raw: "WAITING result block", collapsed: false, marker: "WAITING",
        children: [{ id: "hit-kid", raw: "result child", collapsed: false, children: [] }] },
    ] }]));
  });

  function loadGraph(macro: string) {
    setDoc({
      byId: {
        query: { id: "query", raw: macro, parent: null, children: [], page: "Host", collapsed: false },
        after: { id: "after", raw: "after the query", parent: null, children: [], page: "Host", collapsed: false },
        before: { id: "before", raw: "sibling before", parent: null, children: [], page: "Src", collapsed: false },
        hit: { id: "hit", raw: "WAITING result block", parent: null, children: ["hit-kid"], page: "Src", collapsed: false },
        "hit-kid": { id: "hit-kid", raw: "result child", parent: "hit", children: [], page: "Src", collapsed: false },
      },
      pages: [
        { name: "Host", title: "Host", kind: "page", roots: ["query", "after"], preBlock: null, format: "md", readOnly: false, guide: false },
        { name: "Src", title: "Src", kind: "page", roots: ["before", "hit"], preBlock: null, format: "md", readOnly: false, guide: false },
      ],
      feed: ["Host"], loaded: true,
    });
  }

  const mountHost = () => mount(() => <><ContextMenu /><For each={pageByName("Host")?.roots ?? []}>{(id) => <Block id={id} />}</For></>);

  const resultRow = async (root: HTMLElement, id: string, part: string) => vi.waitFor(() => {
    const el = root.querySelector<HTMLElement>(`[data-block-id="query"] [data-block-id="${id}"] > .block-main ${part}`);
    expect(el).not.toBeNull();
    return el!;
  });

  it.each([
    ["{{tine-query @block}}", "result bullet", "hit", ".bullet-container"],
    ["{{tine-query @block}}", "result text", "hit", ".block-content"],
    ["{{tine-query @block}}", "result child's bullet", "hit-kid", ".bullet-container"],
    ["{{query (task WAITING)}}", "result bullet", "hit", ".bullet-container"],
  ])("%s: right-clicking the %s opens that source block's menu, never the host's", async (macro, _label, id, part) => {
    loadGraph(macro);
    const { root, dispose } = mountHost();
    try {
      const event = rightClick(await resultRow(root, id, part));
      expect(event.defaultPrevented).toBe(true);
      expect(contextMenu()).toMatchObject({ kind: "block", blockId: id });
    } finally { dispose(); }
  });

  it.each(["Cut block", "Delete block"])("%s from a result's menu removes the result on its own page and keeps the query", async (label) => {
    vi.spyOn(backend(), "writeRich").mockResolvedValue();
    loadGraph("{{tine-query @block}}");
    const { root, dispose } = mountHost();
    try {
      rightClick(await resultRow(root, "hit", ".bullet-container"));
      clickMenu(label);
      await vi.waitFor(() => expect(doc.byId.hit).toBeUndefined());
      expect(pageByName("Src")!.roots).toEqual(["before"]);
      expect(pageByName("Host")!.roots).toEqual(["query", "after"]);
      expect(doc.byId.query.raw).toBe("{{tine-query @block}}");
      undo();
      expect(pageByName("Src")!.roots).toEqual(["before", "hit"]);
      expect(doc.byId.query.raw).toBe("{{tine-query @block}}");
    } finally { dispose(); }
  });

  it("a result whose page is not loaded yet (read-only stand-in) never opens the host's menu", async () => {
    vi.spyOn(backend(), "getPage").mockImplementation(() => new Promise(() => {}));
    vi.spyOn(backend(), "getPageByPath").mockImplementation(() => new Promise(() => {}));
    setDoc({
      byId: { query: { id: "query", raw: "{{tine-query @block}}", parent: null, children: [], page: "Host", collapsed: false } },
      pages: [{ name: "Host", title: "Host", kind: "page", roots: ["query"], preBlock: null, format: "md", readOnly: false, guide: false }],
      feed: ["Host"], loaded: true,
    });
    const { root, dispose } = mountHost();
    try {
      const bullet = await vi.waitFor(() => {
        const el = root.querySelector<HTMLElement>('[data-block-id="query"] .ref-block .bullet-container');
        expect(el).not.toBeNull();
        return el!;
      });
      rightClick(bullet);
      expect(contextMenu()).toBeFalsy();
    } finally { dispose(); }
  });

  it("table view: right-clicking a result row never opens the host's block menu", async () => {
    loadGraph("{{tine-query @block}}\ntine.view:: table");
    const { root, dispose } = mountHost();
    try {
      const cell = await vi.waitFor(() => {
        const el = root.querySelector<HTMLElement>('[data-block-id="query"] .sheet-title-cell[data-block-id="hit"]');
        expect(el).not.toBeNull();
        return el!;
      });
      rightClick(cell);
      expect(contextMenu()).toMatchObject({ kind: "sheet-cell", blockId: "hit" });
    } finally { dispose(); }
  });

  it("table view: a row whose page is not loaded yet never opens the host's block menu", async () => {
    vi.spyOn(backend(), "getPage").mockImplementation(() => new Promise(() => {}));
    vi.spyOn(backend(), "getPageByPath").mockImplementation(() => new Promise(() => {}));
    setDoc({
      byId: { query: { id: "query", raw: "{{tine-query @block}}\ntine.view:: table", parent: null, children: [], page: "Host", collapsed: false } },
      pages: [{ name: "Host", title: "Host", kind: "page", roots: ["query"], preBlock: null, format: "md", readOnly: false, guide: false }],
      feed: ["Host"], loaded: true,
    });
    const { root, dispose } = mountHost();
    try {
      const cell = await vi.waitFor(() => {
        const el = root.querySelector<HTMLElement>('[data-block-id="query"] .sheet-title-cell[data-block-id="hit"]');
        expect(el).not.toBeNull();
        return el!;
      });
      rightClick(cell);
      // The table's own (non-destructive) sheet menu may answer; the host's
      // block menu, whose Cut/Delete would remove the query, must not.
      expect(contextMenu()?.kind).not.toBe("block");
    } finally { dispose(); }
  });

  it("legacy query table: right-clicking a block cell never opens the host's block menu", async () => {
    const parsed: ParsedQuery = { query: { anchor: "block", filter: { kind: "and", items: [] }, source: { kind: "builder" } }, view: {}, legacy_table: true } as ParsedQuery;
    vi.mocked(backend().parseQuery).mockResolvedValue(parsed);
    loadGraph("{{query (task WAITING)}}");
    const { root, dispose } = mountHost();
    try {
      const cell = await vi.waitFor(() => {
        const el = root.querySelector<HTMLElement>('[data-block-id="query"] table tbody td');
        expect(el).not.toBeNull();
        return el!;
      });
      rightClick(cell);
      expect(contextMenu()).toBeFalsy();
    } finally { dispose(); }
  });

  it.each(["Delete", "Backspace", "x"])("keyboard: Escape out of a result's editor, then %s, removes the result only", async (key) => {
    vi.spyOn(backend(), "writeRich").mockResolvedValue();
    const disposeKeys = installKeybindings();
    loadGraph("{{tine-query @block}}");
    const { root, dispose } = mountHost();
    try {
      const content = await resultRow(root, "hit", ".block-content");
      content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      const editor = await vi.waitFor(() => {
        const el = root.querySelector<HTMLTextAreaElement>('[data-block-id="query"] [data-block-id="hit"] textarea');
        expect(el).not.toBeNull();
        return el!;
      });
      expect(editingId()).toBe("hit");
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(selectedIds()).toEqual(["hit"]));
      document.dispatchEvent(new KeyboardEvent("keydown", { key, ctrlKey: key === "x", metaKey: key === "x", bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(doc.byId.hit).toBeUndefined());
      expect(pageByName("Src")!.roots).toEqual(["before"]);
      expect(pageByName("Host")!.roots).toEqual(["query", "after"]);
      expect(doc.byId.query.raw).toBe("{{tine-query @block}}");
    } finally { dispose(); disposeKeys(); }
  });

  it("keyboard: Backspace at the start of a result never merges into or edits the query host", async () => {
    const disposeKeys = installKeybindings();
    loadGraph("{{tine-query @block}}");
    const { root, dispose } = mountHost();
    try {
      const content = await resultRow(root, "hit", ".block-content");
      content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      const editor = await vi.waitFor(() => {
        const el = root.querySelector<HTMLTextAreaElement>('[data-block-id="query"] [data-block-id="hit"] textarea');
        expect(el).not.toBeNull();
        return el!;
      });
      editor.setSelectionRange(0, 0);
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Backspace", bubbles: true, cancelable: true }));
      await Promise.resolve();
      expect(doc.byId.query.raw).toBe("{{tine-query @block}}");
      expect(pageByName("Host")!.roots).toEqual(["query", "after"]);
      expect(doc.byId.query.children).toEqual([]);
    } finally { dispose(); disposeKeys(); }
  });

  it("the host's own bullet still opens the host's menu", async () => {
    loadGraph("{{tine-query @block}}");
    const { root, dispose } = mountHost();
    try {
      await resultRow(root, "hit", ".bullet-container");
      rightClick(root.querySelector('[data-block-id="query"] > .block-main > .block-controls .bullet-container')!);
      expect(contextMenu()).toMatchObject({ kind: "block", blockId: "query" });
    } finally { dispose(); }
  });
});

describe("block embeds (GH #726 sweep)", () => {
  const leaf = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, collapsed: false, children });
  beforeEach(() => {
    vi.stubGlobal("IntersectionObserver", AllNearObserver);
    const target = leaf("target", "embedded root", [leaf("kid", "embedded child")]);
    const group: RefGroup = { page: "HostPage", kind: "page", blocks: [{ ...target, children: [] }] };
    vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map((id) => (id === "target" ? group : null)));
    loadSingle({ name: "HostPage", kind: "page", title: "HostPage", pre_block: null, blocks: [leaf("host", "{{embed ((target))}}"), target] });
  });

  const embedded = async (root: HTMLElement, id: string) => vi.waitFor(() => {
    const el = root.querySelector<HTMLElement>(`.embed-block [data-block-id="${id}"] > .block-main .bullet-container`);
    expect(el).not.toBeNull();
    return el!;
  });

  it("a child inside the embed opens its own menu, not the embed host's", async () => {
    const { root, dispose } = mount(() => <For each={pageByName("HostPage")!.roots}>{(id) => <Block id={id} />}</For>);
    try {
      rightClick(await embedded(root, "kid"));
      expect(contextMenu()).toMatchObject({ kind: "block", blockId: "kid" });
    } finally { dispose(); }
  });

  it("the transparent embed's root row stays the occurrence's handle (master GH #514), as before", async () => {
    const { root, dispose } = mount(() => <For each={pageByName("HostPage")!.roots}>{(id) => <Block id={id} />}</For>);
    try {
      rightClick(await embedded(root, "target"));
      expect(contextMenu()).toMatchObject({ kind: "block", blockId: "host" });
    } finally { dispose(); }
  });
});
