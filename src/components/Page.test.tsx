import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { Show, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend, QueryNotReadyError } from "../backend";
import { addDirty, graphBinding, setBaseRev } from "../persistence";
import { managedStorageRuntime } from "../managedStorageRuntime";
import { notifyGraphRebound } from "../modeHooks";
import { initParser } from "../render/parse";
import {
  doc,
  editorActivationFor,
  pageByName,
  readPageProperty,
  resetStore,
  setRaw,
  setDoc,
  extendFeedForScroll,
  flushPage,
  holdPageMutationUi,
  isDirty,
  moveBlockFeed,
  pageToDto,
  setBlockMoving,
  undo,
  type FeedPage,
  type Node as StoreNode,
} from "../store";
import { editingId, editingOwner, activeSurface, endEdit, startEditing } from "../editorController";
import { journalTitle } from "../journal";
import type { GraphMeta, JournalFeedPage, PageDto, RefGroup } from "../types";
import { TagPageTable, TagTableToggle } from "./Page";
import { PageView, reloadJournalsFeedFromStart, withToday } from "./Page";
import { focusBlock, mainPaneRouter, resetTabsToJournals, tabRoute } from "../router";
import { bumpGraphEpoch, clearConflict, clearRecent, closeContextMenu, contextMenu, graphEpoch, markConflict, registerLiveSaveConflict, recentPages, rightSidebar, setDataRev, setGraphMeta, setRightSidebar, setToasts, toasts } from "../ui";
import { resetSharedQueryResultsForTests } from "../queryResultCache";

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
  resetSharedQueryResultsForTests();
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
  // Editable DTO installation now includes an activation IPC boundary before
  // feed publication. Drain the resulting promise chain, not just the original
  // read + resource continuation.
  for (let i = 0; i < 12; i++) await Promise.resolve();
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

function feedResponse(pages: PageDto[], patch: Partial<JournalFeedPage> = {}): JournalFeedPage {
  return { pages, next_before_day: null, done: true, as_of_day: localDay(), ...patch };
}

function graphMetaWithTemplate(template: string | null): GraphMeta {
  return {
    root: "/tmp/journal-rollover-graph",
    journals_dir: "journals",
    pages_dir: "pages",
    preferred_workflow: "now",
    shortcuts: {},
    start_of_week: 6,
    block_hidden_properties: [],
    linked_references_collapsed_threshold: 100,
    default_journal_template: template,
    favorites: [],
    journal_page_title_format: "MMM do, yyyy",
    journal_file_name_format: "yyyy_MM_dd",
    preferred_format: "md",
    macros: {},
    enable_timetracking: true,
    show_brackets: true,
    logbook_with_second_support: true,
    logbook_enabled_in_timestamped_blocks: false,
    logbook_enabled_in_all_blocks: false,
    guide_announced: true,
  };
}

