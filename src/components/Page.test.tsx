import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { Show, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { pageByName, readPageProperty, resetStore, setRaw, extendFeedForScroll, flushPage, isDirty, undo, moveBlockFeed } from "../document";
import { setBlockMoving } from "../document/edits/moves";
import { pageToDto } from "../document/convert";
import { type FeedPage, type Node as StoreNode } from "../document/model";
import { doc, setDoc } from "../document/model";
import { loadSingle } from "../document/workingSet";
import { editingId, editingOwner, activeSurface, endEdit, startEditing } from "../editorController";
import { journalTitle } from "../journal";
import type { JournalFeedPage, PageDto, PageRead, RefGroup } from "../types";
import { TagPageTable, TagTableToggle } from "./Page";
import { PageView, reloadJournalsFeedFromStart, withToday } from "./Page";
import { focusBlock, mainPaneRouter, resetTabsToJournals, tabRoute } from "../router";
import { clearConflict } from "../document/save/engine";
import { markConflict } from "../document/save/engine";
import { clearRecent, closeContextMenu, contextMenu, recentPages, rightSidebar, setRecentPages, setRightSidebar } from "../ui";
import { bumpGraphEpoch, graphEpoch, setGraphMeta } from "../graphSession";
import { setToasts, toasts } from "../toasts";
import type { GraphMeta } from "../types";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  // The feed owns a local-midnight timeout. Clear any timer a failed/early-
  // disposed render left behind before handing control back to the next render
  // test; otherwise `tick()` can inherit fake timers and never resolve.
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.restoreAllMocks();
  endEdit("blur");
  closeContextMenu();
  resetStore();
  setGraphMeta(null);
  document.body.innerHTML = "";
  resetTabsToJournals();
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function page(name: string, kind: "page" | "journal", roots: string[], preBlock: string | null = null): FeedPage {
  return { name, kind, title: name, preBlock, roots, format: "md", readOnly: false, guide: false };
}

function node(id: string, raw: string, pageName: string, parent: string | null = null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: pageName, children };
}

function tick(): Promise<void> {
  // Keep general render settling independent of timer virtualization. Focused
  // fake-clock cases drive their own timers explicitly below.
  return new Promise((resolve) => queueMicrotask(resolve));
}

async function flushMicrotasks() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

function localDay() {
  const now = new Date();
  return now.getFullYear() * 10_000 + (now.getMonth() + 1) * 100 + now.getDate();
}

function journalDto(name: string, raw = name): PageDto {
  return {
    name, kind: "journal", title: name, pre_block: null,
    blocks: [{ id: `${name}-id`, raw, collapsed: false, children: [] }],
  };
}

/** A backend read without a file id. These fixtures predate `PageRead.id`
 *  (B15b); a page that has no id is saved via `resolvePage`, and an id-less
 *  read matches an id-less loaded page exactly as a path-less one did. */
function unpinned(dto: PageDto): PageRead {
  return dto as PageRead;
}

function feedResponse(pages: PageDto[], patch: Partial<JournalFeedPage> = {}): JournalFeedPage {
  return { pages: pages.map(unpinned), next_before_day: null, done: true, as_of_day: localDay(), ...patch };
}

