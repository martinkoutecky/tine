// D-20261030-EMB (Martin 2026-10-30): block embeds go back to 0.8's one bullet.
// "Where a block sits is per page; what it says is shared." The one bullet of a
// whole-block `{{embed ((uuid))}}` is the source root's row, and that row's
// position on this page is the HOST's: indent/outdent, selection and fold
// commands act on the host; the text and children stay the source's. The root
// fold is the host's `collapsed::` (per page), falling back to the source's.
import { focusedEditor, maybeEditorIn } from "../tests/editorDriver";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { editingId, endEdit } from "../editorController";
import { installKeybindings, runGlobalCommand } from "../keybindings";
import { initParser } from "../render/parse";
import { blockProperty, loadFeed, pageByName, resetStore, selectedIds, setCollapsed, undo } from "../document";
import { doc } from "../document/model";
import { closeContextMenu, contextMenu } from "../ui";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";

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

let disposeKeys: (() => void) | null = null;
beforeAll(initParser);
beforeEach(() => {
  vi.stubGlobal("IntersectionObserver", AllNearObserver);
  disposeKeys = installKeybindings();
});
afterEach(() => {
  disposeKeys?.();
  disposeKeys = null;
  closeContextMenu();
  endEdit("page-navigation");
  resetStore();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

const leaf = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, collapsed: false, children });
const page = (name: string, blocks: BlockDto[]): PageDto => ({ name, title: name, kind: "page", pre_block: null, blocks });

/** Source page with `target` > `kid` > `grandkid`; each named page holds `prev` and a host. */
function load(hosts: Record<string, string>, sourceCollapsed = false) {
  const target = { ...leaf("target", "Embedded root\nid:: target", [leaf("kid", "Embedded child", [leaf("grandkid", "Embedded grandchild")])]), collapsed: sourceCollapsed };
  if (sourceCollapsed) target.raw += "\ncollapsed:: true";
  const source = page("Source", [target]);
  loadFeed([
    ...Object.entries(hosts).map(([name, raw]) => page(name, [leaf(`prev-${name}`, "Before"), leaf(`host-${name}`, raw)])),
    source,
  ]);
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) =>
    ids.map((id) => (id === "target" ? { page: "Source", kind: "page", blocks: [{ ...target, children: [] }] } : null)));
  vi.spyOn(backend(), "getPage").mockResolvedValue(source as never);
  vi.spyOn(backend(), "writeRich").mockResolvedValue();
}

function mount(name: string) {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <><ContextMenu /><For each={pageByName(name)?.roots}>{id => <Block id={id} />}</For></>, root);
  return { root, dispose };
}

const rootRow = (root: HTMLElement) => vi.waitFor(() => {
  const row = root.querySelector<HTMLElement>('.embed-block [data-block-id="target"]');
  expect(row).not.toBeNull();
  return row!;
});

async function editRoot(root: HTMLElement) {
  const content = (await rootRow(root)).querySelector<HTMLElement>(":scope > .block-main .block-content")!;
  content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  return vi.waitFor(() => {
    const editor = maybeEditorIn(root.querySelector<HTMLElement>(".embed-block")!, "target");
    expect(editor).not.toBeNull();
    return editor!;
  });
}

const childVisible = async (root: HTMLElement) => (await rootRow(root)).textContent?.includes("Embedded child") ?? false;
const tick = async () => { for (let i = 0; i < 3; i++) await new Promise((r) => setTimeout(r, 0)); };

