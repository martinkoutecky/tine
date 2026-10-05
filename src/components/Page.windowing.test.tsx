import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { CollapseSurfaceContext, type CollapseSurfaceApi } from "./Block";
import { backend } from "../backend";
import { extendSelectionTo, node, pageByName, resetStore, selectedIds, selectBlock } from "../document";
import { editingId, endEdit, startEditing } from "../editorController";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { resetNearObserverForTests } from "../lazyObserve";
import { revealOutlineBlock } from "../outlineViewport";
import { openInPageFind, setInPageFindQuery, inPageFindMatches, closeInPageFind } from "../inpageFind";
import { installKeybindings } from "../keybindings";
import { PageView } from "./Page";

class WindowObserver {
  static observers: WindowObserver[] = [];
  targets = new Set<Element>();
  constructor(private callback: IntersectionObserverCallback) { WindowObserver.observers.push(this); }
  observe(target: Element) { this.targets.add(target); }
  unobserve(target: Element) { this.targets.delete(target); }
  disconnect() { this.targets.clear(); }
  set(target: Element, near: boolean) {
    this.callback([{ target, isIntersecting: near } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
  }
  static set(target: Element, near: boolean) {
    for (const observer of this.observers) if (observer.targets.has(target)) observer.set(target, near);
  }
}

afterEach(() => {
  closeInPageFind({ restoreFocus: false });
  endEdit("page-navigation");
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  WindowObserver.observers = [];
  resetNearObserverForTests(); resetStore(); resetTabsToJournals();
  document.body.replaceChildren();
});

async function mount(count = 2000, nested = false, branched = false, collapse?: CollapseSurfaceApi) {
  vi.stubGlobal("IntersectionObserver", WindowObserver);
  const blocks = Array.from({ length: count }, (_, i) => ({
    id: `window-${i}`, has_id: false, raw: `Row ${i} unique content`, children: i === 0 ? [{ id: "child", has_id: false, raw: "Child", children: [], collapsed: false }] : [], collapsed: false,
  }));
  const dto = {
    id: "pages/Window.md", name: "Window", title: "Window", kind: "page" as const, pre_block: null,
    blocks: branched ? Array.from({ length: 50 }, (_, i) => ({ id: `branch-${i}`, raw: `Branch ${i}`, collapsed: false, children: blocks.slice(i * 40, (i + 1) * 40) }))
      : nested ? [{ id: "window-parent", raw: "Parent", collapsed: false, children: blocks }] : blocks,
  };
  vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Window", pageKind: "page" });
  const pane = document.createElement("div"); pane.dataset.paneId = "main";
  const host = document.createElement("div"); host.className = "main-content";
  pane.append(host); document.body.append(pane);
  const disposeRender = render(() => <CollapseSurfaceContext.Provider value={collapse ?? null}><PageView /></CollapseSurfaceContext.Provider>, host);
  const disposeKeys = installKeybindings();
  const dispose = () => { disposeKeys(); disposeRender(); };
  await vi.waitFor(() => expect(host.querySelector(".page-title")?.textContent).toContain("Window"));
  return { host, dispose };
}

it("GH #623: the routed 2,000-block page mounts bounded shells, including its first usable row", async () => {
  const { host, dispose } = await mount();
  try {
    expect(host.querySelector('[data-block-id="window-0"]')).not.toBeNull();
    expect(host.querySelectorAll(".ls-block").length).toBeLessThan(100);
    expect(node("window-1999")?.raw).toBe("Row 1999 unique content");
    expect(host.querySelector('[data-block-id="window-1999"]')).toBeNull();
  } finally { dispose(); }
});

it("a 2,000-block tree with only small sibling lists also bounds routed shells and reveals descendants", async () => {
  const { host, dispose } = await mount(2000, false, true);
  try {
    expect(host.querySelectorAll(".ls-block").length).toBeLessThan(100);
    revealOutlineBlock("window-1999", host);
    expect(host.querySelector('[data-block-id="window-1999"]')).not.toBeNull();
    expect(host.querySelectorAll(".ls-block").length).toBeLessThan(150);
  } finally { dispose(); }
});

it("occurrence-local collapse compacts large outline geometry without changing the source", async () => {
  const [folded, setFolded] = createSignal(false);
  const collapse: CollapseSurfaceApi = {
    collapsed: (_id, stored) => folded() || stored,
    toggle: (_id, current) => setFolded(!current),
    setMany: (_ids, closed) => setFolded(closed),
  };
  const { host, dispose } = await mount(2000, false, true, collapse);
  try {
    expect(host.querySelectorAll('[data-outline-window] > [aria-hidden="true"]').length).toBeGreaterThan(0);
    collapse.setMany(pageByName("Window")!.roots, true);
    expect(host.querySelectorAll(".ls-block")).toHaveLength(50);
    expect(host.querySelector('[data-block-id="window-0"]')).toBeNull();
    expect(host.querySelectorAll('[data-outline-window] > [aria-hidden="true"]')).toHaveLength(0);
    expect(node("branch-0")?.collapsed).toBe(false);
    collapse.setMany(pageByName("Window")!.roots, false);
    revealOutlineBlock("window-1999", host);
    expect(host.querySelector('[data-block-id="window-1999"]')).not.toBeNull();
    expect(host.querySelectorAll(".ls-block").length).toBeLessThan(150);
  } finally { dispose(); }
});

it("click-to-edit retains the clicked instance when editor state changes the page's root getter", async () => {
  const { host, dispose } = await mount();
  try {
    const content = host.querySelector<HTMLElement>('[data-block-id="window-0"] .block-content-wrapper')!;
    content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    await vi.waitFor(() => expect(host.querySelector('[data-block-id="window-0"] textarea.block-editor')).not.toBeNull());
    expect(editingId()).toBe("window-0");
  } finally { dispose(); }
});

it("revealing an already-visible row does not pin its window after scrolling away", async () => {
  const { host, dispose } = await mount();
  try {
    const window = host.querySelector<HTMLElement>("[data-outline-window]")!;
    revealOutlineBlock("window-0", host);
    WindowObserver.set(window, false);
    expect(host.querySelector('[data-block-id="window-0"]')).toBeNull();
  } finally { dispose(); }
});

it("model edit intent mounts an unrendered destination and keeps its caret alive outside the window", async () => {
  const { host, dispose } = await mount();
  try {
    startEditing("window-1000", 4);
    await vi.waitFor(() => expect(host.querySelector('[data-block-id="window-1000"] textarea')).not.toBeNull());
    const editor = host.querySelector<HTMLTextAreaElement>('[data-block-id="window-1000"] textarea')!;
    expect(editor.selectionStart).toBe(4);
    const window = editor.closest<HTMLElement>("[data-outline-window]")!;
    WindowObserver.set(window, false);
    expect(editor.isConnected).toBe(true);
    expect(editingId()).toBe("window-1000");
    endEdit("page-navigation");
  } finally { dispose(); }
});

it("Shift selection across an unrendered range keeps model membership and mounts the focus on reveal", async () => {
  const { host, dispose } = await mount();
  try {
    selectBlock("window-20"); extendSelectionTo("window-1000");
    expect(selectedIds()).toHaveLength(981);
    expect(revealOutlineBlock("window-1000", host)).toBe(true);
    expect(host.querySelector('[data-block-id="window-1000"] .selected')).not.toBeNull();
    expect(host.querySelectorAll(".ls-block").length).toBeLessThan(100);
  } finally { dispose(); }
});

it("Find searches all model rows and renders a far-away match before scrolling it", async () => {
  const { host, dispose } = await mount();
  try {
    Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
    openInPageFind(); setInPageFindQuery("Row 1999 unique");
    expect(inPageFindMatches().map((m) => m.blockId)).toEqual(["window-1999"]);
    await vi.waitFor(() => expect(host.querySelector('[data-block-id="window-1999"]')).not.toBeNull());
  } finally { dispose(); }
});

it("an outline whose windows are all offscreen still participates in browser print", async () => {
  const { host, dispose } = await mount();
  try {
    WindowObserver.set(host.querySelector("[data-outline-window]")!, false);
    expect(host.querySelectorAll(".ls-block")).toHaveLength(0);
    window.dispatchEvent(new Event("beforeprint"));
    expect(host.querySelectorAll(".ls-block")).toHaveLength(2001);
    expect(host.querySelectorAll(".ast-deferred")).toHaveLength(0);
    window.dispatchEvent(new Event("afterprint"));
    expect(host.querySelectorAll(".ls-block")).toHaveLength(0);
  } finally { window.dispatchEvent(new Event("afterprint")); dispose(); }
});

it.each([60, 2000])("print mounts the complete outline and returns to bounded shells afterward (%i rows)", async (count) => {
  const { host, dispose } = await mount(count);
  try {
    window.dispatchEvent(new Event("beforeprint"));
    expect(host.querySelectorAll(".ls-block")).toHaveLength(count + 1);
    expect(host.querySelectorAll(".ast-deferred")).toHaveLength(0);
    expect(host.querySelector(`[data-block-id="window-${count - 1}"]`)?.textContent).toContain(`Row ${count - 1} unique content`);
    window.dispatchEvent(new Event("afterprint"));
    expect(host.querySelectorAll(".ls-block").length).toBeLessThan(100);
  } finally { window.dispatchEvent(new Event("afterprint")); dispose(); }
});

it("collapse/expand and zoom retain a large child outline with model-driven descendant reveal", async () => {
  const { host, dispose } = await mount(2000, true);
  try {
    expect(host.querySelector('[data-block-id="window-1999"]')).toBeNull();
    host.querySelector<HTMLElement>('[data-block-id="window-parent"] > .block-main .collapse-toggle')!.click();
    expect(node("window-parent")?.collapsed).toBe(true);
    expect(host.querySelector('[data-block-id="window-0"]')).toBeNull();
    host.querySelector<HTMLElement>('[data-block-id="window-parent"] > .block-main .collapse-toggle')!.click();
    expect(node("window-parent")?.collapsed).toBe(false);
    mainPaneRouter.focusBlock("window-parent");
    await vi.waitFor(() => expect(host.querySelector(".zoomed-page")).not.toBeNull());
    revealOutlineBlock("window-1999", host);
    expect(host.querySelector('[data-block-id="window-1999"]')).not.toBeNull();
    expect(host.querySelectorAll(".ls-block").length).toBeLessThan(100);
  } finally { dispose(); }
});

it("openPageAtBlock reveals a far-down shell through the real router", async () => {
  const { host, dispose } = await mount();
  try {
    expect(host.querySelector('[data-block-id="window-1999"]')).toBeNull();
    mainPaneRouter.openPageAtBlock({ name: "Window", pageKind: "page", block: "window-1999" });
    await vi.waitFor(() => expect(host.querySelector('[data-block-id="window-1999"].block-flash')).not.toBeNull());
  } finally { dispose(); }
});

it.each(["ArrowDown", "ArrowUp", "Enter", "Backspace", "Tab"])("%s keeps the live editor across a shell-window edge", async (key) => {
  const { host, dispose } = await mount();
  try {
    const source = key === "ArrowUp" || key === "Backspace" || key === "Tab" ? "window-23" : "window-22";
    startEditing(source, key === "Backspace" || key === "ArrowUp" ? 0 : node(source)!.raw.length);
    await vi.waitFor(() => expect(host.querySelector("textarea.block-editor")).not.toBeNull());
    const textarea = host.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key, code: key, bubbles: true, cancelable: true }));
    await vi.waitFor(() => {
      const current = editingId();
      expect(current).not.toBeNull();
      expect(host.querySelector(`[data-block-id="${current}"] textarea.block-editor`)).not.toBeNull();
      if (key === "ArrowDown") expect(current).toBe("window-23");
      if (key === "ArrowUp" || key === "Backspace") expect(current).toBe("window-22");
      if (key === "Enter") { expect(current).not.toBe(source); expect(pageByName("Window")!.roots).toHaveLength(2001); }
      if (key === "Tab") expect(node("window-23")?.parent).toBe("window-22");
    });
    if (key === "Backspace") {
      expect(node("window-23")).toBeUndefined();
      expect(node("window-22")?.raw).toContain("Row 23 unique content");
    }
    expect(host.querySelectorAll(".ls-block").length).toBeLessThan(150);
  } finally { dispose(); }
});