describe("Journals feed generation lifecycle", () => {
  it.each([
    { dirty: false, template: false },
    { dirty: true, template: false },
    { dirty: false, template: true },
    { dirty: true, template: true },
  ])("shows the new local day without ending an active editor (unsaved: $dirty, template: $template)", async ({ dirty, template }) => {
    vi.stubGlobal("IntersectionObserver", class {
      observe() {}
      unobserve() {}
      disconnect() {}
    });
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2030, 11, 31, 22));
    setGraphMeta(graphMetaWithTemplate(template ? "Daily" : null));
    const yesterday = journalTitle(new Date());
    const oldDto = journalDto(yesterday, "Existing notes stay here");
    let materialized: PageDto | null = null;
    vi.spyOn(backend(), "getPage").mockImplementation(async (name) => name === yesterday ? oldDto : materialized);
    vi.spyOn(backend(), "listTemplates").mockResolvedValue([{
      name: "Daily", page: "Templates", kind: "page",
      blocks: [{ id: "template-note", raw: "Daily template note", collapsed: false, children: [] }],
    }]);
    const save = vi.spyOn(backend(), "savePage").mockImplementation(async (dto) => {
      materialized = dto;
      return { revision: "new-day-template" };
    });
    const api = vi.spyOn(backend(), "journalFeedPage").mockImplementation(async () =>
      feedResponse(materialized ? [materialized, oldDto] : [oldDto])
    );
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      await flushMicrotasks();
      expect(doc.feed).toContain(yesterday);
      expect(mounted.root.textContent).toContain("Existing notes stay here");
      startEditing(oldDto.blocks[0].id, 5);
      await flushMicrotasks();
      const editor = mounted.root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
      expect(editor).not.toBeNull();
      if (dirty) {
        editor.value = "Existing notes with unsaved overnight text";
        editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "text" }));
      }
      editor.focus();
      editor.setSelectionRange(3, 12, "backward");
      expect(document.activeElement).toBe(editor);
      const text = editor.value;
      const owner = editingOwner();
      const surface = activeSurface();
      const oldPage = pageByName(yesterday);
      const oldNode = doc.byId[oldDto.blocks[0].id];
      const activation = editorActivationFor(yesterday);
      vi.setSystemTime(new Date(2031, 0, 2, 8));
      const today = journalTitle(new Date());
      window.dispatchEvent(new Event("focus"));
      document.dispatchEvent(new Event("visibilitychange"));
      await flushMicrotasks();
      await flushMicrotasks();
      await flushMicrotasks();
      await flushMicrotasks();
      await vi.waitFor(() => expect(doc.feed[0]).toBe(today));
      expect(mounted.root.textContent).toContain(today);
      expect(doc.feed).toContain(yesterday);
      expect(pageByName(yesterday)).toBe(oldPage);
      expect(doc.byId[oldDto.blocks[0].id]).toBe(oldNode);
      expect(mounted.root.querySelector("textarea.block-editor")).toBe(editor);
      expect(editor.value).toBe(text);
      expect([editor.selectionStart, editor.selectionEnd, editor.selectionDirection]).toEqual([3, 12, "backward"]);
      expect(document.activeElement).toBe(editor);
      expect(editingId()).toBe(oldDto.blocks[0].id);
      expect(editingOwner()).toBe(owner);
      expect(activeSurface()).toBe(surface);
      expect(editorActivationFor(yesterday)).toBe(activation);
      expect(isDirty(yesterday)).toBe(dirty);
      expect(save).toHaveBeenCalledTimes(template ? 1 : 0);
      if (template) expect(mounted.root.textContent).toContain("Daily template note");
      window.dispatchEvent(new Event("focus"));
      document.dispatchEvent(new Event("visibilitychange"));
      await flushMicrotasks();
      expect(api).toHaveBeenCalledTimes(2);
      expect(save).toHaveBeenCalledTimes(template ? 1 : 0);
      expect(doc.feed.filter((name) => name === today)).toHaveLength(1);
    } finally {
      mounted.dispose();
      vi.unstubAllGlobals();
    }
  });

  it("preserves an editor acquired during the rollover read and retains older loaded days", async () => {
    vi.stubGlobal("IntersectionObserver", class {
      observe() {}
      unobserve() {}
      disconnect() {}
    });
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2030, 6, 15, 22));
    const yesterday = journalTitle(new Date());
    const old = journalDto(yesterday, "existing notes");
    const older = journalDto("Jul 1st, 2030", "older loaded notes");
    const api = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue(feedResponse([old, older]));
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      await flushMicrotasks();
      const oldPage = pageByName(yesterday);
      const olderPage = pageByName(older.name);
      let release!: (response: JournalFeedPage) => void;
      api.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
      vi.setSystemTime(new Date(2030, 6, 16, 8));
      window.dispatchEvent(new Event("focus"));
      await flushMicrotasks();
      expect(api).toHaveBeenCalledTimes(2);
      startEditing(old.blocks[0].id, 3);
      setRaw(old.blocks[0].id, "unsaved while feed read was in flight");
      release(feedResponse([journalDto(journalTitle(new Date()))]));
      await flushMicrotasks();
      await flushMicrotasks();
      expect(doc.feed).toEqual([journalTitle(new Date()), yesterday, older.name]);
      expect(pageByName(yesterday)).toBe(oldPage);
      expect(pageByName(older.name)).toBe(olderPage);
      expect(doc.byId[old.blocks[0].id].raw).toBe("unsaved while feed read was in flight");
      expect(editingId()).toBe(old.blocks[0].id);
    } finally {
      mounted.dispose();
      vi.unstubAllGlobals();
    }
  });

  it("does not install a rollover day whose surface disappears during activation", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2030, 6, 15, 22));
    const yesterday = journalTitle(new Date());
    vi.spyOn(backend(), "journalFeedPage").mockImplementation(async () => feedResponse([journalDto(yesterday)]));
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      await flushMicrotasks();
      let release!: () => void;
      vi.spyOn(backend(), "activateAbsentEditor").mockImplementationOnce((name) => new Promise((resolve) => {
        release = () => resolve({ activation: 987654, target: `journals/${name}.md`, prospective: true });
      }));
      vi.setSystemTime(new Date(2030, 6, 16, 8));
      const today = journalTitle(new Date());
      window.dispatchEvent(new Event("focus"));
      await flushMicrotasks();
      expect(release).toBeTypeOf("function");
      mounted.dispose();
      release();
      await flushMicrotasks();
      await flushMicrotasks();
      expect(doc.feed).toEqual([yesterday]);
      expect(pageByName(today)).toBeUndefined();
    } finally {
      mounted.dispose();
    }
  });

  it("surfaces an initial feed read failure instead of claiming the graph has no journals", async () => {
    vi.spyOn(backend(), "journalFeedPage").mockRejectedValue(
      new Error("iCloud journal read failed"),
    );
    const mounted = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(mounted.root.textContent).toContain("iCloud journal read failed"));
      expect(mounted.root.textContent).toContain("Couldn't open this page");
      expect(mounted.root.textContent).not.toContain("No journal entries found");
    } finally {
      mounted.dispose();
    }
  });

  it("saves an edited page route when activation reset abandons the Journals reload", async () => {
    vi.useFakeTimers();
    const existing: PageDto = {
      name: "Résumé 日本語",
      kind: "page",
      title: "Résumé 日本語",
      pre_block: null,
      rev: "sparse-load-rev",
      blocks: [{ id: "nested-utf", raw: "nested UTF original", collapsed: false, children: [] }],
    };
    vi.spyOn(backend(), "journalFeedPage").mockImplementation(() => new Promise(() => {}));
    vi.spyOn(backend(), "getPage").mockResolvedValue(existing);
    const save = vi.spyOn(backend(), "savePage").mockResolvedValue({ revision: "sparse-saved-rev" });
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      // This is Settings' post-activation refresh. Its Journals request is still
      // pending when the user immediately opens a regular page from inventory.
      resetStore();
      bumpGraphEpoch();
      await flushMicrotasks();
      mainPaneRouter.openPage(existing.name, existing.kind, { inPlace: true });
      await flushMicrotasks();
      await flushMicrotasks();

      expect(doc.loaded).toBe(true);
      startEditing("nested-utf", existing.blocks[0].raw.length);
      await tick();
      const editor = mounted.root.querySelector<HTMLTextAreaElement>("textarea.block-editor");
      expect(editor).not.toBeNull();
      const edited = "nested UTF original sparse v2 saved existing UTF page";
      editor!.value = edited;
      editor!.dispatchEvent(new InputEvent("input", {
        bubbles: true,
        inputType: "insertText",
        data: "page",
      }));
      editor!.blur();

      await vi.advanceTimersByTimeAsync(400);
      expect(save).toHaveBeenCalledWith(
        expect.objectContaining({
          name: existing.name,
          blocks: [expect.objectContaining({ raw: edited })],
        }),
        "sparse-load-rev",
        false,
        // Not a forced save, so it presents no conflict observation (GH #254).
        null,
        // Managed replacement authority is likewise absent on an ordinary save.
        null,
      );
    } finally {
      mounted.dispose();
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

  it("coalesces concurrent same-day restarts under one live route/graph owner", async () => {
    let resolveFeed!: (value: JournalFeedPage) => void;
    const api = vi.spyOn(backend(), "journalFeedPage").mockImplementation(() =>
      new Promise((resolve) => { resolveFeed = resolve; })
    );
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      const owner = { graphEpoch: graphEpoch(), graphBinding: graphBinding(), isLive: () => true };
      const duplicate = reloadJournalsFeedFromStart(owner);
      await flushMicrotasks();
      expect(api).toHaveBeenCalledTimes(1);
      resolveFeed(feedResponse([journalDto("single-flight")], { next_before_day: 20300714, done: false }));
      await duplicate;
      expect(doc.feed).toContain("single-flight");

      const beforeInactive = api.mock.calls.length;
      await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), graphBinding: graphBinding(), isLive: () => false });
      expect(api).toHaveBeenCalledTimes(beforeInactive);
    } finally {
      mounted.dispose();
    }
  });

  it("does not publish a feed response read before a same-root graph rebound", async () => {
    let resolveFeed!: (value: JournalFeedPage) => void;
    vi.spyOn(backend(), "journalFeedPage").mockReturnValueOnce(new Promise((resolve) => {
      resolveFeed = resolve;
    }));
    const owner = {
      graphEpoch: graphEpoch(),
      graphBinding: graphBinding(),
      isLive: () => true,
    };
    const loading = reloadJournalsFeedFromStart(owner);
    await vi.waitFor(() => expect(backend().journalFeedPage).toHaveBeenCalledWith(3, null));

    notifyGraphRebound();
    resolveFeed(feedResponse([journalDto("old-binding-feed")]));
    await loading;

    expect(doc.feed).not.toContain("old-binding-feed");
    expect(pageByName("old-binding-feed")).toBeUndefined();
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

  it("materializes the configured template once before the rollover feed refresh despite duplicate triggers", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2030, 11, 31, 23, 59, 59, 990));
    setGraphMeta(graphMetaWithTemplate("Daily"));
    const oldTitle = journalTitle(new Date());
    const events: string[] = [];
    const feed = vi.spyOn(backend(), "journalFeedPage").mockImplementation(async () => {
      events.push("feed");
      return feedResponse([journalDto(localDay() === 20301231 ? oldTitle : "Jan 1st, 2031")]);
    });
    vi.spyOn(backend(), "getPage").mockImplementation(async (name) =>
      name === oldTitle ? journalDto(oldTitle, "existing old day") : null
    );
    vi.spyOn(backend(), "listTemplates").mockResolvedValue([{
      name: "Daily",
      page: "Templates",
      kind: "page",
      blocks: [
        { id: "meetings", raw: "### Meetings", collapsed: false, children: [] },
        { id: "notes", raw: "### Notes", collapsed: false, children: [] },
        { id: "tasks", raw: "### Tasks", collapsed: false, children: [] },
      ],
    }]);
    let releaseSave!: () => void;
    const save = vi.spyOn(backend(), "savePage").mockImplementation(() =>
      new Promise((resolve) => {
        events.push("save");
        releaseSave = () => resolve({ revision: "rollover-rev" });
      })
    );
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      expect(feed).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(35);
      await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(1));
      expect(feed).toHaveBeenCalledTimes(1);

      window.dispatchEvent(new Event("focus"));
      Object.defineProperty(document, "hidden", { configurable: true, value: false });
      document.dispatchEvent(new Event("visibilitychange"));
      await flushMicrotasks();
      expect(save).toHaveBeenCalledTimes(1);
      expect(feed).toHaveBeenCalledTimes(1);

      releaseSave();
      await flushMicrotasks();
      await flushMicrotasks();
      expect(save).toHaveBeenCalledWith(
        expect.objectContaining({
          name: "Jan 1st, 2031",
          kind: "journal",
          title: "Jan 1st, 2031",
          blocks: [
            expect.objectContaining({ raw: "### Meetings" }),
            expect.objectContaining({ raw: "### Notes" }),
            expect.objectContaining({ raw: "### Tasks" }),
          ],
        }),
        null,
        false,
      );
      expect(feed).toHaveBeenCalledTimes(2);
      expect(events).toEqual(["feed", "save", "feed"]);
    } finally {
      mounted.dispose();
    }
  });

  it("keeps rollover lazy when no default journal template is configured", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2030, 0, 31, 23, 59, 59, 990));
    setGraphMeta(graphMetaWithTemplate(null));
    const feed = vi.spyOn(backend(), "journalFeedPage").mockImplementation(async () =>
      feedResponse([journalDto(localDay() === 20300131 ? "Jan 31st, 2030" : "Feb 1st, 2030")])
    );
    const getPage = vi.spyOn(backend(), "getPage");
    const listTemplates = vi.spyOn(backend(), "listTemplates");
    const save = vi.spyOn(backend(), "savePage");
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      expect(feed).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(35);
      await vi.waitFor(() => expect(feed).toHaveBeenCalledTimes(2));
      expect(getPage).not.toHaveBeenCalled();
      expect(listTemplates).not.toHaveBeenCalled();
      expect(save).not.toHaveBeenCalled();
    } finally {
      mounted.dispose();
    }
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
    await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), graphBinding: graphBinding(), isLive: () => true });
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
    await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), graphBinding: graphBinding(), isLive: () => true });
    expect(call).not.toHaveBeenCalled();
    expect(pageByName(today)).toBe(before);
    expect(doc.feed).toEqual([today]);
  });

  it("rejects a false owner before generation acquisition so its live request still lands", async () => {
    let resolveLive!: (value: JournalFeedPage) => void;
    const api = vi.spyOn(backend(), "journalFeedPage").mockImplementation(() => new Promise((resolve) => { resolveLive = resolve; }));
    const live = reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), graphBinding: graphBinding(), isLive: () => true });
    await flushMicrotasks();
    await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), graphBinding: graphBinding(), isLive: () => false });
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

  it.each(["active edit", "dirty", "saving", "conflict", "moving", "explicit mutation"] as const)("defers a %s feed gate then retries on its real release", async (gate) => {
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
    const releaseMutation = gate === "explicit mutation" ? holdPageMutationUi([today]) : null;
    let saved: Promise<boolean> | null = null;
    let releaseSave: (() => void) | null = null;
    if (gate === "saving") {
      vi.spyOn(backend(), "savePage").mockImplementation(() => new Promise((resolve) => { releaseSave = () => resolve({ revision: "rev" }); }));
      saved = flushPage(today);
      await flushMicrotasks();
    }
    try {
      await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), graphBinding: graphBinding(), isLive: () => true });
      await flushMicrotasks();
      expect(api).not.toHaveBeenCalled();
      expect(pageByName(today)).toBe(oldPage);
      expect(doc.feed).toEqual([today]);
      if (gate === "active edit") endEdit("blur");
      if (gate === "dirty") {
        // Saving is the real dirty release and bumps dataRev after the backend
        // accepts it; leave the PageView retry effect to consume that event.
        vi.spyOn(backend(), "savePage").mockResolvedValue({ revision: "rev" });
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
      releaseMutation?.();
      await flushMicrotasks();
      await flushMicrotasks();
      expect(api).toHaveBeenCalledTimes(1);
      expect(doc.feed).toContain(`released-${gate}`);
    } finally {
      mounted.dispose();
    }
  });

  it("keeps today visible when Concord acquires it while a journal refresh is in flight", async () => {
    const today = journalTitle(new Date());
    const older = "August 21st, 2026";
    setDoc({
      byId: {
        today: node("today", "visible today", today),
        older: node("older", "visible older", older),
      },
      pages: [page(today, "journal", ["today"]), page(older, "journal", ["older"])],
      feed: [today, older],
      loaded: true,
    });
    let land!: (response: JournalFeedPage) => void;
    const api = vi.spyOn(backend(), "journalFeedPage")
      .mockImplementationOnce(() => new Promise((resolve) => { land = resolve; }))
      .mockResolvedValue(feedResponse([
        journalDto(today, "fresh today"),
        journalDto(older, "fresh older"),
      ]));
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      expect(api).toHaveBeenCalledTimes(1);
      const release = holdPageMutationUi([today]);
      land(feedResponse([
        journalDto(today, "response today"),
        journalDto(older, "response older"),
      ]));
      await flushMicrotasks();
      expect(doc.feed).toEqual([today, older]);
      expect(pageByName(today)?.roots.map((id) => doc.byId[id].raw)).toEqual(["visible today"]);

      release();
      await vi.waitFor(() => expect(api).toHaveBeenCalledTimes(2));
      await vi.waitFor(() => expect(pageByName(today)?.roots.map((id) => doc.byId[id].raw)).toEqual(["fresh today"]));
      expect(doc.feed).toEqual([today, older]);
    } finally {
      mounted.dispose();
    }
  });

  it("keeps a managed cross-day move visually steady and restores the moved editor", async () => {
    vi.stubGlobal("IntersectionObserver", class {
      observe() {}
      unobserve() {}
      disconnect() {}
      takeRecords() { return []; }
      root = null;
      rootMargin = "0px";
      thresholds = [0];
    });
    const today = journalTitle(new Date());
    const older = "August 21st, 2026";
    setDoc({
      byId: {
        today: node("today", "visible today", today),
        older: node("older", "move me", older),
      },
      pages: [
        { ...page(today, "journal", ["today"]), path: "journals/today.md" },
        { ...page(older, "journal", ["older"]), path: "journals/older.md" },
      ],
      feed: [today, older],
      loaded: true,
    });
    setBaseRev(today, "today-r1");
    setBaseRev(older, "older-r1");
    managedStorageRuntime.bind(7);
    managedStorageRuntime.receiveStatus({
      state: "active",
      runtime: null,
      can_activate: false,
      can_retry: false,
      can_cancel: false,
      cancel_reason: null,
      binding_generation: 7,
      application_page_admission: {
        binding_generation: 7,
        authority: "managed_writable",
        application_save_page_blocks: 511,
        application_page_request_text_bytes: 1_048_576,
        application_page_max_depth: 128,
      },
    } as any);
    vi.spyOn(backend(), "activateEditor").mockImplementation(async (path) => ({
      activation: path.endsWith("today.md") ? 41 : 42,
      target: path,
      prospective: false,
    }));
    vi.spyOn(backend(), "journalFeedPage").mockResolvedValue(feedResponse([
      {
        name: today,
        kind: "journal",
        title: today,
        pre_block: null,
        path: "journals/today.md",
        rev: "today-r1",
        blocks: [{ id: "today", raw: "visible today", collapsed: false, children: [] }],
      },
      {
        name: older,
        kind: "journal",
        title: older,
        pre_block: null,
        path: "journals/older.md",
        rev: "older-r1",
        blocks: [{ id: "older", raw: "move me", collapsed: false, children: [] }],
      },
    ]));
    let accept!: (value: any) => void;
    const actor = new Promise<any>((resolve) => { accept = resolve; });
    const move = vi.spyOn(backend(), "moveManagedApplicationSubtrees").mockReturnValue(actor);
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      await vi.waitFor(() => expect(
        mounted.root.querySelector('[data-block-id="older"]'),
        mounted.root.innerHTML,
      ).not.toBeNull());
      startEditing("older", 3);
      await vi.waitFor(() => expect(mounted.root.querySelector('[data-block-id="older"] textarea')).not.toBeNull());
      const originalEditor = mounted.root.querySelector<HTMLTextAreaElement>('[data-block-id="older"] textarea');
      expect(originalEditor).not.toBeNull();
      expect(document.activeElement).toBe(originalEditor);

      setBlockMoving(true, older);
      const pending = moveBlockFeed("older", -1);
      await vi.waitFor(() => expect(move).toHaveBeenCalledTimes(1));
      // A durable actor hold is not a destructive UI replacement. The page may
      // reject another write for this short window, but it must not visibly dim.
      expect(mounted.root.querySelectorAll(".page-mutation-busy")).toHaveLength(0);

      accept({
        binding_generation: 7,
        application_page_admission: managedStorageRuntime.snapshot().applicationPageAdmission,
        outcome: {
          status: "committed",
          episode_id: move.mock.calls[0][1].episode_id,
          batch_id: "journal-focus",
          recovered: false,
          source: {
            page: { name: older, kind: "journal", title: older, pre_block: null, path: "journals/older.md", rev: "older-r2", blocks: [] },
            revision: "older-r2",
          },
          destination: {
            page: {
              name: today,
              kind: "journal",
              title: today,
              pre_block: null,
              path: "journals/today.md",
              rev: "today-r2",
              blocks: [
                { id: "today", raw: "visible today", collapsed: false, children: [] },
                { id: "older", raw: "move me", collapsed: false, children: [] },
              ],
            },
            revision: "today-r2",
          },
        },
      });
      await expect(pending).resolves.toBe("crossed");
      await flushMicrotasks();
      setBlockMoving(false);

      const movedEditor = mounted.root.querySelector<HTMLTextAreaElement>('[data-block-id="older"] textarea');
      expect(movedEditor).not.toBeNull();
      expect(movedEditor).not.toBe(originalEditor);
      expect(document.activeElement).toBe(movedEditor);
      expect(movedEditor!.selectionStart).toBe(3);
      expect(doc.byId.older.page).toBe(today);
    } finally {
      setBlockMoving(false);
      mounted.dispose();
      managedStorageRuntime.clear();
      managedStorageRuntime.bind(1, { binding_generation: 1, authority: "direct" });
      vi.unstubAllGlobals();
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
    const owner = { graphEpoch: graphEpoch(), graphBinding: graphBinding(), isLive: () => true };
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
    await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), graphBinding: graphBinding(), isLive: () => true });
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
    await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), graphBinding: graphBinding(), isLive: () => true });
    api.mockReset()
      .mockResolvedValueOnce(feedResponse([journalDto("initial")], { next_before_day: 20300714, done: false }))
      .mockImplementationOnce(() => new Promise((resolve) => { resolveAppend = resolve; }))
      .mockImplementationOnce(() => new Promise((resolve) => { resolveRestart = resolve; }));
    const mounted = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      const append = extendFeedForScroll();
      await flushMicrotasks();
      const restart = reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), graphBinding: graphBinding(), isLive: () => true });
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
  it("retries pending tag results automatically", async () => {
    setDoc({ byId: {}, pages: [page("Tag", "page", [])], feed: ["Tag"], loaded: true });
    const run = vi.spyOn(backend(), "runQuery").mockRejectedValueOnce(new QueryNotReadyError("pending_edits"))
      .mockResolvedValue([]);
    const { root, dispose } = mount(() => <TagPageTable pageName="Tag" />);
    try {
      await vi.waitFor(() => expect(root.querySelector('[role="status"]')?.textContent).toContain("Updating"));
      await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(2));
      await vi.waitFor(() => expect(root.querySelector('[role="status"]')).toBeNull());
    } finally { dispose(); }
  });

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
    const runQuery = vi.spyOn(backend(), "runQuery").mockResolvedValue(groups);
    vi.spyOn(backend(), "savePage").mockResolvedValue({ revision: "rev1" });

    const tagPage = pageByName("Tag")!;
    const { root, dispose } = mount(() => (
      <>
        <TagTableToggle page={tagPage} />
        <Show when={readPageProperty("Tag", "tine.tag-table") === "true"}>
          <TagPageTable pageName="Tag" />
        </Show>
      </>
    ));

    await vi.waitFor(() => expect(root.querySelector(".tag-table-toggle")).not.toBeNull());
    const toggle = root.querySelector(".tag-table-toggle") as HTMLButtonElement | null;
    expect(toggle).not.toBeNull();
    toggle!.click();
    expect(readPageProperty("Tag", "tine.tag-table")).toBe("true");

    await vi.waitFor(() => expect(root.textContent).toContain("Tagged row"));
    expect(root.textContent).toContain("Martin");
    expect(runQuery).toHaveBeenCalledTimes(1);

    setDataRev((revision) => revision + 1);
    await vi.waitFor(() => expect(runQuery).toHaveBeenCalledTimes(2));

    (root.querySelector(".sheet-add-row-ghost") as HTMLButtonElement).click();
    await flushMicrotasks();
    await flushMicrotasks();
    // A page saving for the first time acquires its editor activation over IPC
    // (GH #254 increment 3), which adds one await to that first save only. The
    // assertions below are unchanged — this just lets the async work settle.
    await flushMicrotasks();

    const today = pageByName(todayName)!;
    const newId = today.roots[today.roots.length - 1];
    expect(doc.byId[newId].raw).toMatch(/^#Tag\s*$/);
    expect(editingId()).toBe(newId);

    dispose();
  });

  // Harvest W4-P1 item 4 — MEASUREMENT, not a cut. "Three split panes issue
  // three runQuery calls" does not by itself prove amplification: three panes
  // may be asking three real questions. So both controls run. The bound is one
  // call per DISTINCT (page, dataRev) question, per invalidation.
  it("issues one tag query per distinct routed page per invalidation, not one per consumer", async () => {
    const INVALIDATIONS = 5;
    const names = ["TagA", "TagB", "TagC"];
    setDoc({
      byId: {},
      pages: names.map((name) => page(name, "page", [])),
      feed: ["TagA"],
      loaded: true,
    });
    const runQuery = vi.spyOn(backend(), "runQuery").mockResolvedValue([] as RefGroup[]);

    // (a) three consumers, ONE routed page.
    const same = mount(() => (
      <>
        <TagTableToggle page={pageByName("TagA")!} />
        <TagPageTable pageName="TagA" />
        <TagPageTable pageName="TagA" />
      </>
    ));
    await tick();
    await flushMicrotasks();
    const samePerInvalidation: number[] = [];
    // Finish the owned readiness promise chain before starting a measured
    // revision; a fixed count of microtasks can include prior-revision work.
    await new Promise(resolve => setTimeout(resolve, 0));
    for (let i = 0; i < INVALIDATIONS; i++) {
      runQuery.mockClear();
      setDataRev((revision) => revision + 1);
      await tick();
      await flushMicrotasks();
      await new Promise(resolve => setTimeout(resolve, 0));
      samePerInvalidation.push(runQuery.mock.calls.length);
    }
    same.dispose();
    resetSharedQueryResultsForTests();

    // (b) three consumers, THREE distinct routed pages.
    const distinct = mount(() => (
      <>
        <TagPageTable pageName="TagA" />
        <TagPageTable pageName="TagB" />
        <TagPageTable pageName="TagC" />
      </>
    ));
    await tick();
    await flushMicrotasks();
    const distinctPerInvalidation: number[] = [];
    const distinctPagesAsked = new Set<string>();
    for (let i = 0; i < INVALIDATIONS; i++) {
      runQuery.mockClear();
      setDataRev((revision) => revision + 1);
      await tick();
      await flushMicrotasks();
      distinctPerInvalidation.push(runQuery.mock.calls.length);
      for (const [dsl] of runQuery.mock.calls) distinctPagesAsked.add(String(dsl));
    }
    distinct.dispose();

    // eslint-disable-next-line no-console -- the measurement IS the receipt.
    console.log(
      `w4_p1_tag_query invalidations=${INVALIDATIONS} ` +
        `samePageConsumers=3 samePageCallsPerInvalidation=${JSON.stringify(samePerInvalidation)} ` +
        `distinctPageConsumers=3 distinctPageCallsPerInvalidation=${JSON.stringify(distinctPerInvalidation)} ` +
        `distinctQuestionsAsked=${distinctPagesAsked.size}`
    );

    expect(samePerInvalidation).toEqual(Array.from({ length: INVALIDATIONS }, () => 1));
    expect(distinctPerInvalidation).toEqual(Array.from({ length: INVALIDATIONS }, () => 3));
    expect(distinctPagesAsked.size).toBe(names.length);
  });

  it("keeps journal tag-table resources keyed off", async () => {
    const runQuery = vi.spyOn(backend(), "runQuery").mockResolvedValue([]);
    const { dispose } = mount(() => (
      <TagTableToggle page={page("2030-07-15", "journal", [])} />
    ));
    await tick();
    expect(runQuery).not.toHaveBeenCalled();
    dispose();
  });
});