describe("D-20261030-EMB: the embed's one bullet stands for the host", () => {
  it("Tab and Shift+Tab on the edited root move the host, and the caret stays in the embed", async () => {
    load({ A: "{{embed ((target))}}" });
    const { root, dispose } = mount("A");
    try {
      const editor = await editRoot(root);
      editor.keydown(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(doc.byId["host-A"].parent).toBe("prev-A"));
      expect(doc.byId.target.parent).toBeNull(); // the source stays where it is
      expect(pageByName("Source")!.roots).toEqual(["target"]);
      expect(editingId()).toBe("target");
      await vi.waitFor(() => expect(focusedEditor()?.value()).toContain("Embedded root"));
      expect(document.activeElement?.closest('[data-block-id="host-A"]')).not.toBeNull();
      focusedEditor()!.keydown(new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(doc.byId["host-A"].parent).toBeNull());
      expect(pageByName("A")!.roots).toEqual(["prev-A", "host-A"]);
      expect(doc.byId.target.parent).toBeNull();
    } finally { dispose(); }
  });

  it("Move up/down on the edited root moves the host on this page; the other page and the source stay put", async () => {
    load({ A: "{{embed ((target))}}", B: "{{embed ((target))}}" });
    const { root, dispose } = mount("A");
    try {
      const editor = await editRoot(root);
      editor.keydown(new KeyboardEvent("keydown", { key: "ArrowUp", altKey: true, shiftKey: true, bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(pageByName("A")!.roots).toEqual(["host-A", "prev-A"]));
      expect(pageByName("B")!.roots).toEqual(["prev-B", "host-B"]);
      expect(pageByName("Source")!.roots).toEqual(["target"]);
      expect(editingId()).toBe("target");
      await vi.waitFor(() => expect(focusedEditor()?.value()).toContain("Embedded root"));
      focusedEditor()!.keydown(new KeyboardEvent("keydown", { key: "ArrowDown", altKey: true, shiftKey: true, bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(pageByName("A")!.roots).toEqual(["prev-A", "host-A"]));
      expect(pageByName("B")!.roots).toEqual(["prev-B", "host-B"]);
      expect(pageByName("Source")!.roots).toEqual(["target"]);
    } finally { dispose(); }
  });

  it("Escape from the root selects the host", async () => {
    load({ A: "{{embed ((target))}}" });
    const { root, dispose } = mount("A");
    try {
      const editor = await editRoot(root);
      editor.keydown(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
      expect(selectedIds()).toEqual(["host-A"]);
    } finally { dispose(); }
  });

  it("a fold command on the selected host folds the embed root on this page only", async () => {
    load({ A: "{{embed ((target))}}" });
    const { root, dispose } = mount("A");
    try {
      const editor = await editRoot(root);
      editor.keydown(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
      // GH #696 one level at a time: the root's open child first (a fold below
      // the root, so the source's), then the root itself (the host's).
      runGlobalCommand("editor/collapse-children"); await tick();
      expect(doc.byId.kid.collapsed).toBe(true);
      expect(blockProperty("host-A", "collapsed")).toBeNull();
      runGlobalCommand("editor/collapse-children"); await tick();
      expect(blockProperty("host-A", "collapsed")).toBe("true");
      expect(doc.byId.target.collapsed).toBe(false);
      expect(await childVisible(root)).toBe(false);
    } finally { dispose(); }
  });

  it("a host with its own text keeps its bullet; the embed root inside keeps the source's", async () => {
    load({ A: "Note {{embed ((target))}}" });
    const { root, dispose } = mount("A");
    try {
      const row = await rootRow(root);
      const hostBullet = root.querySelector('[data-block-id="host-A"] > .block-main > .block-controls .bullet-container');
      expect(hostBullet).not.toBeNull();
      row.querySelector(":scope > .block-main .bullet-container")!
        .dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 }));
      expect(contextMenu()).toMatchObject({ kind: "block", blockId: "target" });
      closeContextMenu();
      const editor = await editRoot(root);
      editor.keydown(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
      expect(selectedIds()).toEqual(["target"]);
    } finally { dispose(); }
  });
});

describe("D-20261030-EMB: root fold per host page, source fold as the fallback", () => {
  it("folds the same block on one page, leaves it open on another, and both survive a reload", async () => {
    load({ A: "{{embed ((target))}}", B: "{{embed ((target))}}" });
    let a = mount("A");
    let b = mount("B");
    try {
      (await rootRow(a.root)).querySelector<HTMLButtonElement>(":scope > .block-main .collapse-toggle")!.click();
      await tick();
      expect(await childVisible(a.root)).toBe(false);
      expect(await childVisible(b.root)).toBe(true);
      expect(blockProperty("host-A", "collapsed")).toBe("true");
      expect(blockProperty("host-B", "collapsed")).toBeNull();
      expect(doc.byId.target.raw).not.toContain("collapsed::");
    } finally { a.dispose(); b.dispose(); }
    document.body.replaceChildren();
    a = mount("A");
    b = mount("B");
    try {
      await tick();
      expect(await childVisible(a.root)).toBe(false);
      expect(await childVisible(b.root)).toBe(true);
      // Page B has no fold of its own: it follows the source.
      setCollapsed("target", true); await tick();
      expect(await childVisible(b.root)).toBe(false);
      (await rootRow(b.root)).querySelector<HTMLButtonElement>(":scope > .block-main .collapse-toggle")!.click();
      await tick();
      expect(await childVisible(b.root)).toBe(true);
      expect(blockProperty("host-B", "collapsed")).toBe("false");
      expect(doc.byId.target.collapsed).toBe(true);
      undo(); await tick();
      expect(blockProperty("host-B", "collapsed")).toBeNull();
    } finally { a.dispose(); b.dispose(); }
  });

  it("a fold below the root writes the source and shows on every page", async () => {
    load({ A: "{{embed ((target))}}", B: "{{embed ((target))}}" });
    const a = mount("A");
    const b = mount("B");
    try {
      const kid = await vi.waitFor(() => {
        const toggle = a.root.querySelector<HTMLButtonElement>('.embed-block [data-block-id="kid"] > .block-main .collapse-toggle');
        expect(toggle).not.toBeNull();
        return toggle!;
      });
      expect(b.root.textContent).toContain("Embedded grandchild");
      kid.click(); await tick();
      expect(doc.byId.kid.collapsed).toBe(true);
      expect(doc.byId.kid.raw).toContain("collapsed:: true");
      expect(blockProperty("host-A", "collapsed")).toBeNull();
      expect(a.root.textContent).not.toContain("Embedded grandchild");
      expect(b.root.textContent).not.toContain("Embedded grandchild");
      undo(); await tick();
      expect(doc.byId.kid.collapsed).toBe(false);
      expect(b.root.textContent).toContain("Embedded grandchild");
    } finally { a.dispose(); b.dispose(); }
  });
});