describe("Journals feed generation lifecycle", () => {
  it.each([false, true])("adds a new day around an active editor (dirty: %s)", async (dirty) => {
    vi.stubGlobal("IntersectionObserver", class {
      observe() {}
      unobserve() {}
      disconnect() {}
    });
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2030, 6, 15, 22));
    const previous = journalTitle(new Date());
    const old = journalDto(previous, "Existing notes stay here");
    vi.spyOn(backend(), "journalFeedPage").mockImplementation(async () => feedResponse([old]));
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      await vi.waitFor(() => expect(mounted.root.textContent).toContain("Existing notes stay here"));
      startEditing(old.blocks[0].id, 5);
      await flushMicrotasks();
      const editor = mounted.root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
      expect(editor).not.toBeNull();
      if (dirty) {
        editor.value = "Existing notes with unsaved overnight text";
        editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "text" }));
      }
      editor.focus();
      editor.setSelectionRange(3, 12, "backward");
      const text = editor.value;
      const owner = editingOwner();
      const surface = activeSurface();
      const oldPage = pageByName(previous);
      const oldNode = doc.byId[old.blocks[0].id];
      vi.setSystemTime(new Date(2030, 6, 16, 8));
      const today = journalTitle(new Date());
      await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
      await flushMicrotasks();
      expect(doc.feed[0]).toBe(today);
      expect(doc.feed).toContain(previous);
      expect(pageByName(previous)).toBe(oldPage);
      expect(doc.byId[old.blocks[0].id]).toBe(oldNode);
      expect(mounted.root.querySelector("textarea.block-editor")).toBe(editor);
      expect(editor.value).toBe(text);
      expect([editor.selectionStart, editor.selectionEnd, editor.selectionDirection]).toEqual([3, 12, "backward"]);
      expect(document.activeElement).toBe(editor);
      expect(editingId()).toBe(old.blocks[0].id);
      expect(editingOwner()).toBe(owner);
      expect(activeSurface()).toBe(surface);
      expect(isDirty(previous)).toBe(dirty);
    } finally {
      mounted.dispose();
      vi.unstubAllGlobals();
    }
  });

  it("retains an editor acquired while a new-day feed read is in flight", async () => {
    vi.stubGlobal("IntersectionObserver", class {
      observe() {}
      unobserve() {}
      disconnect() {}
    });
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2030, 6, 15, 22));
    const yesterday = journalTitle(new Date());
    const old = journalDto(yesterday, "Keep this node");
    let finishRead!: (value: JournalFeedPage) => void;
    vi.spyOn(backend(), "journalFeedPage")
      .mockResolvedValueOnce(feedResponse([old]))
      .mockImplementationOnce(() => new Promise((resolve) => { finishRead = resolve; }));
    const mounted = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(mounted.root.textContent).toContain("Keep this node"));
      vi.setSystemTime(new Date(2030, 6, 16, 8));
      const refresh = reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
      startEditing(old.blocks[0].id, 4);
      await flushMicrotasks();
      const editor = mounted.root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
      editor.focus();
      editor.setSelectionRange(2, 8, "backward");
      const owner = editingOwner();
      const oldNode = doc.byId[old.blocks[0].id];
      finishRead(feedResponse([old]));
      await refresh;
      await flushMicrotasks();
      expect(doc.feed[0]).toBe(journalTitle(new Date()));
      expect(doc.byId[old.blocks[0].id]).toBe(oldNode);
      expect(mounted.root.querySelector("textarea.block-editor")).toBe(editor);
      expect([editor.selectionStart, editor.selectionEnd, editor.selectionDirection]).toEqual([2, 8, "backward"]);
      expect(document.activeElement).toBe(editor);
      expect(editingOwner()).toBe(owner);
    } finally {
      mounted.dispose();
      vi.unstubAllGlobals();
    }
  });

  it("moves an edited block between loaded journal days without replacing its model node", async () => {
    vi.stubGlobal("IntersectionObserver", class {
      observe() {}
      unobserve() {}
      disconnect() {}
    });
    const today = journalTitle(new Date());
    const older = "August 21st, 2026";
    const first = journalDto(today, "Visible today");
    const second = journalDto(older, "Move me");
    vi.spyOn(backend(), "journalFeedPage").mockResolvedValue(feedResponse([first, second]));
    vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["today-r2", "older-r2"] });
    const mounted = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(mounted.root.querySelector(`[data-block-id="${second.blocks[0].id}"]`)).not.toBeNull());
      startEditing(second.blocks[0].id, 3);
      await vi.waitFor(() => expect(mounted.root.querySelector("textarea.block-editor")).not.toBeNull());
      const editor = mounted.root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
      editor.focus();
      editor.setSelectionRange(2, 2);
      const original = doc.byId[second.blocks[0].id];
      expect(await moveBlockFeed(second.blocks[0].id, -1)).toBe("crossed");
      expect(doc.byId[second.blocks[0].id]).toBe(original);
      expect(doc.byId[second.blocks[0].id].page).toBe(today);
      expect(editingId()).toBe(second.blocks[0].id);
      await vi.waitFor(() => expect(mounted.root.querySelector<HTMLTextAreaElement>(`[data-block-id="${second.blocks[0].id}"] textarea`)).not.toBeNull());
      const movedEditor = mounted.root.querySelector<HTMLTextAreaElement>(`[data-block-id="${second.blocks[0].id}"] textarea`)!;
      expect(movedEditor.value).toBe("Move me");
      expect(document.activeElement).toBe(movedEditor);
    } finally {
      mounted.dispose();
      vi.unstubAllGlobals();
    }
  });

  it("writes a configured template before reading each new local day into the feed", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2030, 6, 15, 12));
    setGraphMeta({
      root: "/tmp/journal-midnight", default_journal_template: "Daily",
      journal_page_title_format: "MMM do, yyyy", journal_file_name_format: "yyyy_MM_dd",
    } as GraphMeta);
    const order: string[] = [];
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    vi.spyOn(backend(), "listTemplates").mockResolvedValue([{
      name: "Daily", page: "Templates", kind: "page",
      blocks: [{ id: "template", raw: "Template body", collapsed: false, children: [] }],
    }]);
    vi.spyOn(backend(), "resolvePage").mockImplementation(async () => ({ kind: "absent", id: `journals/${localDay()}.md` }));
    vi.spyOn(backend(), "savePages").mockImplementation(async () => {
      order.push(`save:${localDay()}`);
      return { ok: ["revision"] };
    });
    vi.spyOn(backend(), "journalFeedPage").mockImplementation(async () => {
      order.push(`feed:${localDay()}`);
      return feedResponse([]);
    });
    const owner = { graphEpoch: graphEpoch(), isLive: () => true };
    await reloadJournalsFeedFromStart(owner);
    vi.setSystemTime(new Date(2030, 6, 16, 0, 0, 1));
    await reloadJournalsFeedFromStart(owner);
    expect(order).toEqual(["save:20300715", "feed:20300715", "save:20300716", "feed:20300716"]);
  });
  it("keeps a startup route pending when its feed read is superseded during publication", async () => {
    vi.stubGlobal("IntersectionObserver", class {
      observe() {}
      unobserve() {}
      disconnect() {}
    });
    let resolveFirst!: (value: JournalFeedPage) => void;
    let resolveSecond!: (value: JournalFeedPage) => void;
    const api = vi.spyOn(backend(), "journalFeedPage")
      .mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; }))
      .mockImplementationOnce(() => new Promise((resolve) => { resolveSecond = resolve; }));
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      // A second live surface or watcher asks while the first native read waits
      // for the store's initial snapshot publication.
      const newer = reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
      expect(api).toHaveBeenCalledTimes(2);
      // Another page load can fill the shared working-set feed first, while
      // the Journals route still needs its own feed response.
      loadSingle({ ...journalDto("ordinary page"), kind: "page" }, { endEdit: false });
      resolveFirst(feedResponse([journalDto("superseded")]));
      await flushMicrotasks();
      expect(mounted.root.textContent).not.toContain("Journal feed read failed");
      expect(mounted.root.querySelector(".page-loading"),
        "OG-09B: a superseded startup feed read must wait for the winning feed; exemplar PageView in src/components/Page.tsx"
      ).not.toBeNull();
      resolveSecond(feedResponse([journalDto("published")]));
      await newer;
      await vi.waitFor(() => expect(mounted.root.textContent).toContain("published"));
      expect(mounted.root.textContent).not.toContain("Couldn't open this page");
    } finally {
      mounted.dispose();
      vi.unstubAllGlobals();
    }
  });

  it("retries the visible Journals route when the superseding owner disappears", async () => {
    vi.stubGlobal("IntersectionObserver", class {
      observe() {}
      unobserve() {}
      disconnect() {}
    });
    let resolveFirst!: (value: JournalFeedPage) => void;
    let resolveSecond!: (value: JournalFeedPage) => void;
    const api = vi.spyOn(backend(), "journalFeedPage")
      .mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; }))
      .mockImplementationOnce(() => new Promise((resolve) => { resolveSecond = resolve; }))
      .mockResolvedValueOnce(feedResponse([journalDto("recovered")]));
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      let ownerLive = true;
      const newer = reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => ownerLive });
      ownerLive = false;
      resolveFirst(feedResponse([journalDto("superseded")]));
      await flushMicrotasks();
      resolveSecond(feedResponse([journalDto("orphaned")]));
      await newer;
      await vi.waitFor(() => expect(mounted.root.textContent).toContain("recovered"));
      expect(api,
        "OG-09B: a visible Journals route must retry when its superseding owner disappears; exemplar PageView in src/components/Page.tsx"
      ).toHaveBeenCalledTimes(3);
      expect(mounted.root.textContent).not.toContain("Couldn't open this page");
    } finally {
      mounted.dispose();
      vi.unstubAllGlobals();
    }
  });

  it("settles an initial Journals route load without reacting to its own feed replacement", async () => {
    const api = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue(feedResponse([journalDto("settled")]));
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
      await flushMicrotasks();
      expect(api).toHaveBeenCalledTimes(1);
      expect(api).toHaveBeenLastCalledWith(3, null);
    } finally {
      mounted.dispose();
    }
  });

  it("uses a real route/graph owner and discards an out-of-order older restart", async () => {
    let resolveOld!: (value: JournalFeedPage) => void;
    let resolveNew!: (value: JournalFeedPage) => void;
    let calls = 0;
    vi.spyOn(backend(), "journalFeedPage").mockImplementation(() => {
      calls += 1;
      return new Promise((resolve) => {
        if (calls === 1) resolveOld = resolve;
        else resolveNew = resolve;
      });
    });
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      const owner = { graphEpoch: graphEpoch(), isLive: () => true };
      const winning = reloadJournalsFeedFromStart(owner);
      await flushMicrotasks();
      resolveNew(feedResponse([journalDto("newer")], { next_before_day: 20300714, done: false }));
      await winning;
      expect(doc.feed).toContain("newer");
      resolveOld(feedResponse([journalDto("older")]));
      await flushMicrotasks();
      expect(doc.feed).toContain("newer");
      expect(doc.feed).not.toContain("older");

      const beforeInactive = calls;
      await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => false });
      expect(calls).toBe(beforeInactive);
    } finally {
      mounted.dispose();
    }
  });

  it("arms one local-calendar timer and cleans it up when the Journals surface disposes", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2030, 6, 15, 23, 59, 59, 990));
    const call = vi.spyOn(backend(), "journalFeedPage").mockImplementation(async () =>
      feedResponse([journalDto("timer-day")])
    );
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      expect(call).toHaveBeenCalledTimes(1);
      vi.advanceTimersByTime(34);
      await flushMicrotasks();
      expect(call).toHaveBeenCalledTimes(1);
      // 10 ms to local midnight plus the intentional 25 ms post-midnight
      // margin: this is the timer, not the old loader self-loop.
      vi.advanceTimersByTime(1);
      await flushMicrotasks();
      expect(call).toHaveBeenCalledTimes(2);
      expect(call.mock.calls).toEqual([[3, null], [3, null]]);
      expect(vi.getTimerCount()).toBeGreaterThan(0);
    } finally {
      mounted.dispose();
    }
    const afterDispose = call.mock.calls.length;
    vi.advanceTimersByTime(24 * 60 * 60 * 1000);
    expect(call.mock.calls.length).toBe(afterDispose);
  });

  it("does not let an unrelated sidebar/page editor defer the visible feed refresh", async () => {
    const today = journalTitle(new Date());
    setDoc({
      byId: {
        feed: node("feed", "feed", today),
        sidebar: node("sidebar", "sidebar", "Unrelated"),
      },
      pages: [page(today, "journal", ["feed"]), page("Unrelated", "page", ["sidebar"])],
      feed: [today], loaded: true,
    });
    startEditing("sidebar", 0);
    const call = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue(feedResponse([journalDto("fresh")]));
    await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
    expect(call).toHaveBeenCalledTimes(1);
    expect(doc.feed).toContain("fresh");
  });

  it("defers a visible dirty edit without replacing its working-set object", async () => {
    const today = journalTitle(new Date());
    setDoc({
      byId: { feed: node("feed", "unsaved", today) },
      pages: [page(today, "journal", ["feed"])], feed: [today], loaded: true,
    });
    const before = pageByName(today);
    setRaw("feed", "unsaved changed");
    const call = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue(feedResponse([journalDto("would-clobber")]));
    await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
    expect(call).not.toHaveBeenCalled();
    expect(pageByName(today)).toBe(before);
    expect(doc.feed).toEqual([today]);
  });

  it("rejects a false owner before generation acquisition so its live request still lands", async () => {
    let resolveLive!: (value: JournalFeedPage) => void;
    const api = vi.spyOn(backend(), "journalFeedPage").mockImplementation(() => new Promise((resolve) => { resolveLive = resolve; }));
    const live = reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
    await flushMicrotasks();
    await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => false });
    expect(api).toHaveBeenCalledTimes(1);
    resolveLive(feedResponse([journalDto("live-response")]));
    await live;
    expect(doc.feed).toContain("live-response");
    expect(api).toHaveBeenCalledTimes(1);
  });

  it("drops an unresolved response after its Journals surface is disposed", async () => {
    let resolve!: (value: JournalFeedPage) => void;
    vi.spyOn(backend(), "journalFeedPage").mockImplementation(() => new Promise((done) => { resolve = done; }));
    const mounted = mount(() => <PageView />);
    await flushMicrotasks();
    expect(doc.feed).toEqual([]);
    mounted.dispose();
    resolve(feedResponse([journalDto("must-not-land")]));
    await flushMicrotasks();
    expect(doc.feed).toEqual([]);
    await expect(extendFeedForScroll()).resolves.toBe(false);
  });

  it.each(["active edit", "dirty", "saving", "conflict", "moving"] as const)("defers a %s feed gate then retries on its real release", async (gate) => {
    const api = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue(feedResponse([journalDto("initial")]))
    const mounted = mount(() => <PageView />);
    await flushMicrotasks();
    api.mockClear();
    const today = journalTitle(new Date());
    setDoc({
      byId: { feed: node("feed", "original", today) },
      pages: [page(today, "journal", ["feed"])], feed: [today], loaded: true,
    });
    const oldPage = pageByName(today);
    api.mockResolvedValue(feedResponse([journalDto(`released-${gate}`)]));
    if (gate === "active edit") startEditing("feed", 0);
    if (gate === "dirty" || gate === "saving") setRaw("feed", "dirty");
    if (gate === "conflict") markConflict(today);
    if (gate === "moving") setBlockMoving(true, today);
    let saved: Promise<boolean> | null = null;
    let releaseSave: (() => void) | null = null;
    if (gate === "saving") {
      vi.spyOn(backend(), "savePages").mockImplementation(() => new Promise((resolve) => { releaseSave = () => resolve({ ok: ["rev"] }); }));
      saved = flushPage(today);
      await flushMicrotasks();
    }
    try {
      await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
      await flushMicrotasks();
      expect(api).not.toHaveBeenCalled();
      expect(pageByName(today)).toBe(oldPage);
      expect(doc.feed).toEqual([today]);
      if (gate === "active edit") endEdit("blur");
      if (gate === "dirty") {
        // Saving is the real dirty release and bumps dataRev after the backend
        // accepts it; leave the PageView retry effect to consume that event.
        vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["rev"] });
        await flushPage(today);
        await new Promise<void>((resolve) => setTimeout(resolve, 750));
      }
      if (gate === "saving") {
        releaseSave!();
        await saved!;
        await new Promise<void>((resolve) => setTimeout(resolve, 750));
      }
      if (gate === "conflict") clearConflict(today);
      if (gate === "moving") setBlockMoving(false);
      await flushMicrotasks();
      await flushMicrotasks();
      expect(api).toHaveBeenCalledTimes(1);
      expect(doc.feed).toContain(`released-${gate}`);
    } finally {
      mounted.dispose();
    }
  });

  it("reports no feed failure before the window binds its graph, then loads (og 12e P2)", async () => {
    vi.stubGlobal("IntersectionObserver", class { observe() {} unobserve() {} disconnect() {} });
    const binding = vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(0);
    const api = vi.spyOn(backend(), "journalFeedPage").mockImplementation(async () => {
      if (backend().graphBindingGeneration() === 0) throw new Error("missing-graph-binding");
      return feedResponse([journalDto("bound-day", "bound content")]);
    });
    setToasts([]);
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks(); await tick(); await flushMicrotasks();
      window.dispatchEvent(new Event("focus"));
      await flushMicrotasks(); await tick(); await flushMicrotasks();
      expect(toasts().map((t) => t.message)).not.toContain("Could not load journal feed. It will retry when the view refreshes.");
      expect(mounted.root.textContent).not.toContain("Journal feed read failed");
      binding.mockReturnValue(1);
      bumpGraphEpoch();
      await vi.waitFor(() => expect(mounted.root.textContent).toContain("bound content"));
      expect(toasts().filter((t) => t.kind === "error")).toEqual([]);
      expect(api).toHaveBeenCalled();
    } finally {
      mounted.dispose();
    }
  });

  it("retains the old feed after a current-generation error and retries it", async () => {
    const today = journalTitle(new Date());
    setDoc({
      byId: { old: node("old", "old visible content", today) },
      pages: [page(today, "journal", ["old"])], feed: [today], loaded: true,
    });
    const api = vi.spyOn(backend(), "journalFeedPage")
      .mockRejectedValueOnce(new Error("temporary backend error"))
      .mockResolvedValueOnce(feedResponse([journalDto("retried", "fresh content")]));
    const owner = { graphEpoch: graphEpoch(), isLive: () => true };
    await reloadJournalsFeedFromStart(owner);
    expect(doc.feed).toEqual([today]);
    await reloadJournalsFeedFromStart(owner);
    expect(api).toHaveBeenCalledTimes(2);
    expect(doc.feed).toContain("retried");
  });

  it("makes at most one immediate retry when native and browser local days disagree", async () => {
    const api = vi.spyOn(backend(), "journalFeedPage")
      .mockResolvedValueOnce(feedResponse([journalDto("wrong-day")], { as_of_day: 19990101 }))
      .mockResolvedValueOnce(feedResponse([journalDto("matched-day")]));
    await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
    expect(api).toHaveBeenCalledTimes(2);
    expect(doc.feed).toContain("matched-day");
  });

  it("revalidates on focus and visible rollover, but bounds a second clock mismatch", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2030, 6, 15, 12));
    const api = vi.spyOn(backend(), "journalFeedPage")
      .mockResolvedValueOnce(feedResponse([journalDto("day-one")]))
      .mockResolvedValueOnce(feedResponse([journalDto("wrong-one")], { as_of_day: 19990101 }))
      .mockResolvedValueOnce(feedResponse([journalDto("wrong-two")], { as_of_day: 19990101 }))
      .mockResolvedValueOnce(feedResponse([journalDto("day-two")], { as_of_day: 20300716 }));
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      expect(api).toHaveBeenCalledTimes(1);
      vi.setSystemTime(new Date(2030, 6, 16, 12));
      window.dispatchEvent(new Event("focus"));
      await flushMicrotasks();
      expect(api).toHaveBeenCalledTimes(3);
      await flushMicrotasks();
      expect(api).toHaveBeenCalledTimes(3);
      Object.defineProperty(document, "hidden", { configurable: true, value: false });
      document.dispatchEvent(new Event("visibilitychange"));
      await flushMicrotasks();
      expect(api).toHaveBeenCalledTimes(4);
      expect(doc.feed).toContain("day-two");
    } finally {
      mounted.dispose();
    }
  });

  it("discards a stale append after a newer restart owns the cursor", async () => {
    let resolveAppend!: (value: JournalFeedPage) => void;
    let resolveRestart!: (value: JournalFeedPage) => void;
    const api = vi.spyOn(backend(), "journalFeedPage");
    // A prior failed refresh intentionally leaves a retry pending. Establish a
    // completed generation first, as a real Journals route would, so this test
    // isolates append ownership instead of inheriting another test's retry.
    api.mockResolvedValueOnce(feedResponse([journalDto("baseline")]));
    await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
    api.mockReset()
      .mockResolvedValueOnce(feedResponse([journalDto("initial")], { next_before_day: 20300714, done: false }))
      .mockImplementationOnce(() => new Promise((resolve) => { resolveAppend = resolve; }))
      .mockImplementationOnce(() => new Promise((resolve) => { resolveRestart = resolve; }));
    const mounted = mount(() => <PageView />);
    try {
      await tick(); await tick();
      const append = extendFeedForScroll();
      await flushMicrotasks();
      const restart = reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
      await flushMicrotasks();
      resolveAppend(feedResponse([journalDto("stale-append")], { next_before_day: null, done: true }));
      await append;
      // The old append's finally must not clear the newer restart's loading
      // marker: a second intersection is blocked until that restart resolves.
      await extendFeedForScroll();
      expect(api).toHaveBeenCalledTimes(3);
      resolveRestart(feedResponse([journalDto("restart-winner")], { next_before_day: null, done: true }));
      await restart;
      expect(api).toHaveBeenCalledTimes(3);
      expect(api.mock.calls).toEqual([[3, null], [3, 20300714], [3, null]]);
      expect(doc.feed).toContain("restart-winner");
      expect(doc.feed).not.toContain("stale-append");
    } finally {
      mounted.dispose();
    }
  });

  it("keeps a returned real today DTO and only creates a placeholder when absent", () => {
    const today = journalTitle(new Date());
    const real = journalDto(today, "real today content");
    expect(withToday([real])[0]).toBe(real);
    const missing = withToday([journalDto("older")]);
    expect(missing[0].name).toBe(today);
    expect(missing[0].blocks[0].raw).toBe("");
  });
});

