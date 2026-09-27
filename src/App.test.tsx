import type { PageDto, PageRead } from "./types";
import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { installMobileExternalLinkHandler } from "./App";
import { paneRouter, resetPaneLayoutToSingle, restorePaneLayout } from "./panes";
import { markDirty, resetStore, setBlockMoving } from "./document";
import { pageToDto } from "./document/convert";
import { type FeedPage, type Node as StoreNode } from "./document/model";
import { setDoc } from "./document/model";
import { isConflicted } from "./document";
import { pageInventoryRev } from "./graphSession";
import { applyGraphChange as handleGraphChange } from "./document";

function addAnchor(href: string): HTMLAnchorElement {
  const a = document.createElement("a");
  a.href = href;
  a.textContent = href;
  document.body.appendChild(a);
  return a;
}

function click(el: Element): MouseEvent {
  const event = new MouseEvent("click", { bubbles: true, cancelable: true });
  el.dispatchEvent(event);
  return event;
}

afterEach(() => {
  document.body.innerHTML = "";
  vi.restoreAllMocks();
  resetStore();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
});

function page(name: string, kind: "page" | "journal", roots: string[]): FeedPage {
  return { name, kind, title: name, preBlock: null, roots, format: "md", readOnly: false, guide: false };
}

function node(id: string, pageName: string): StoreNode {
  return { id, raw: "loaded elsewhere", collapsed: false, parent: null, page: pageName, children: [] };
}

describe("mobile external link delegation", () => {
  it("opens external links through the OS browser on Android", async () => {
    vi.spyOn(backend(), "appPlatform").mockResolvedValue("android");
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue();
    const uninstall = await installMobileExternalLinkHandler();
    try {
      const a = addAnchor("https://x.test/path");
      const targetClick = vi.fn();
      a.addEventListener("click", targetClick);

      const event = click(a);

      expect(event.defaultPrevented).toBe(true);
      expect(targetClick).not.toHaveBeenCalled();
      expect(openExternal).toHaveBeenCalledTimes(1);
      expect(openExternal).toHaveBeenCalledWith("https://x.test/path");
    } finally {
      uninstall();
    }
  });

  it("does not intercept external links on desktop", async () => {
    vi.spyOn(backend(), "appPlatform").mockResolvedValue("desktop");
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue();
    const uninstall = await installMobileExternalLinkHandler();
    try {
      const a = addAnchor("https://x.test/path");
      a.target = "_blank";
      const event = click(a);

      expect(event.defaultPrevented).toBe(false);
      expect(openExternal).not.toHaveBeenCalled();
    } finally {
      uninstall();
    }
  });

  it("leaves internal hash links untouched on Android", async () => {
    vi.spyOn(backend(), "appPlatform").mockResolvedValue("android");
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue();
    const uninstall = await installMobileExternalLinkHandler();
    try {
      const event = click(addAnchor("#x"));

      expect(event.defaultPrevented).toBe(false);
      expect(openExternal).not.toHaveBeenCalled();
    } finally {
      uninstall();
    }
  });
});