describe("zoomed block view", () => {
  it("exposes the retained draft when its physical conflict page cannot be opened (GH #541)", async () => {
    setGraphMeta({ root: "/graph", preferred_format: "md" } as never);
    const dto: PageDto = { name: "Missing notes", title: "Missing notes", kind: "page",
      path: "pages/Missing notes.md", pre_block: null,
      blocks: [{ id: "retained", raw: "Several days of retained writing", children: [], collapsed: false }] };
    await registerLiveSaveConflict(dto, "old-rev", 1);
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
    const save = vi.spyOn(backend(), "savePage");
    mainPaneRouter.openFile(dto.path!, dto.name, dto.kind, { inPlace: true });
    const { root, dispose } = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      expect(root.textContent).toContain("Several days of retained writing");
      expect([...root.querySelectorAll("button")].some((b) => b.textContent === "Copy draft")).toBe(true);
      expect(save).not.toHaveBeenCalled();
    } finally { dispose(); clearConflict(dto.name); }
  });
  it("zooms to the unique authored ID rather than a sibling's matching runtime locator (GH #373)", async () => {
    const claimed = "12345678-1234-8234-8234-123456789abc";
    const intendedRuntime = "87654321-4321-8321-8321-cba987654321";
    const pageName = "Preserved zoom identity";
    const path = "pages/Preserved zoom identity.md";
    setDoc({
      byId: {
        [claimed]: node(claimed, "Wrong structural sibling", pageName),
        [intendedRuntime]: node(
          intendedRuntime,
          `Intended preserved block\nid:: ${claimed}`,
          pageName,
        ),
      },
      pages: [{ ...page(pageName, "page", [claimed, intendedRuntime]), path }],
      feed: [],
      loaded: true,
    });
    const read = vi.spyOn(backend(), "getPageByPath");
    mainPaneRouter.replaceActiveRoute({
      kind: "page",
      name: pageName,
      pageKind: "page",
      path,
      block: claimed,
    });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await flushMicrotasks();

      expect(read).not.toHaveBeenCalled();
      expect(root.querySelector(".zoomed-page")).not.toBeNull();
      expect(root.querySelector(`[data-block-id="${intendedRuntime}"]`)).not.toBeNull();
      expect(root.querySelector(`[data-block-id="${claimed}"]`)).toBeNull();
      expect(root.textContent).toContain("Intended preserved block");
      expect(root.textContent).not.toContain("Wrong structural sibling");
    } finally {
      dispose();
    }
  });

  it("reuses the exact loaded owner when a bullet click stamps its durable id (GH #354)", async () => {
    const uuid = "12345678-1234-4234-8234-123456789abc";
    const path = "pages/Zoom race.md";
    const dto: PageDto = {
      name: "Zoom race",
      kind: "page",
      title: "Zoom race",
      path,
      pre_block: null,
      rev: "disk-rev",
      blocks: [{ id: uuid, raw: "Click my bullet", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { [uuid]: node(uuid, dto.blocks[0].raw, dto.name) },
      pages: [{ ...page(dto.name, "page", [uuid]), path }],
      feed: [],
      loaded: true,
    });
    setBaseRev(dto.name, dto.rev!);
    const read = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    let finishSave = () => {};
    vi.spyOn(backend(), "savePage").mockImplementation(() => new Promise((resolve) => {
      finishSave = () => resolve({ revision: "saved-rev" });
    }));
    mainPaneRouter.openFile(path, dto.name, dto.kind, { inPlace: true });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      expect(root.querySelector(".page-blocks")).not.toBeNull();
      read.mockClear();
      setToasts([]);

      root.querySelector<HTMLElement>(".bullet-container")!.click();
      await flushMicrotasks();

      expect(read).not.toHaveBeenCalled();
      expect(root.querySelector(".zoomed-page")).not.toBeNull();
      expect(root.textContent).toContain("Click my bullet");
      expect(toasts()).toEqual([]);
      finishSave();
      await flushMicrotasks();
    } finally {
      finishSave();
      dispose();
    }
  });

  it("resolves a durable zoom route to the current transient live node", async () => {
    const uuid = "12345678-1234-4234-8234-123456789abc";
    const transient = "bfresh-zoom";
    const raw = `Fresh zoom target\nid:: ${uuid}`;
    const dto = {
      name: "Fresh zoom",
      kind: "page" as const,
      title: "Fresh zoom",
      pre_block: null,
      path: "pages/Fresh zoom.md",
      blocks: [{ id: uuid, raw, collapsed: false, children: [] }],
    };
    setDoc({
      byId: { [transient]: node(transient, raw, dto.name) },
      pages: [{ ...page(dto.name, "page", [transient]), path: dto.path }],
      feed: [dto.name],
      loaded: true,
    });
    const read = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    mainPaneRouter.replaceActiveRoute({
      kind: "page",
      name: dto.name,
      pageKind: dto.kind,
      path: dto.path,
      block: uuid,
    });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      expect(read).not.toHaveBeenCalled();
      expect(root.querySelector(".zoomed-page")).not.toBeNull();
      expect(root.querySelector(`[data-block-id="${transient}"]`)).not.toBeNull();
      expect(root.textContent).toContain("Fresh zoom target");
    } finally {
      dispose();
    }
  });

  it("loads a restored durable zoom route when its owner is absent", async () => {
    const uuid = "87654321-4321-4321-8321-cba987654321";
    const dto: PageDto = {
      name: "Restored zoom",
      kind: "page",
      title: "Restored zoom",
      path: "pages/Restored zoom.md",
      pre_block: null,
      rev: "restored-rev",
      blocks: [{ id: uuid, raw: `Restored target\nid:: ${uuid}`, collapsed: false, children: [] }],
    };
    const read = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    mainPaneRouter.replaceActiveRoute({
      kind: "page",
      name: dto.name,
      pageKind: dto.kind,
      path: dto.path,
      block: uuid,
    });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      expect(read).toHaveBeenCalledOnce();
      expect(root.querySelector(".zoomed-page")).not.toBeNull();
      expect(root.textContent).toContain("Restored target");
    } finally {
      dispose();
    }
  });

  it("does not reuse a different same-name physical owner for a pinned zoom route", async () => {
    const uuid = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const name = "Duplicate zoom";
    const pathA = "pages/client-a/Duplicate zoom.md";
    const pathB = "pages/client-b/Duplicate zoom.md";
    const dtoB: PageDto = {
      name,
      kind: "page",
      title: name,
      path: pathB,
      pre_block: null,
      rev: "b-rev",
      blocks: [{ id: uuid, raw: `Owner B\nid:: ${uuid}`, collapsed: false, children: [] }],
    };
    setDoc({
      byId: { [uuid]: node(uuid, `Owner A dirty\nid:: ${uuid}`, name) },
      pages: [{ ...page(name, "page", [uuid]), path: pathA }],
      feed: [],
      loaded: true,
    });
    setBaseRev(name, "a-rev");
    setRaw(uuid, `Owner A still dirty\nid:: ${uuid}`);
    const read = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dtoB);
    setToasts([]);
    mainPaneRouter.replaceActiveRoute({
      kind: "page",
      name,
      pageKind: "page",
      path: pathB,
      block: uuid,
    });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      expect(read).toHaveBeenCalledOnce();
      expect(root.querySelector(".zoomed-page")).toBeNull();
      expect(pageByName(name)?.path).toBe(pathA);
      expect(toasts().at(-1)?.message).toContain("has unsaved changes");
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
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    focusBlock(parent);

    const { root, dispose } = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      await flushMicrotasks();
      expect(root.querySelector(`[data-block-id="${child}"]`)).not.toBeNull();
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
      pages: [page(dto.name, "page", [parent, outside])], feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    focusBlock(parent);
    const { root, dispose } = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      await tick();
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
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
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
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
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
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
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
    const dto = {
      name: "Zoom",
      kind: "page" as const,
      title: "Zoom",
      pre_block: null,
      blocks: [{ id: "zoom", raw: "Root", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { zoom: node("zoom", "Root", "Zoom") },
      pages: [page("Zoom", "page", ["zoom"])], feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    focusBlock("zoom");
    const mounted = mount(() => <PageView />);
    await tick(); await tick();
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
  it("commits title rename once from blur or Enter and lets Escape cancel (GH #233)", async () => {
    const dto: PageDto = {
      name: "Rename me",
      kind: "page",
      title: "Rename me",
      pre_block: null,
      path: "pages/Rename me.md",
      blocks: [{ id: "rename-root", raw: "Body", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { "rename-root": node("rename-root", "Body", dto.name) },
      pages: [{ ...page(dto.name, "page", ["rename-root"]), path: dto.path }],
      feed: [],
      loaded: true,
    });
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
    const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({ skippedConflictedReferrers: [] });
    mainPaneRouter.openFile(dto.path!, dto.name, "page", { inPlace: true });

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
      expect(rename).toHaveBeenLastCalledWith("Rename me", "Blurred name", dto.path);

      rename.mockClear();
      const entered = await begin("Entered name");
      entered.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      entered.dispatchEvent(new FocusEvent("blur", { bubbles: false }));
      await flushMicrotasks();
      expect(rename).toHaveBeenCalledTimes(1);

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

  it.each(["save refusal", "other page conflict", "missing conflict review", "many conflicts"])("names the actual blocker when title rename cannot flush: %s (GH #535)", async (blocker) => {
    const dto: PageDto = {
      name: "Rename me", kind: "page", title: "Rename me", pre_block: null,
      path: "pages/Rename me.md",
      blocks: [{ id: "rename-root", raw: "Unsaved body", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { "rename-root": node("rename-root", "Unsaved body", dto.name) },
      pages: [{ ...page(dto.name, "page", ["rename-root"]), path: dto.path }],
      feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
    const save = vi.spyOn(backend(), "savePage").mockRejectedValue({
      kind: "direct-save-failure", reasonCode: "identity.name_taken",
      message: "Another file owns this page name",
    });
    const rename = vi.spyOn(backend(), "renamePage");
    const warning = vi.spyOn(globalThis, "alert").mockImplementation(() => {});
    mainPaneRouter.openFile(dto.path!, dto.name, "page", { inPlace: true });
    const { root, dispose } = mount(() => <PageView />);
    try {
      await flushMicrotasks(); await flushMicrotasks();
      if (blocker === "save refusal") addDirty(dto.name);
      else if (blocker === "missing conflict review") {
        save.mockRejectedValue({ kind: "save-conflict", reasonCode: "conflict.pinned_owner", epoch: null });
        addDirty(dto.name);
      } else if (blocker === "many conflicts") {
        for (const name of ["First", "Second", "Third", "Fourth"]) markConflict(name);
      } else {
        await registerLiveSaveConflict({ ...dto, name: "Other page", path: "pages/Other page.md" }, null, 1);
        markConflict("Other page");
      }
      root.querySelector<HTMLElement>(".page-title")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
      await tick();
      const input = root.querySelector<HTMLInputElement>(".page-title-input")!;
      input.value = "Renamed";
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      for (let i = 0; i < 10; i++) await flushMicrotasks();
      expect(rename).not.toHaveBeenCalled();
      expect(doc.byId["rename-root"].raw).toBe("Unsaved body");
      if (blocker === "save refusal") {
        expect(save).toHaveBeenCalled();
        expect(isDirty(dto.name)).toBe(true);
        expect(warning).toHaveBeenCalledWith(expect.stringContaining('“Rename me”'));
        expect(warning.mock.calls[0][0]).not.toContain("conflict");
      } else if (blocker === "missing conflict review") {
        expect(save).toHaveBeenCalled();
        expect(warning).toHaveBeenCalledWith(expect.stringContaining('“Rename me”'));
        expect(warning.mock.calls[0][0]).toContain("no conflict review");
        expect(warning.mock.calls[0][0]).not.toContain("Open that page");
      } else if (blocker === "many conflicts") {
        expect(warning.mock.calls[0][0]).toContain("and 1 more");
        expect(warning.mock.calls[0][0]).not.toContain("Fourth");
      } else {
        expect(save).not.toHaveBeenCalled();
        expect(warning).toHaveBeenCalledWith(expect.stringContaining('“Other page” has an unresolved save conflict'));
      }
    } finally {
      for (const name of ["Other page", dto.name, "First", "Second", "Third", "Fourth"]) clearConflict(name);
      dispose();
    }
  });

  it("keeps a path-bearing title owner through sidebar, new-tab, and menu gestures", async () => {
    const path = "pages/client-b/Twin.md";
    const dto: PageDto = {
      name: "Twin", kind: "page", title: "Twin", pre_block: null, path,
      blocks: [{ id: "twin-b", raw: "Client B", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { "twin-b": node("twin-b", "Client B", "Twin") },
      pages: [{ ...page("Twin", "page", ["twin-b"]), path }], feed: [], loaded: true,
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
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });

    const { root, dispose } = mount(() => <><PageView /><PageView /></>);
    try {
      await vi.waitFor(() => {
        expect(root.querySelectorAll("[data-page-actions-trigger]")).toHaveLength(2);
      });
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
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
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
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });
    const { root, dispose } = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      expect(root.querySelector("[data-page-actions-trigger]")).not.toBeNull();

      setDoc("pages", 0, "kind", "journal");
      mainPaneRouter.openPage(dto.name, "journal", { inPlace: true });
      await flushMicrotasks();
      expect(root.querySelector("[data-page-actions-trigger]")).not.toBeNull();

      setDoc("pages", 0, "guide", true);
      await flushMicrotasks();
      expect(root.querySelector("[data-page-actions-trigger]")).toBeNull();
    } finally {
      dispose();
    }
  });
});

describe("page route loading", () => {
  it("keeps a visible readiness status while the requested page is still loading", async () => {
    const dto: PageDto = {
      name: "Patient page",
      kind: "page",
      title: "Patient page",
      pre_block: null,
      blocks: [{ id: "patient-page", raw: "Loaded body", collapsed: false, children: [] }],
    };
    let resolvePage!: (value: PageDto) => void;
    vi.spyOn(backend(), "getPage").mockImplementation(() => new Promise((resolve) => {
      resolvePage = resolve;
    }));
    mainPaneRouter.openPage(dto.name, dto.kind, { inPlace: true });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      const loading = root.querySelector<HTMLElement>(".page-loading");
      expect(loading?.getAttribute("role")).toBe("status");
      expect(loading?.getAttribute("aria-live")).toBe("polite");
      expect(loading?.textContent).toContain("Loading page");

      resolvePage(dto);
      await flushMicrotasks();
      expect(root.querySelector(".page-loading")).toBeNull();
      expect(root.textContent).toContain("Loaded body");
    } finally {
      dispose();
    }
  });

  it("fails closed when a shared zoom UUID is loaded from a different exact owner", async () => {
    const sharedId = "77777777-7777-4777-8777-777777777777";
    const sharedRaw = "Same copied UUID and content";
    const pathA = "pages/client-a/Twin.md";
    const pathB = "pages/client-b/Twin.md";
    const dto: PageDto = {
      name: "Twin",
      kind: "page",
      title: "Twin",
      path: pathB,
      pre_block: null,
      blocks: [{ id: sharedId, raw: sharedRaw, collapsed: false, children: [] }],
    };
    setDoc({
      byId: { [sharedId]: node(sharedId, sharedRaw, dto.name) },
      pages: [{ ...page(dto.name, "page", [sharedId]), path: pathB }],
      feed: [],
      loaded: true,
    });
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    mainPaneRouter.openFile(pathB, dto.name, dto.kind, { inPlace: true });
    focusBlock(sharedId);

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      expect(root.querySelector(".zoomed-page")).not.toBeNull();
      expect(root.textContent).toContain(sharedRaw);

      // The name-keyed working-set slot is replaced by A. Its copied UUID/raw
      // must not satisfy a zoom route that still claims exact owner B.
      setDoc({
        byId: { [sharedId]: node(sharedId, sharedRaw, dto.name) },
        pages: [{ ...page(dto.name, "page", [sharedId]), path: pathA }],
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
    const api = vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
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
      return new Promise((resolve) => { resolveFast = resolve as (value: typeof fast) => void; });
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

  it("does not arm persistence from an obsolete successful page load", async () => {
    const stale: PageDto = {
      name: "Stale page",
      kind: "page",
      title: "Stale page",
      pre_block: null,
      rev: "stale-rev",
      blocks: [{ id: "stale-block", raw: "obsolete content", collapsed: false, children: [] }],
    };
    let resolveStale!: (value: PageDto) => void;
    vi.spyOn(backend(), "getPage").mockImplementation((name) => {
      if (name === stale.name) {
        return new Promise((resolve) => { resolveStale = resolve; });
      }
      return new Promise(() => {});
    });
    mainPaneRouter.openPage(stale.name, stale.kind, { inPlace: true });

    const { dispose } = mount(() => <PageView />);
    try {
      await flushMicrotasks();
      mainPaneRouter.openPage("Current page", "page", { inPlace: true });
      await flushMicrotasks();
      resolveStale(stale);
      await flushMicrotasks();

      expect(pageByName(stale.name)).toBeUndefined();
      expect(doc.loaded).toBe(false);
    } finally {
      dispose();
    }
  });

  it("does not install a routed DTO read before a same-root graph rebound", async () => {
    const stale: PageDto = {
      name: "Rebound page",
      kind: "page",
      title: "Rebound page",
      pre_block: null,
      path: "pages/Rebound page.md",
      rev: "rebound-rev",
      blocks: [{ id: "rebound-page-block", raw: "old graph", collapsed: false, children: [] }],
    };
    let resolveStale!: (value: PageDto) => void;
    vi.spyOn(backend(), "getPage").mockReturnValueOnce(new Promise((resolve) => {
      resolveStale = resolve;
    }));
    const activate = vi.spyOn(backend(), "activateEditor");
    mainPaneRouter.openPage(stale.name, stale.kind, { inPlace: true });

    const { dispose } = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(backend().getPage).toHaveBeenCalledWith(stale.name, stale.kind));
      notifyGraphRebound();
      resolveStale(stale);
      await flushMicrotasks();

      expect(pageByName(stale.name)).toBeUndefined();
      expect(activate).not.toHaveBeenCalled();
      expect(doc.loaded).toBe(false);
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
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
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
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
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
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
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
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
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
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await flushMicrotasks();
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
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
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
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
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
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
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