describe("tag-page table", () => {
  it("toggles a query-sourced table and adds new rows to today's journal", async () => {
    const todayName = journalTitle(new Date());
    setDoc({
      byId: {
        existing: node("existing", "existing", todayName),
        row: node("row", "TODO Tagged row #Tag\nowner:: Martin", "Source"),
      },
      pages: [
        page("Tag", "page", []),
        page("Source", "page", ["row"]),
        page(todayName, "journal", ["existing"]),
      ],
      feed: ["Tag"],
      loaded: true,
    });
    const groups: RefGroup[] = [
      {
        page: "Source",
        kind: "page",
        blocks: [
          {
            id: "row",
            raw: doc.byId.row.raw,
            collapsed: false,
            children: [],
            marker: "TODO",
            tags: ["Tag"],
            properties: [["owner", "Martin"]],
          },
        ],
      },
    ];
    vi.spyOn(backend(), "runQuery").mockResolvedValue(groups);
    vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["rev1"] });

    const tagPage = pageByName("Tag")!;
    const { root, dispose } = mount(() => (
      <>
        <TagTableToggle page={tagPage} />
        <Show when={readPageProperty("Tag", "tine.tag-table") === "true"}>
          <TagPageTable pageName="Tag" />
        </Show>
      </>
    ));

    await tick();
    const toggle = root.querySelector(".tag-table-toggle") as HTMLButtonElement | null;
    expect(toggle).not.toBeNull();
    toggle!.click();
    expect(readPageProperty("Tag", "tine.tag-table")).toBe("true");

    await tick();
    expect(root.textContent).toContain("Tagged row");
    expect(root.textContent).toContain("Martin");

    (root.querySelector(".sheet-add-row-ghost") as HTMLButtonElement).click();
    await flushMicrotasks();
    await flushMicrotasks();

    const today = pageByName(todayName)!;
    const newId = today.roots[today.roots.length - 1];
    expect(doc.byId[newId].raw).toMatch(/^#Tag\s*$/);
    await vi.waitFor(() => expect(editingId()).toBe(newId));

    dispose();
  });
});