describe("journal watcher feed reconciliation", () => {
  it("keeps a clean page loaded when its own saved bytes echo through the watcher", async () => {
    const name = "Own save";
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
    setDoc({ byId: { same: { ...node("same", name), raw: "saved" } }, pages: [page(name, "page", ["same"])], feed: [name], loaded: true });
    const read = vi.spyOn(backend(), "getPage").mockResolvedValue({ name, kind: "page", title: name, id: "pages/Own save.md", pre_block: null, blocks: [{ id: "same", raw: "saved", collapsed: false, children: [] }] } as PageRead);

    await handleGraphChange({ name, kind: "page", created: false, removed: false });
    expect(read).toHaveBeenCalledTimes(1);
    expect(pageToDto(name)!.blocks[0].raw).toBe("saved");
  });

  it("marks a dirty page conflicted and preserves its edits after an external change", async () => {
    const name = "Dirty";
    setDoc({ byId: { local: { ...node("local", name), raw: "local edit" } }, pages: [page(name, "page", ["local"])], feed: [], loaded: true });
    markDirty(name);
    const read = vi.spyOn(backend(), "getPage");

    await handleGraphChange({ name, kind: "page", created: false, removed: false });
    expect(isConflicted(name)).toBe(true);
    expect(pageToDto(name)!.blocks[0].raw).toBe("local edit");
    expect(read).not.toHaveBeenCalled();
  });

  it("keeps a removed dirty page open as a conflict", async () => {
    const name = "Removed while dirty";
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
    setDoc({ byId: { local: node("local", name) }, pages: [page(name, "page", ["local"])], feed: [name], loaded: true });
    markDirty(name);
    await handleGraphChange({ name, kind: "page", created: false, removed: true });
    expect(isConflicted(name)).toBe(true);
    expect(paneRouter("main").route()).toMatchObject({ kind: "page", name });
  });

  it("leaves a page alone while a block move is in progress", async () => {
    const name = "Moving";
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
    setDoc({ byId: { old: node("old", name) }, pages: [page(name, "page", ["old"])], feed: [name], loaded: true });
    const read = vi.spyOn(backend(), "getPage");
    setBlockMoving(true);
    try {
      await handleGraphChange({ name, kind: "page", created: false, removed: false });
      expect(read).not.toHaveBeenCalled();
      expect(isConflicted(name)).toBe(false);
    } finally {
      setBlockMoving(false);
    }
  });

  it("reloads a loaded satellite page outside the visible route and feed", async () => {
    const name = "Satellite";
    setDoc({ byId: { old: node("old", name) }, pages: [page(name, "page", ["old"])], feed: [], loaded: true });
    vi.spyOn(backend(), "getPage").mockResolvedValue({ name, kind: "page", title: name, id: "pages/Satellite.md", pre_block: null, blocks: [{ id: "fresh", raw: "external", collapsed: false, children: [] }] } as PageRead);
    await handleGraphChange({ name, kind: "page", created: false, removed: false });
    expect(pageToDto(name)!.blocks[0].raw).toBe("external");
  });

  it("restarts Journals for an unloaded journal without fetching a page DTO", async () => {
    const name = "15th July, 2030";
    const read = vi.spyOn(backend(), "getPage");
    const now = new Date();
    const feed = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
      pages: [], next_before_day: null, done: true,
      as_of_day: now.getFullYear() * 10_000 + (now.getMonth() + 1) * 100 + now.getDate(),
    });
    await handleGraphChange({ name, kind: "journal", created: false, removed: false });
    await Promise.resolve();
    expect(read).not.toHaveBeenCalled();
    expect(feed).toHaveBeenCalledWith(3, null);
  });

  it("reloads a loaded journal in Journals and restarts its feed", async () => {
    const name = "15th July, 2030";
    setDoc({ byId: { old: node("old", name) }, pages: [page(name, "journal", ["old"])], feed: [], loaded: true });
    const read = vi.spyOn(backend(), "getPage").mockResolvedValue({ name, kind: "journal", title: name, id: "journals/2030_07_15.md", pre_block: null, blocks: [{ id: "fresh", raw: "external journal", collapsed: false, children: [] }] } as PageRead);
    const now = new Date();
    const feed = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
      pages: [], next_before_day: null, done: true,
      as_of_day: now.getFullYear() * 10_000 + (now.getMonth() + 1) * 100 + now.getDate(),
    });
    await handleGraphChange({ name, kind: "journal", created: false, removed: false });
    await Promise.resolve();
    expect(read).toHaveBeenCalledTimes(1);
    expect(pageToDto(name)!.blocks[0].raw).toBe("external journal");
    expect(feed).toHaveBeenCalledWith(3, null);
  });

  it("restarts Journals while preserving a dirty journal removed on disk", async () => {
    const name = "15th July, 2030";
    setDoc({ byId: { local: node("local", name) }, pages: [page(name, "journal", ["local"])], feed: [], loaded: true });
    markDirty(name);
    const now = new Date();
    const feed = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
      pages: [], next_before_day: null, done: true,
      as_of_day: now.getFullYear() * 10_000 + (now.getMonth() + 1) * 100 + now.getDate(),
    });
    await handleGraphChange({ name, kind: "journal", created: false, removed: true });
    await Promise.resolve();
    expect(isConflicted(name)).toBe(true);
    expect(pageToDto(name)!.blocks[0].raw).toBe("loaded elsewhere");
    expect(feed).toHaveBeenCalledWith(3, null);
  });

  it("navigates away from a removed page", async () => {
    const name = "Deleted";
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
    await handleGraphChange({ name, kind: "page", created: false, removed: true });
    expect(paneRouter("main").route().kind).toBe("journals");
  });

  it("does not mark a new graph page conflicted from an old watcher event (I-20)", async () => {
    const name = "Same name";
    setDoc({ byId: { fresh: node("fresh", name) }, pages: [page(name, "page", ["fresh"])], feed: [], loaded: true });
    markDirty(name);
    await handleGraphChange({ name, kind: "page", created: false, removed: true, binding_generation: Number.MAX_SAFE_INTEGER });
    expect(isConflicted(name), "I-20: old watcher events must not block the new graph; exemplar src/App.tsx handleGraphChange").toBe(false);
  });

  it("does not reload old watcher bytes into a same-name page after a graph switch (I-20)", async () => {
    const name = "Same name";
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
    setDoc({ byId: { old: { ...node("old", name), raw: "old graph" } }, pages: [page(name, "page", ["old"])], feed: [name], loaded: true });
    let finish!: (dto: PageRead | null) => void;
    const read = vi.spyOn(backend(), "getPage").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const changed = handleGraphChange({ name, kind: "page", created: false, removed: false });
    await vi.waitFor(() => expect(read).toHaveBeenCalled());
    resetStore();
    setDoc({ byId: { fresh: { ...node("fresh", name), raw: "new graph" } }, pages: [page(name, "page", ["fresh"])], feed: [name], loaded: true });
    finish({ name, kind: "page", title: name, id: "pages/Same name.md", pre_block: null, blocks: [{ id: "stale", raw: "stale watcher", collapsed: false, children: [] }] });
    await changed;
    expect(pageToDto(name)!.blocks[0].raw).toBe("new graph");
  });

  it("restarts a live Journals feed when the changed journal was already loaded in another pane", async () => {
    const name = "15th July, 2030";
    restorePaneLayout(
      { kind: "split", dir: "row", ratio: 0.5, children: [{ kind: "pane", paneId: "main" }, { kind: "pane", paneId: "pane-2" }] },
      new Map([
        ["main", { tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 }],
        ["pane-2", { tabs: [{ history: [{ kind: "page", name, pageKind: "journal" }], pos: 0, pinned: false }], activeIndex: 0 }],
      ]),
      "main"
    );
    setDoc({ byId: { loaded: node("loaded", name) }, pages: [page(name, "journal", ["loaded"])], feed: [], loaded: true });
    vi.spyOn(backend(), "getPage").mockResolvedValue({ name, kind: "journal", title: name, pre_block: null, blocks: [] } as PageDto as PageRead);
    const now = new Date();
    const feed = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
      pages: [], next_before_day: null, done: true,
      as_of_day: now.getFullYear() * 10_000 + (now.getMonth() + 1) * 100 + now.getDate(),
    });

    await handleGraphChange({ name, kind: "journal", created: false, removed: false });
    await Promise.resolve();
    expect(feed).toHaveBeenCalledTimes(1);
    expect(feed).toHaveBeenCalledWith(3, null);
  });
});

describe("watcher page inventory invalidation", () => {
  it("bumps the rare page-inventory revision for an external create", async () => {
    const before = pageInventoryRev();
    await handleGraphChange({ name: "Created Elsewhere", kind: "page", created: true, removed: false });
    expect(pageInventoryRev()).toBeGreaterThan(before);
  });
});
