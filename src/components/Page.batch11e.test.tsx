import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { feedNames, pageByName, resetStore, setRaw, setPageProperty, flushPage, isDirty } from "../document";
import { pageToDto } from "../document/convert";
import { type FeedPage, type Node as StoreNode, doc, setDoc } from "../document/model";
import { endEdit, startEditing } from "../editorController";
import { journalTitle } from "../journal";
import type { GraphMeta, JournalFeedPage, PageDto, PageRead } from "../types";
import { PageView, reloadJournalsFeedFromStart } from "./Page";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { clearRecent, recentPages, setRecentPages } from "../ui";
import { graphEpoch, setGraphMeta } from "../graphSession";
import { installKeybindings } from "../keybindings";
import { setToasts, toasts } from "../toasts";

beforeAll(async () => { await initParser(); });
afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  endEdit("blur");
  resetStore();
  setGraphMeta(null);
  setToasts([]);
  clearRecent();
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
function node(id: string, raw: string, pageName: string): StoreNode {
  return { id, raw, collapsed: false, parent: null, page: pageName, children: [] };
}
function journalDto(name: string, raw = name): PageDto {
  return { name, kind: "journal", title: name, pre_block: null,
    blocks: [{ id: `${name}-id`, raw, collapsed: false, children: [] }] };
}
function localDay() {
  const now = new Date();
  return now.getFullYear() * 10_000 + (now.getMonth() + 1) * 100 + now.getDate();
}
function feedResponse(pages: PageDto[]): JournalFeedPage {
  return { pages: pages as PageRead[], next_before_day: null, done: true, as_of_day: localDay() };
}