describe("zoomed block view", () => {
  it("resolves a durable zoom route to the current transient live node", async () => {
    const uuid = "12345678-1234-4234-8234-123456789abc";
    const transient = "bfresh-zoom";
    const raw = `Fresh zoom target\nid:: ${uuid}`;
    const dto = {
      name: "Fresh zoom",
      kind: "page" as const,
      title: "Fresh zoom",
      pre_block: null,
      id: "pages/Fresh zoom.md",
      blocks: [{ id: uuid, raw, collapsed: false, children: [] }],
    };
    setDoc({
      byId: { [transient]: node(transient, raw, dto.name) },
      pages: [{ ...page(dto.name, "page", [transient]), id: dto.id }],
      feed: [dto.name],
      loaded: true,
    });
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    mainPaneRouter.replaceActiveRoute({
      kind: "page",
      name: dto.name,
      pageKind: dto.kind,
      path: dto.id,
      block: uuid,
    });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      expect(root.querySelector(".zoomed-page")).not.toBeNull();
      expect(root.querySelector(`[data-block-id="${transient}"]`)).not.toBeNull();
      expect(root.textContent).toContain("Fresh zoom target");
    } finally {
      dispose();
    }
  });

  it("reveals a collapsed root's children without changing its stored collapse state", async () => {
    const parent = "11111111-1111-4111-8111-111111111111";
    const child = "22222222-2222-4222-8222-222222222222";
    const dto = {
      name: "Outline",
      kind: "page" as const,
      title: "Outline",
      pre_block: null,
      blocks: [{
        id: parent,
        raw: "Collapsed section\ncollapsed:: true\nid:: 11111111-1111-4111-8111-111111111111",
        collapsed: true,
        children: [{ id: child, raw: "Hidden child", collapsed: false, children: [] }],
      }],
    };
    setDoc({
      byId: {
        [parent]: { ...node(parent, dto.blocks[0].raw, dto.name, null, [child]), collapsed: true },
        [child]: node(child, "Hidden child", dto.name, parent),
      },
      pages: [page(dto.name, "page", [parent])],
      feed: [dto.name],
      loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    focusBlock(parent);

    const { root, dispose } = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(root.querySelector(`[data-block-id="${child}"]`)).not.toBeNull());
      expect(doc.byId[parent].collapsed).toBe(true);
    } finally {
      dispose();
    }
  });

  it("Enter at a collapsed zoom root creates and focuses a rendered child, not an outside sibling", async () => {
    const parent = "11111111-1111-4111-8111-111111111111";
    const oldChild = "22222222-2222-4222-8222-222222222222";
    const outside = "33333333-3333-4333-8333-333333333333";
    const dto = {
      name: "Outline",
      kind: "page" as const,
      title: "Outline",
      pre_block: null,
      id: "pages/Outline.md",
      blocks: [
        { id: parent, raw: "Root\ncollapsed:: true", collapsed: true, children: [{ id: oldChild, raw: "Old", collapsed: false, children: [] }] },
        { id: outside, raw: "Outside", collapsed: false, children: [] },
      ],
    };
    setDoc({
      byId: {
        [parent]: { ...node(parent, dto.blocks[0].raw, dto.name, null, [oldChild]), collapsed: true },
        [oldChild]: node(oldChild, "Old", dto.name, parent),
        [outside]: node(outside, "Outside", dto.name),
      },
      pages: [{ ...page(dto.name, "page", [parent, outside]), id: dto.id }], feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    focusBlock(parent);
    const { root, dispose } = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(root.querySelector(".zoomed-page")).not.toBeNull());
      startEditing(parent, 0);
      await tick();
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      textarea.selectionStart = 0;
      textarea.selectionEnd = 0;
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      await tick();
      const created = doc.byId[parent].children[0];
      expect(created).not.toBe(oldChild);
      expect(editingId()).toBe(created);
      expect(root.querySelector(`[data-block-id="${created}"] textarea`)).not.toBeNull();
      expect(doc.pages[0].roots).toEqual([parent, outside]);
      expect(doc.byId[parent].collapsed).toBe(true);
    } finally {
      dispose();
    }
  });
});

describe("trailing page block target", () => {
  it("creates one focused root, accepts immediate input, then adds another empty root on the next click", async () => {
    const dto = {
      name: "Continue",
      kind: "page" as const,
      title: "Continue",
      pre_block: null,
      blocks: [{ id: "last", raw: "Last text", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { last: node("last", "Last text", "Continue") },
      pages: [page("Continue", "page", ["last"])], feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    mainPaneRouter.openPage("Continue", "page");
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick(); await tick();
      const target = root.querySelector(".page-trailing-block-target") as HTMLButtonElement;
      target.click();
      await tick();
      expect(doc.pages[0].roots).toHaveLength(2);
      const created = doc.pages[0].roots[1];
      expect(editingId()).toBe(created);
      const textarea = root.querySelector(`[data-block-id="${created}"] textarea`) as HTMLTextAreaElement | null;
      expect(textarea).not.toBeNull();
      expect(document.activeElement).toBe(textarea);
      expect(textarea?.selectionStart).toBe(0);
      expect(textarea?.selectionEnd).toBe(0);
      textarea!.value = "typed immediately";
      textarea!.dispatchEvent(new Event("input", { bubbles: true }));
      expect(doc.byId[created].raw).toBe("typed immediately");
      textarea!.value = "";
      textarea!.dispatchEvent(new Event("input", { bubbles: true }));
      endEdit("blur");
      target.click();
      await tick();
      // GH #158: the trailing target always adds a NEW root the user can write in
      // (stacking empty last blocks is allowed) rather than silently reusing the
      // previous empty one — so a user whose last block is empty (and possibly
      // indented) can always get a fresh unindented block below it.
      expect(doc.pages[0].roots).toHaveLength(3);
      const second = doc.pages[0].roots[2];
      expect(second).not.toBe(created);
      expect(doc.byId[second].parent).toBeNull();
      expect(editingId()).toBe(second);
    } finally { dispose(); }
  });

  it("adds a new unindented root when the last visible block is an empty indented child (GH #158)", async () => {
    const dto = {
      name: "Indented tail",
      kind: "page" as const,
      title: "Indented tail",
      pre_block: null,
      blocks: [{
        id: "parent",
        raw: "Parent text",
        collapsed: false,
        children: [{ id: "kid", raw: "", collapsed: false, children: [] }],
      }],
    };
    setDoc({
      byId: {
        parent: node("parent", "Parent text", dto.name, null, ["kid"]),
        kid: node("kid", "", dto.name, "parent"),
      },
      pages: [page(dto.name, "page", ["parent"])], feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    mainPaneRouter.openPage(dto.name, "page");
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick(); await tick();
      (root.querySelector(".page-trailing-block-target") as HTMLButtonElement).click();
      await tick();
      // Must NOT reuse the indented empty child; must create a fresh root-level block.
      expect(editingId()).not.toBe("kid");
      expect(doc.pages[0].roots).toHaveLength(2);
      const created = doc.pages[0].roots[1];
      expect(doc.byId[created].parent).toBeNull();
      expect(editingId()).toBe(created);
      expect(root.querySelector(`[data-block-id="${created}"] textarea`)).not.toBeNull();
    } finally { dispose(); }
  });

  it("keeps creation as one structural Undo entry when no text edit intervenes", async () => {
    const dto = {
      name: "Undo tail", kind: "page" as const, title: "Undo tail", pre_block: null,
      blocks: [{ id: "last", raw: "Last text", collapsed: false, children: [] }],
    };
    setDoc({ byId: { last: node("last", "Last text", dto.name) }, pages: [page(dto.name, "page", ["last"])], feed: [], loaded: true });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    mainPaneRouter.openPage(dto.name, "page");
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick(); await tick();
      (root.querySelector(".page-trailing-block-target") as HTMLButtonElement).click();
      await tick();
      endEdit("blur");
      undo();
      expect(doc.pages[0].roots).toEqual(["last"]);
    } finally { dispose(); }
  });

  it("adds a zoom-root child and hides the target on read-only pages", async () => {
    // A loaded page carries its file id, as every backend read does. (An
    // id-less fixture here would acquire one from its first save and then no
    // longer match the id-less reread.)
    const dto: PageRead = {
      name: "Zoom",
      kind: "page" as const,
      title: "Zoom",
      pre_block: null,
      id: "pages/Zoom.md",
      blocks: [{ id: "zoom", raw: "Root", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { zoom: node("zoom", "Root", "Zoom") },
      pages: [{ ...page("Zoom", "page", ["zoom"]), id: dto.id }], feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    focusBlock("zoom");
    const mounted = mount(() => <PageView />);
    await vi.waitFor(() => expect(mounted.root.querySelector(".page-trailing-block-target")).not.toBeNull());
    (mounted.root.querySelector(".page-trailing-block-target") as HTMLButtonElement).click();
    await tick();
    expect(doc.byId.zoom.children).toHaveLength(1);
    expect(doc.byId[doc.byId.zoom.children[0]].parent).toBe("zoom");
    mounted.dispose();

    setDoc("pages", 0, "readOnly", true);
    const readonly = mount(() => <PageView />);
    await tick();
    expect(readonly.root.querySelector(".page-trailing-block-target")).toBeNull();
    readonly.dispose();
  });
});

describe("page actions entry point", () => {
  it("discards the first A read after navigating A to B to A", async () => {
    let finishOld!: (page: PageRead | null) => void;
    let finishNew!: (page: PageRead | null) => void;
    const read = vi.spyOn(backend(), "getPage")
      .mockImplementationOnce(() => new Promise((resolve) => { finishOld = resolve; }))
      .mockResolvedValueOnce(null)
      .mockImplementationOnce(() => new Promise((resolve) => { finishNew = resolve; }));
    mainPaneRouter.openPage("A", "page", { inPlace: true });
    const mounted = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(1));
      mainPaneRouter.openPage("B", "page", { inPlace: true });
      await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(2));
      mainPaneRouter.openPage("A", "page", { inPlace: true });
      await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(3));
      finishOld({ name: "A", kind: "page", title: "A", pre_block: null,
        id: "pages/A.md", blocks: [{ id: "a-root", raw: "old", collapsed: false, children: [] }] });
      await flushMicrotasks();
      finishNew({ name: "A", kind: "page", title: "A", pre_block: null,
        id: "pages/A.md", blocks: [{ id: "a-root", raw: "new", collapsed: false, children: [] }] });
      await flushMicrotasks();
      expect(pageByName("A")?.roots.map((id) => doc.byId[id]?.raw)).toEqual(["new"]);
    } finally { mounted.dispose(); }
  });
  it("keeps the current tab when an older page read returns a canonical name", async () => {
    let finish!: (page: PageRead | null) => void;
    const read = vi.spyOn(backend(), "getPage")
      .mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }))
      .mockResolvedValue(null);
    mainPaneRouter.openPage("case variant", "page", { inPlace: true });
    const { dispose } = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(read).toHaveBeenCalledWith("case variant", "page"));
      mainPaneRouter.openPage("Elsewhere", "page", { inPlace: true });
      finish({ name: "Case Variant", kind: "page", title: "Case Variant", pre_block: null, blocks: [], id: "pages/Case Variant.md" });
      await flushMicrotasks();
      expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Elsewhere" });
    } finally {
      dispose();
    }
  });

  it.each(["Elsewhere", "journals"])("keeps the current tab when a rename completes after navigating to %s", async (destination) => {
    const dto: PageRead = { name: "Rename away", kind: "page", title: "Rename away", pre_block: null,
      id: "pages/Rename away.md", blocks: [{ id: "rename-away-root", raw: "Body", collapsed: false, children: [] }] };
    setDoc({ byId: { "rename-away-root": node("rename-away-root", "Body", dto.name) },
      pages: [{ ...page(dto.name, "page", ["rename-away-root"]), id: dto.id }], feed: [], loaded: true });
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
    let finish!: () => void;
    const rename = vi.spyOn(backend(), "renamePage").mockImplementationOnce(() => new Promise((resolve) => { finish = () => resolve({ outcome: "renamed", touched: [] }); }));
    mainPaneRouter.openFile(dto.id, dto.name, "page", { inPlace: true });
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick(); await tick();
      root.querySelector<HTMLElement>(".page-title")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
      await tick();
      const input = root.querySelector<HTMLInputElement>(".page-title-input")!;
      input.value = "Renamed away";
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      input.dispatchEvent(new FocusEvent("blur"));
      await vi.waitFor(() => expect(rename).toHaveBeenCalledOnce());
      if (destination === "journals") mainPaneRouter.openJournals({ inPlace: true });
      else mainPaneRouter.openPage(destination, "page", { inPlace: true });
      finish();
      await flushMicrotasks();
      if (destination === "journals") expect(mainPaneRouter.route()).toMatchObject({ kind: "journals" });
      else expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: destination });
    } finally {
      dispose();
    }
  });

  it("lands on the renamed page when the rename was reached from another page (og 12e P1)", async () => {
    const dto: PageRead = { name: "Reached rename", kind: "page", title: "Reached rename", pre_block: null,
      id: "pages/Reached rename.md", blocks: [{ id: "reached-root", raw: "Body", collapsed: false, children: [] }] };
    setDoc({ byId: { "reached-root": node("reached-root", "Body", dto.name) },
      pages: [{ ...page(dto.name, "page", ["reached-root"]), id: dto.id }], feed: [], loaded: true });
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
    const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "renamed", touched: [] });
    mainPaneRouter.openPage("Previous page", "page", { inPlace: true });
    mainPaneRouter.openFile(dto.id, dto.name, "page");
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick(); await tick();
      root.querySelector<HTMLElement>(".page-title")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
      await tick();
      const input = root.querySelector<HTMLInputElement>(".page-title-input")!;
      input.value = "Reached renamed";
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      input.dispatchEvent(new FocusEvent("blur"));
      await vi.waitFor(() => expect(rename).toHaveBeenCalledOnce());
      await flushMicrotasks();
      await vi.waitFor(() => expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Reached renamed" }));
    } finally {
      dispose();
    }
  });

  it("commits title rename once from blur or Enter and lets Escape cancel (GH #233)", async () => {
    const dto: PageRead = {
      name: "Rename me",
      kind: "page",
      title: "Rename me",
      pre_block: null,
      id: "pages/Rename me.md",
      blocks: [{ id: "rename-root", raw: "Body", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { "rename-root": node("rename-root", "Body", dto.name) },
      pages: [{ ...page(dto.name, "page", ["rename-root"]), id: dto.id }],
      feed: [],
      loaded: true,
    });
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
    let finishFirstRename!: () => void;
    const rename = vi.spyOn(backend(), "renamePage")
      .mockImplementationOnce(() => new Promise((resolve) => { finishFirstRename = () => resolve({ outcome: "renamed", touched: [] }); }))
      .mockResolvedValue({ outcome: "renamed", touched: [] });
    mainPaneRouter.openFile(dto.id, dto.name, "page", { inPlace: true });

    const { root, dispose } = mount(() => <PageView />);
    const begin = async (next: string) => {
      await tick();
      await tick();
      root.querySelector<HTMLElement>(".page-title")!.dispatchEvent(
        new MouseEvent("dblclick", { bubbles: true })
      );
      await tick();
      const input = root.querySelector<HTMLInputElement>(".page-title-input")!;
      input.value = next;
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      return input;
    };

    try {
      const blurred = await begin("Blurred name");
      blurred.dispatchEvent(new FocusEvent("blur", { bubbles: false }));
      await flushMicrotasks();
      expect(rename).toHaveBeenCalledTimes(1);
      expect(rename).toHaveBeenLastCalledWith("Rename me", "Blurred name", "rename-page", dto.id, undefined, []);
      setRaw("rename-root", "typed during rename", { timetracking: false });
      expect(doc.byId["rename-root"].raw).toBe("Body");
      finishFirstRename();
      await flushMicrotasks();

      rename.mockClear();
      const entered = await begin("Entered name");
      entered.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      entered.dispatchEvent(new FocusEvent("blur", { bubbles: false }));
      await flushMicrotasks();
      expect(rename).toHaveBeenCalledTimes(1);
      // The rename first asks whether the new name is taken (merge, GH #327).
      await vi.waitFor(() => expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Entered name" }));

      rename.mockClear();
      const escaped = await begin("Cancelled name");
      escaped.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      escaped.dispatchEvent(new FocusEvent("blur", { bubbles: false }));
      await flushMicrotasks();
      expect(rename).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("keeps a path-bearing title owner through sidebar, new-tab, and menu gestures", async () => {
    const path = "pages/client-b/Twin.md";
    const dto: PageRead = {
      name: "Twin", kind: "page", title: "Twin", pre_block: null, id: path,
      blocks: [{ id: "twin-b", raw: "Client B", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { "twin-b": node("twin-b", "Client B", "Twin") },
      pages: [{ ...page("Twin", "page", ["twin-b"]), id: path }], feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    mainPaneRouter.openFile(path, "Twin", "page", { inPlace: true });
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick(); await tick();
      const title = root.querySelector<HTMLElement>(".page-title")!;
      title.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
      expect(rightSidebar()[0]).toMatchObject({ kind: "page", name: "Twin", path });

      title.dispatchEvent(new MouseEvent("auxclick", { bubbles: true, button: 1 }));
      expect(mainPaneRouter.tabs().some((tab) => {
        const route = tabRoute(tab);
        return route.kind === "page" && route.name === "Twin" && route.path === path;
      })).toBe(true);

      title.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
      expect(contextMenu()).toMatchObject({ kind: "page", name: "Twin", pageKind: "page", path });
    } finally {
      setRightSidebar([]);
      dispose();
    }
  });

  it("exposes expanded state only on the trigger that owns the open page menu", async () => {
    const dto: PageDto = {
      name: "Duplicate actions",
      kind: "page",
      title: "Duplicate actions",
      pre_block: null,
      blocks: [{ id: "duplicate-actions-root", raw: "Body", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { "duplicate-actions-root": node("duplicate-actions-root", "Body", dto.name) },
      pages: [page(dto.name, "page", ["duplicate-actions-root"])],
      feed: [],
      loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });

    const { root, dispose } = mount(() => <><PageView /><PageView /></>);
    try {
      await tick();
      await tick();
      const triggers = [...root.querySelectorAll<HTMLButtonElement>("[data-page-actions-trigger]")];
      expect(triggers).toHaveLength(2);
      expect(triggers.map((trigger) => trigger.getAttribute("aria-expanded"))).toEqual(["false", "false"]);

      triggers[0].click();
      await tick();
      expect(triggers.map((trigger) => trigger.getAttribute("aria-expanded"))).toEqual(["true", "false"]);

      triggers[1].click();
      await tick();
      expect(triggers.map((trigger) => trigger.getAttribute("aria-expanded"))).toEqual(["false", "true"]);

      closeContextMenu();
      await tick();
      expect(triggers.map((trigger) => trigger.getAttribute("aria-expanded"))).toEqual(["false", "false"]);
      const titles = [...root.querySelectorAll<HTMLElement>(".page-title")];
      titles[0].dispatchEvent(new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: 10,
        clientY: 10,
      }));
      await tick();
      expect(triggers.map((trigger) => trigger.getAttribute("aria-expanded"))).toEqual(["false", "false"]);
    } finally {
      dispose();
    }
  });

  it("exposes a named page actions ellipsis as a real menu button", async () => {
    const dto: PageDto = {
      name: "Actions",
      kind: "page",
      title: "Actions",
      pre_block: null,
      blocks: [{ id: "actions-root", raw: "Body", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { "actions-root": node("actions-root", "Body", dto.name) },
      pages: [page(dto.name, "page", ["actions-root"])],
      feed: [],
      loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      const trigger = root.querySelector<HTMLButtonElement>("[data-page-actions-trigger]");
      expect(trigger).not.toBeNull();
      expect(trigger?.textContent?.trim()).toBe("⋯");
      expect(trigger?.getAttribute("aria-label")).toBe("Page actions");
      expect(trigger?.getAttribute("aria-haspopup")).toBe("menu");
      expect(trigger?.getAttribute("aria-expanded")).toBe("false");
      trigger!.click();
      await tick();
      expect(trigger?.getAttribute("aria-expanded")).toBe("true");
      closeContextMenu();
      await tick();
      expect(trigger?.getAttribute("aria-expanded")).toBe("false");
    } finally {
      dispose();
    }
  });

  it("keeps the trigger on read-only pages and journals but excludes bundled Guides", async () => {
    const dto: PageDto = {
      name: "Action matrix",
      kind: "page",
      title: "Action matrix",
      pre_block: null,
      blocks: [{ id: "matrix-root", raw: "Body", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { "matrix-root": node("matrix-root", "Body", dto.name) },
      pages: [{ ...page(dto.name, "page", ["matrix-root"]), readOnly: true }],
      feed: [],
      loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      expect(root.querySelector("[data-page-actions-trigger]")).not.toBeNull();

      setDoc("pages", 0, "kind", "journal");
      mainPaneRouter.openPage(dto.name, "journal", { inPlace: true });
      await tick();
      expect(root.querySelector("[data-page-actions-trigger]")).not.toBeNull();

      setDoc("pages", 0, "guide", true);
      await tick();
      expect(root.querySelector("[data-page-actions-trigger]")).toBeNull();
    } finally {
      dispose();
    }
  });
});

describe("page route loading", () => {
  it("rekeys a pinned page route and Recent entry to the disk spelling", async () => {
    const dto: PageRead = {
      name: "contents", title: "contents", kind: "page", id: "pages/contents.md",
      pre_block: null,
      blocks: [{ id: "contents-root", raw: "Table of contents", children: [], collapsed: false }],
    };
    const read = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    setRecentPages([{ name: "Contents", kind: "page", path: "pages/Contents.md" }]);
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Contents", pageKind: "page", path: "pages/Contents.md" });
    const { root, dispose } = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "contents", path: "pages/contents.md" }));
      expect(read).toHaveBeenCalledWith("pages/Contents.md");
      expect(root.textContent).toContain("Table of contents");
      expect(recentPages().filter((r) => r.path === "pages/Contents.md")).toEqual([]);
    } finally {
      dispose();
      clearRecent();
    }
  });

  it("fails closed when a shared zoom UUID is loaded from a different exact owner", async () => {
    const sharedId = "77777777-7777-4777-8777-777777777777";
    const sharedRaw = "Same copied UUID and content";
    const pathA = "pages/client-a/Twin.md";
    const pathB = "pages/client-b/Twin.md";
    const dto: PageRead = {
      name: "Twin",
      kind: "page",
      title: "Twin",
      id: pathB,
      pre_block: null,
      blocks: [{ id: sharedId, raw: sharedRaw, collapsed: false, children: [] }],
    };
    setDoc({
      byId: { [sharedId]: node(sharedId, sharedRaw, dto.name) },
      pages: [{ ...page(dto.name, "page", [sharedId]), id: pathB }],
      feed: [],
      loaded: true,
    });
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    mainPaneRouter.openFile(pathB, dto.name, dto.kind, { inPlace: true });
    focusBlock(sharedId);

    const { root, dispose } = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(root.querySelector(".zoomed-page")).not.toBeNull());
      expect(root.textContent).toContain(sharedRaw);

      // The name-keyed working-set slot is replaced by A. Its copied UUID/raw
      // must not satisfy a zoom route that still claims exact owner B.
      setDoc({
        byId: { [sharedId]: node(sharedId, sharedRaw, dto.name) },
        pages: [{ ...page(dto.name, "page", [sharedId]), id: pathA }],
        feed: [],
        loaded: true,
      });
      await tick();

      expect(root.querySelector(".zoomed-page")).toBeNull();
      expect(root.querySelector(".zoom-breadcrumb")).toBeNull();
      expect(root.querySelector(".block-content")).toBeNull();
      expect(root.textContent).not.toContain(sharedRaw);
    } finally {
      dispose();
    }
  });

  it("adopts the existing page's canonical case for a mixed-case page route", async () => {
    clearRecent();
    const dto: PageDto = {
      name: "page1",
      kind: "page",
      title: "page1",
      pre_block: null,
      blocks: [{ id: "canonical-page", raw: "canonical page content", collapsed: false, children: [] }],
    };
    const api = vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    mainPaneRouter.openPage("Page1", "page", { inPlace: true });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      await flushMicrotasks();
      expect(api).toHaveBeenNthCalledWith(1, "Page1", "page");
      expect(mainPaneRouter.route()).toEqual({ kind: "page", name: "page1", pageKind: "page" });
      expect(recentPages()[0]).toMatchObject({ name: "page1", kind: "page" });
      expect(root.textContent).toContain("canonical page content");
      expect(root.querySelector(".page-trailing-block-target")).not.toBeNull();
    } finally {
      dispose();
      clearRecent();
    }
  });

  it("ignores an obsolete load failure after a newer route has loaded", async () => {
    const fastId = "11111111-1111-4111-8111-111111111111";
    const fast = {
      name: "Fast page",
      kind: "page" as const,
      title: "Fast page",
      pre_block: null,
      blocks: [{ id: fastId, raw: "new route content", collapsed: false, children: [] }],
    };
    vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
      pages: [], next_before_day: null, done: true,
      as_of_day: new Date().getFullYear() * 10000 + (new Date().getMonth() + 1) * 100 + new Date().getDate(),
    });
    let rejectSlow!: (reason: Error) => void;
    let resolveFast!: (value: typeof fast) => void;
    vi.spyOn(backend(), "getPage").mockImplementation((name) => {
      if (name === "Slow page") {
        return new Promise((_, reject) => { rejectSlow = reject; });
      }
      return new Promise((resolve) => { resolveFast = (value) => resolve(unpinned(value)); });
    });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      mainPaneRouter.openPage("Slow page", "page", { inPlace: true });
      await tick();
      mainPaneRouter.openPage(fast.name, "page", { inPlace: true });
      await tick();
      resolveFast(fast);
      await tick();
      await tick();
      expect(root.textContent).toContain("new route content");

      rejectSlow(new Error("obsolete slow-page failure"));
      await tick();
      await tick();
      expect(root.textContent).toContain("new route content");
      expect(root.textContent).not.toContain("obsolete slow-page failure");
    } finally {
      dispose();
    }
  });

  it("does not select a collapsed final root's hidden empty descendant", async () => {
    const dto = {
      name: "Collapsed tail",
      kind: "page" as const,
      title: "Collapsed tail",
      pre_block: null,
      blocks: [{
        id: "lead",
        raw: "Lead",
        collapsed: false,
        children: [],
      }, {
        id: "parent",
        raw: "collapsed:: true\nid:: parent",
        collapsed: true,
        children: [{ id: "hidden", raw: "", collapsed: false, children: [] }],
      }],
    };
    setDoc({
      byId: {
        lead: node("lead", "Lead", dto.name),
        parent: { ...node("parent", dto.blocks[1].raw, dto.name, null, ["hidden"]), collapsed: true },
        hidden: node("hidden", "", dto.name, "parent"),
      },
      pages: [page(dto.name, "page", ["lead", "parent"])], feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    mainPaneRouter.openPage(dto.name, "page");
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick(); await tick();
      (root.querySelector(".page-trailing-block-target") as HTMLButtonElement).click();
      await tick();
      expect(editingId()).not.toBe("hidden");
      expect(doc.pages[0].roots).toHaveLength(3);
      const created = doc.pages[0].roots[2];
      expect(root.querySelector(`[data-block-id="${created}"] textarea`)).not.toBeNull();
    } finally {
      dispose();
    }
  });

  it("does not select storage children of a blank-looking opaque Sheet tail", async () => {
    const dto = {
      name: "Opaque tail",
      kind: "page" as const,
      title: "Opaque tail",
      pre_block: null,
      blocks: [{
        id: "lead",
        raw: "Lead",
        collapsed: false,
        children: [],
      }, {
        id: "grid",
        raw: "tine.view:: grid",
        collapsed: false,
        children: [{ id: "storage", raw: "", collapsed: false, children: [] }],
      }],
    };
    setDoc({
      byId: {
        lead: node("lead", "Lead", dto.name),
        grid: node("grid", dto.blocks[1].raw, dto.name, null, ["storage"]),
        storage: node("storage", "", dto.name, "grid"),
      },
      pages: [page(dto.name, "page", ["lead", "grid"])], feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    mainPaneRouter.openPage(dto.name, "page");
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick(); await tick();
      (root.querySelector(".page-trailing-block-target") as HTMLButtonElement).click();
      await tick();
      expect(editingId()).not.toBe("storage");
      expect(doc.pages[0].roots).toHaveLength(3);
      const created = doc.pages[0].roots[2];
      expect(root.querySelector(`[data-block-id="${created}"] textarea`)).not.toBeNull();
    } finally {
      dispose();
    }
  });
});