describe("batch 11e page identity and journal continuity", () => {
  it("keeps the selected text and caret after a keyboard move to another journal day", async () => {
    const uninstallKeys = installKeybindings();
    vi.stubGlobal("IntersectionObserver", class { observe() {} unobserve() {} disconnect() {} });
    const today = journalTitle(new Date());
    const older = "August 21st, 2026";
    const first = journalDto(today, "Visible today");
    const second = journalDto(older, "Move me");
    vi.spyOn(backend(), "journalFeedPage").mockResolvedValue(feedResponse([first, second]));
    vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["today-r2", "older-r2"] });
    const mounted = mount(() => <PageView />);
    try {
      const id = second.blocks[0].id;
      await vi.waitFor(() => expect(mounted.root.querySelector(`[data-block-id="${id}"]`)).not.toBeNull());
      startEditing(id, 2);
      await vi.waitFor(() => expect(mounted.root.querySelector<HTMLTextAreaElement>(`[data-block-id="${id}"] textarea`)).not.toBeNull());
      const editor = mounted.root.querySelector<HTMLTextAreaElement>(`[data-block-id="${id}"] textarea`)!;
      editor.focus();
      editor.setSelectionRange(2, 5, "forward");
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", code: "ArrowUp", altKey: true, shiftKey: true, bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(doc.byId[id].page).toBe(today));
      await vi.waitFor(() => {
        const moved = mounted.root.querySelector<HTMLTextAreaElement>(`[data-block-id="${id}"] textarea`)!;
        expect(moved.value).toBe("Move me");
        expect(document.activeElement).toBe(moved);
        expect([moved.selectionStart, moved.selectionEnd, moved.selectionDirection]).toEqual([2, 5, "forward"]);
      });
    } finally { mounted.dispose(); uninstallKeys(); vi.unstubAllGlobals(); }
  });

  it("keeps the old feed and surfaces a template read failure before retrying", async () => {
    setGraphMeta({ root: "/tmp/template-read-failure", default_journal_template: "Daily" } as GraphMeta);
    setDoc({ byId: { old: node("old", "Old feed", "Yesterday") }, pages: [page("Yesterday", "journal", ["old"])], feed: ["Yesterday"] });
    const failure = new Error("template page unreadable");
    const dayRead = vi.spyOn(backend(), "getPage").mockRejectedValueOnce(failure).mockResolvedValue(null);
    vi.spyOn(backend(), "listTemplates").mockResolvedValue([]);
    const feedRead = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue(feedResponse([journalDto("Today", "New feed")]));
    const owner = { graphEpoch: graphEpoch(), isLive: () => true };
    await reloadJournalsFeedFromStart(owner);
    expect(feedNames()).toContain("Yesterday");
    expect(feedRead).not.toHaveBeenCalled();
    expect(toasts().some((toast) => toast.kind === "error" && toast.message.includes("journal feed"))).toBe(true);
    await reloadJournalsFeedFromStart(owner);
    expect(dayRead).toHaveBeenCalledTimes(2);
    expect(feedRead).toHaveBeenCalledTimes(1);
    expect(feedNames()).toContain("Today");
  });

  it("rekeys its own title property save before the old route can be revisited", async () => {
    const id = "pages/Physical.md";
    setDoc({ byId: { body: node("body", "Body", "Physical") },
      pages: [{ ...page("Physical", "page", ["body"], "title:: Physical"), id }], feed: ["Physical"], loaded: true });
    setRecentPages([{ name: "Physical", kind: "page", path: id }]);
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Physical", pageKind: "page", path: id });
    const save = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["effective-rev"] });
    vi.spyOn(backend(), "getPageByPath").mockImplementation(async () => ({ name: save.mock.calls.length ? "Effective" : "Physical",
      kind: "page", title: save.mock.calls.length ? "Effective" : "Physical", id,
      rev: save.mock.calls.length ? "effective-rev" : "physical-rev",
      pre_block: save.mock.calls.length ? "title:: Effective" : "title:: Physical",
      blocks: [{ id: "body", raw: "Body", children: [], collapsed: false }] }));
    const mounted = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(mounted.root.textContent).toContain("Body"));
      setPageProperty("Physical", "title", "Effective");
      expect(pageToDto("Physical")?.pre_block).toContain("title:: Effective");
      expect(await flushPage("Physical")).toBe(true);
      expect(save).toHaveBeenCalled();
      expect(save.mock.calls[0][0][0].page.pre_block).toContain("title:: Effective");
      expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Effective", path: id });
      expect(recentPages()).toContainEqual({ name: "Effective", kind: "page", path: id });
      expect(pageByName("Effective")?.id).toBe(id);
      expect(pageByName("Physical")).toBeUndefined();
      expect(doc.byId.body?.page).toBe("Effective");
      setRaw("body", "Saved after title change", { timetracking: false });
      expect(isDirty("Effective")).toBe(true);
      expect(await flushPage("Effective")).toBe(true);
      expect(save).toHaveBeenCalledTimes(2);
      expect(save.mock.calls.at(-1)?.[0][0]).toMatchObject({ id, baseRev: "effective-rev" });
    } finally { mounted.dispose(); clearRecent(); }
  });

  it("rekeys to the physical name when its own title property is removed", async () => {
    const id = "pages/Physical.md";
    setDoc({ byId: { body: node("body", "Body", "Effective") },
      pages: [{ ...page("Effective", "page", ["body"], "title:: Effective"), id }], feed: ["Effective"], loaded: true });
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Effective", pageKind: "page", path: id });
    const save = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["removed-rev"] });
    vi.spyOn(backend(), "getPageByPath").mockImplementation(async () => ({ name: save.mock.calls.length ? "Physical" : "Effective",
      kind: "page", title: save.mock.calls.length ? "Physical" : "Effective", id,
      pre_block: save.mock.calls.length ? null : "title:: Effective",
      blocks: [{ id: "body", raw: "Body", children: [], collapsed: false }] }));
    const mounted = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(mounted.root.textContent).toContain("Body"));
      setPageProperty("Effective", "title", null);
      expect(await flushPage("Effective")).toBe(true);
      expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Physical", path: id });
      expect(pageByName("Physical")?.id).toBe(id);
      expect(pageByName("Effective")).toBeUndefined();
    } finally { mounted.dispose(); }
  });

  it("rekeys a live pinned title identity by its unchanged physical path", async () => {
    const id = "pages/Physical.md";
    setDoc({ byId: { body: node("body", "Keep edit", "Physical") },
      pages: [{ ...page("Physical", "page", ["body"]), id }], feed: ["Physical"], loaded: true });
    const dto: PageRead = { name: "Effective", title: "Effective", kind: "page", id,
      pre_block: "title:: Effective", blocks: [{ id: "body", raw: "Keep edit", children: [], collapsed: false }] };
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    setRecentPages([{ name: "Physical", kind: "page", path: id }]);
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Physical", pageKind: "page", path: id });
    const { root, dispose } = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Effective", path: id }));
      expect(pageByName("Effective")?.id).toBe(id);
      expect(pageByName("Physical")).toBeUndefined();
      expect(recentPages()).toContainEqual({ name: "Effective", kind: "page", path: id });
      expect(root.textContent).toContain("Keep edit");
    } finally { dispose(); clearRecent(); }
  });

  it("uses the loaded file path when revisiting a pathless stale title tab", async () => {
    const id = "pages/Physical.md";
    setDoc({ byId: { body: node("body", "Existing body", "Physical") },
      pages: [{ ...page("Physical", "page", ["body"]), id }], feed: [], loaded: true });
    const byPath = vi.spyOn(backend(), "getPageByPath").mockResolvedValue({ name: "Effective", kind: "page",
      title: "Effective", id, pre_block: "title:: Effective", rev: "effective-rev",
      blocks: [{ id: "body", raw: "Existing body", children: [], collapsed: false }] });
    const byName = vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Physical", pageKind: "page" });
    const mounted = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Effective", path: id }));
      expect(byPath).toHaveBeenCalledWith(id);
      expect(byName).not.toHaveBeenCalledWith("Physical", "page");
      expect(pageByName("Physical")).toBeUndefined();
      expect(mounted.root.textContent).toContain("Existing body");
    } finally { mounted.dispose(); }
  });
});


it("reports a failed journal feed append while retaining the loaded feed", async () => {
  let intersect!: IntersectionObserverCallback;
  vi.stubGlobal("IntersectionObserver", class {
    constructor(callback: IntersectionObserverCallback) { intersect = callback; }
    observe() {} unobserve() {} disconnect() {}
  });
  const today = journalTitle(new Date());
  const api = vi.spyOn(backend(), "journalFeedPage")
    .mockResolvedValueOnce({ ...feedResponse([journalDto(today, "Keep this journal")]), next_before_day: localDay(), done: false })
    .mockRejectedValueOnce(new Error("io:PermissionDenied"));
  const mounted = mount(() => <PageView />);
  await vi.waitFor(() => expect(mounted.root.textContent).toContain("Keep this journal"));
  intersect([{ isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver);
  await vi.waitFor(() => expect(api).toHaveBeenCalledTimes(2));
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(mounted.root.textContent).toContain("Keep this journal");
  expect(toasts().some((t) => t.kind === "error" && t.message.includes("journals"))).toBe(true);
  mounted.dispose();
});