describe("page properties", () => {
  it("keeps a first-bullet alias editor mounted while the property is being typed (GH #62)", async () => {
    const propsId = "11111111-1111-4111-8111-111111111111";
    const dto = {
      name: "Books",
      kind: "page" as const,
      title: "Books",
      pre_block: null,
      blocks: [{ id: propsId, raw: "", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { [propsId]: node(propsId, "", dto.name) },
      pages: [page(dto.name, "page", [propsId])],
      feed: [dto.name],
      loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      startEditing(propsId, 0);
      await tick();
      setDoc("byId", propsId, "raw", "alias::");
      await tick();
      expect(editingId()).toBe(propsId);
      expect(root.querySelector(`[data-block-id="${propsId}"] textarea`)).not.toBeNull();

      setDoc("byId", propsId, "raw", "alias:: book");
      await tick();
      expect(root.querySelector(`[data-block-id="${propsId}"] textarea`)).not.toBeNull();

      endEdit("blur");
      await tick();
      expect(root.querySelector(`[data-block-id="${propsId}"]`)).toBeNull();
      expect(root.querySelector(".page-aliases")?.textContent).toContain("book");
    } finally {
      dispose();
    }
  });

  it("renders a properties-only first block as page properties (GH #86)", async () => {
    const propsId = "11111111-1111-4111-8111-111111111111";
    const bodyId = "22222222-2222-4222-8222-222222222222";
    const dto = {
      name: "Books",
      kind: "page" as const,
      title: "Books",
      pre_block: null,
      blocks: [
        { id: propsId, raw: "alias:: book\ntags:: blah", collapsed: false, children: [] },
        { id: bodyId, raw: "Reading list", collapsed: false, children: [] },
      ],
    };
    setDoc({
      byId: {
        [propsId]: node(propsId, dto.blocks[0].raw, dto.name),
        [bodyId]: node(bodyId, dto.blocks[1].raw, dto.name),
      },
      pages: [page(dto.name, "page", [propsId, bodyId])],
      feed: [dto.name],
      loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      expect(root.querySelector(".page-aliases")?.textContent).toContain("book");
      expect(root.querySelector(".page-properties")?.textContent).toContain("blah");
      expect(root.querySelector(`[data-block-id="${propsId}"]`)).toBeNull();
      expect(root.querySelector(`[data-block-id="${bodyId}"]`)).not.toBeNull();
    } finally {
      dispose();
    }
  });

  it("links bare alias and tag values but leaves ordinary bare properties as text (GH #139)", async () => {
    const propsId = "11111111-1111-4111-8111-111111111111";
    const bodyId = "22222222-2222-4222-8222-222222222222";
    const raw = [
      "aliases:: Book shelf，Reading",
      "tags:: books, [[Knowledge work]]",
      "owner:: Martin",
      "reviewer:: [[Jane Doe]]",
      'status:: "Draft, Private"',
    ].join("\n");
    const dto = {
      name: "Books",
      kind: "page" as const,
      title: "Books",
      pre_block: null,
      blocks: [
        { id: propsId, raw, collapsed: false, children: [] },
        { id: bodyId, raw: "Reading list", collapsed: false, children: [] },
      ],
    };
    setDoc({
      byId: {
        [propsId]: node(propsId, raw, dto.name),
        [bodyId]: node(bodyId, dto.blocks[1].raw, dto.name),
      },
      pages: [page(dto.name, "page", [propsId, bodyId])], feed: [dto.name], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      const rows = [...root.querySelectorAll<HTMLElement>(".prop-row")];
      const row = (key: string) => rows.find((candidate) => candidate.querySelector(".prop-key")?.textContent === key)!;
      expect([...row("tags").querySelectorAll(".page-ref")].map((link) => link.textContent)).toEqual(["books", "Knowledge work"]);
      expect([...row("aliases").querySelectorAll(".page-ref")].map((link) => link.textContent)).toEqual(["Book shelf", "Reading"]);
      expect(row("owner").querySelector(".page-ref")).toBeNull();
      // Explicit custom-property refs keep the app's ordinary dimmed [[bracket]]
      // styling; only the newly inferred bare built-ins need a plain label.
      expect([...row("reviewer").querySelectorAll(".page-ref")].map((link) => link.textContent)).toEqual(["[[Jane Doe]]"]);
      expect(row("status").querySelector(".page-ref")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("keeps an editable blank body after hiding an only properties block", async () => {
    const propsId = "11111111-1111-4111-8111-111111111111";
    const dto = {
      name: "Only properties",
      kind: "page" as const,
      title: "Only properties",
      pre_block: null,
      blocks: [{ id: propsId, raw: "alias:: property-only", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { [propsId]: node(propsId, dto.blocks[0].raw, dto.name) },
      pages: [page(dto.name, "page", [propsId])], feed: [dto.name], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      const visibleRoots = pageByName(dto.name)!.roots.filter((id) => id !== propsId);
      expect(visibleRoots).toHaveLength(1);
      expect(doc.byId[visibleRoots[0]].raw).toBe("");
      expect(root.querySelector(`[data-block-id="${visibleRoots[0]}"]`)).not.toBeNull();
    } finally {
      dispose();
    }
  });
});

describe("Markdown preamble content", () => {
  it("opens canonical page-header properties in the ordinary editor without dirtying on entry", async () => {
    const bodyId = "33333333-3333-4333-8333-333333333333";
    const dto = {
      name: "Header",
      kind: "page" as const,
      title: "Header",
      pre_block: "klíč:: hodnota\n\nalias:: Book",
      blocks: [{ id: bodyId, raw: "First body", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { [bodyId]: node(bodyId, "First body", dto.name) },
      pages: [page(dto.name, "page", [bodyId], dto.pre_block)],
      feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick(); await tick();
      (root.querySelector(".page-properties .prop-row") as HTMLElement).click();
      await tick();
      const headerId = pageByName(dto.name)!.roots[0];
      const editor = root.querySelector(`[data-block-id="${headerId}"] textarea`) as HTMLTextAreaElement;
      expect(editor.value).toBe(dto.pre_block);
      expect(doc.byId[headerId].originatedFromPageHeader).toBe(true);
      expect(isDirty(dto.name)).toBe(false);
      expect(pageToDto(dto.name)?.pre_block).toBe(dto.pre_block);

      editor.value = "klíč:: změněno\n\nalias:: Book";
      editor.dispatchEvent(new Event("input", { bubbles: true }));
      expect(isDirty(dto.name)).toBe(true);
      expect(pageToDto(dto.name)?.pre_block).toBe("klíč:: změněno\n\nalias:: Book");
      expect(pageToDto(dto.name)?.blocks.map((block) => block.raw)).toEqual(["First body"]);
      endEdit("blur");
      await tick();
      expect(root.querySelector(".page-properties")?.textContent).toContain("změněno");
      expect(root.querySelector(`[data-block-id="${headerId}"] textarea`)).toBeNull();
    } finally {
      dispose();
    }
  });

  it("renders text before the first bullet and promotes it only when edited (GH #85)", async () => {
    const bodyId = "22222222-2222-4222-8222-222222222222";
    const dto = {
      name: "Imported",
      kind: "page" as const,
      title: "Imported",
      pre_block: "Intro before the outline",
      blocks: [{ id: bodyId, raw: "First marked block", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { [bodyId]: node(bodyId, dto.blocks[0].raw, dto.name) },
      pages: [page(dto.name, "page", [bodyId], dto.pre_block)],
      feed: [dto.name],
      loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(unpinned(dto));
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      const preamble = root.querySelector(".preamble-block .block-content-wrapper") as HTMLElement | null;
      expect(preamble?.textContent).toContain("Intro before the outline");
      expect(pageByName(dto.name)?.preBlock).toBe(dto.pre_block);

      preamble!.click();
      await tick();
      const promoted = pageByName(dto.name)!.roots[0];
      expect(doc.byId[promoted].raw).toBe("Intro before the outline");
      expect(pageByName(dto.name)?.preBlock).toBeNull();
      expect(editingId()).toBe(promoted);
      expect(root.querySelector(`[data-block-id="${promoted}"] textarea`)).not.toBeNull();
    } finally {
      dispose();
    }
  });
});
