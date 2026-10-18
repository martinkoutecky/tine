import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { PageView } from "./Page";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore, flushAll, appendFeed, pageByName } from "../document";
import { openPage, resetTabsToJournals } from "../router";
import { currentDayKey, journalTitle } from "../journal";
import { resetReferenceSectionState } from "../referenceSectionState";
import { setGraphMeta, bumpDataRev } from "../graphSession";
import { startEditing, endEdit } from "../editorController";
import { resetNearObserverForTests } from "../lazyObserve";
import type { GraphMeta, PageRead, JournalFeedPage } from "../types";
import * as viewportAnchor from "./journalViewportAnchor";

beforeAll(initParser);

it("GF4: immediate completion merges before first paint without viewport anchor work", async () => {
  const frames: FrameRequestCallback[] = [];
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
  const capture = vi.spyOn(viewportAnchor, "captureJournalViewport");
  const day = (name: string): PageRead => ({ id: `journals/${name}.md`, name, title: name, kind: "journal", pre_block: null, blocks: [] });
  const current = day(journalTitle(new Date()));
  const old = day("Jan 1st, 2000");
  const late = day("Jan 2nd, 2000");
  const response = (pages: PageRead[], pending = false): JournalFeedPage => ({ pages, references_pending: pending, as_of_day: currentDayKey(), done: true, next_before_day: null });
  const feed = vi.spyOn(backend(), "journalFeedPage").mockResolvedValueOnce(response([current, old], true)).mockResolvedValue(response([current, late, old]));
  const host = document.createElement("div"); host.className = "main-content"; document.body.append(host);
  const dispose = render(() => <PageView />, host);
  try {
    await vi.waitFor(() => expect(host.querySelectorAll("h1.journal-title")).toHaveLength(3));
    expect(feed).toHaveBeenCalledTimes(2);
    expect(frames.length).toBeGreaterThan(0);
    expect(capture).not.toHaveBeenCalled();
    for (const callback of frames.splice(0)) callback(performance.now());
    expect([...host.querySelectorAll("h1.journal-title")].map(el => el.textContent?.trim())).toEqual([current.name, late.name, old.name]);
    expect(host.scrollTop).toBe(0);
  } finally { dispose(); }
});

it("GF4: bodies render while references load, then late days preserve the focused editor and scroll anchor", async () => {
  const resizeCallbacks = new Map<Element, ResizeObserverCallback>();
  vi.stubGlobal("ResizeObserver", class {
    targets = new Set<Element>();
    constructor(private callback: ResizeObserverCallback) {}
    observe(target: Element) { this.targets.add(target); resizeCallbacks.set(target, this.callback); }
    unobserve(target: Element) { this.targets.delete(target); resizeCallbacks.delete(target); }
    disconnect() { for (const target of this.targets) resizeCallbacks.delete(target); this.targets.clear(); }
  });
  const today = journalTitle(new Date());
  const day = (name: string, id: string): PageRead => ({ id: `journals/${id}.md`, name, title: name, kind: "journal", pre_block: null,
    blocks: [{ id, raw: `Body ${id}`, children: [], collapsed: false }] });
  const old = day("Jan 1st, 2000", "gf4-old");
  const current = day(today, "gf4-today");
  const late = { ...day("Jan 3rd, 2000", "gf4-late"), blocks: [], rev: null };
  let finish!: (value: JournalFeedPage) => void;
  const feed = vi.spyOn(backend(), "journalFeedPage").mockResolvedValueOnce({ pages: [current, old], as_of_day: currentDayKey(), done: true, next_before_day: null, references_pending: true } as JournalFeedPage)
    .mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const host = document.createElement("div"); host.className = "main-content"; document.body.append(host);
  const dispose = render(() => <PageView />, host);
  try {
    await vi.waitFor(() => expect(host.textContent).toContain("Body gf4-today"));
    await vi.waitFor(() => expect(feed).toHaveBeenCalledTimes(2));
    startEditing("gf4-old");
    await vi.waitFor(() => expect(host.querySelector("textarea.block-editor")).not.toBeNull());
    const editor = host.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
    editor.focus();
    host.scrollTop = 100;
    // jsdom has no layout: model the displacement caused by a day inserted
    // above the viewport and require the scroller to compensate for it.
    let lateHeight = 0;
    vi.spyOn(editor, "getBoundingClientRect").mockImplementation(() => ({ top: 30 + (host.querySelectorAll(".page-section").length === 2 ? 0 : 140 + lateHeight) - (host.scrollTop - 100) } as DOMRect));
    const value = editor.value;
    finish({ pages: [current, late, old], as_of_day: currentDayKey(), done: true, next_before_day: null });
    await vi.waitFor(() => expect(host.querySelectorAll(".page-section")).toHaveLength(3));
    expect([...host.querySelectorAll("h1.journal-title")].map(el => el.textContent?.trim())).toEqual([today, late.name, old.name]);
    expect(host.querySelector("textarea.block-editor")).toBe(editor);
    expect(document.activeElement).toBe(editor);
    expect(editor.value).toBe(value);
    expect(host.scrollTop).toBe(240);
    const resize = resizeCallbacks.get(host.querySelector(".page")!)!;
    lateHeight = 40; // The inserted day's linked references finish later.
    resize([], {} as ResizeObserver);
    expect(host.scrollTop).toBe(280);
    host.dispatchEvent(new Event("wheel"));
    lateHeight = 80;
    resize([], {} as ResizeObserver);
    expect(host.scrollTop).toBe(280);
  } finally { dispose(); }
});

it("GF4: completion covers days appended during loading, then retires with its route", async () => {
  const makeDay = (name: string): PageRead => ({ id: `journals/${name}.md`, name, title: name, kind: "journal", pre_block: null, blocks: [] });
  const current = makeDay(journalTitle(new Date()));
  const old = makeDay("Jan 3rd, 2000");
  const older = makeDay("Jan 1st, 2000");
  const late = makeDay("Jan 2nd, 2000");
  const response = (pages: PageRead[], pending = false): JournalFeedPage => ({ pages, references_pending: pending, as_of_day: currentDayKey(), done: true, next_before_day: null });
  const completions: ((value: JournalFeedPage) => void)[] = [];
  const read = vi.spyOn(backend(), "journalFeedPage").mockResolvedValueOnce(response([current, old], true))
    .mockImplementation(() => new Promise(resolve => completions.push(resolve)));
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <PageView />, host);
  try {
    await vi.waitFor(() => expect(completions).toHaveLength(1));
    appendFeed([older]);
    completions[0](response([current, old]));
    await vi.waitFor(() => expect(completions).toHaveLength(2));
    expect(read.mock.calls.at(-1)).toEqual([3, null, true, 20000101]);
    vi.spyOn(backend(), "getPage").mockResolvedValue({ ...makeDay("Elsewhere"), kind: "page", id: "pages/Elsewhere.md" });
    openPage("Elsewhere", "page");
    await vi.waitFor(() => expect(host.querySelector("h1.page-title")?.textContent).toContain("Elsewhere"));
    completions[1](response([current, old, late, older]));
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(pageByName(late.name)).toBeUndefined();
    expect(host.querySelector("h1.page-title")?.textContent).toContain("Elsewhere");
  } finally { dispose(); }
});

it("GF4: a remaining Journals surface owns the merge when the surface that started it closes", async () => {
  const day = (name: string): PageRead => ({ id: `journals/${name}.md`, name, title: name, kind: "journal", pre_block: null, blocks: [] });
  const current = day(journalTitle(new Date()));
  const late = day("Jan 2nd, 2000");
  let finish!: (value: JournalFeedPage) => void;
  vi.spyOn(backend(), "journalFeedPage").mockImplementation((_limit, _before, wait) => wait
    ? new Promise(resolve => { finish = resolve; })
    : Promise.resolve({ pages: [current], references_pending: true, as_of_day: currentDayKey(), done: true, next_before_day: null }));
  const first = document.createElement("div"); const second = document.createElement("div"); document.body.append(first, second);
  const disposeFirst = render(() => <PageView />, first);
  const disposeSecond = render(() => <PageView />, second);
  try {
    await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
    disposeSecond();
    finish({ pages: [current, late], as_of_day: currentDayKey(), done: true, next_before_day: null });
    await vi.waitFor(() => expect(first.textContent).toContain(late.name));
  } finally { disposeFirst(); disposeSecond(); }
});
afterEach(() => {
  endEdit("blur");
  resetStore(); resetTabsToJournals(); setGraphMeta(null); resetReferenceSectionState();
  resetNearObserverForTests(); vi.unstubAllGlobals(); vi.restoreAllMocks();
  document.body.replaceChildren();
});

it("GH #481: feed-source editing refreshes journal backlinks without leaving Journals", async () => {
  vi.stubGlobal("IntersectionObserver", class {
    constructor(private callback: IntersectionObserverCallback) {}
    observe(target: Element) { if (target.classList.contains("journal-linked-references")) this.callback([{ target, isIntersecting: true } as IntersectionObserverEntry], this as unknown as IntersectionObserver); }
    unobserve() {} disconnect() {}
  });
  const today = journalTitle(new Date());
  const older = "Sep 20th, 2026";
  const block = { id: "feed-source", raw: `Reminder [[${older}]]`, children: [], collapsed: false };
  const pages: PageRead[] = [today, older].map((name, index) => ({ name, title: name, kind: "journal", id: `journals/live-${index}.md`, pre_block: null, blocks: index ? [] : [block] }));
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages, as_of_day: currentDayKey(), next_before_day: null, done: true });
  const backlinks = vi.spyOn(backend(), "getBacklinks").mockImplementation(async name => name === older ? [{ page: today, kind: "journal", blocks: [block] }] : []);
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <PageView />, host);
  try {
    await vi.waitFor(() => expect(host.querySelector(".references-count")?.textContent).toBe("1"));
    startEditing("feed-source");
    backlinks.mockResolvedValue([]);
    bumpDataRev();
    await vi.waitFor(() => expect(host.querySelector(".linked-references")).toBeNull());
    expect(host.querySelector(".page-section")).not.toBeNull();
  } finally { dispose(); }
});

it("Journals feed loads each day's shared references only on approach, omits zero and honors collapse", async () => {
  const observed = new Set<Element>();
  const callbacks = new Map<Element, IntersectionObserverCallback>();
  vi.stubGlobal("IntersectionObserver", class {
    constructor(private callback: IntersectionObserverCallback) {}
    observe(el: Element) { observed.add(el); callbacks.set(el, this.callback); }
    unobserve(el: Element) { observed.delete(el); }
    disconnect() { observed.clear(); }
  });
  setGraphMeta({ linked_references_collapsed_threshold: 0 } as GraphMeta);
  const today = journalTitle(new Date());
  const pages: PageRead[] = [today, "Sep 20th, 2026", "Sep 19th, 2026"].map((name, i) => ({
    id: `journals/qc2-${i}.md`, name, title: name, kind: "journal", pre_block: null,
    blocks: [{ id: `qc2-${i}`, raw: `Journal body ${i}`, collapsed: false, children: [] }],
  }));
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages, as_of_day: currentDayKey(), next_before_day: null, done: true });
  const backlinks = vi.spyOn(backend(), "getBacklinks").mockImplementation(async name => name === pages[2].name ? [] : [{
    page: "Planning", kind: "page", blocks: [{ id: `ref-${name}`, raw: `Reminder [[${name}]]`, collapsed: false, children: [] }],
  }]);
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <PageView />, host);
  try {
    await vi.waitFor(() => expect(host.querySelectorAll(".journal-linked-references")).toHaveLength(3));
    expect(backlinks).not.toHaveBeenCalled();
    const sections = [...host.querySelectorAll(".journal-linked-references")];
    expect(sections.every(section => observed.has(section))).toBe(true);
    const approach = (target: Element) => callbacks.get(target)!([{ target, isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver);
    approach(sections[1]);
    await vi.waitFor(() => expect(sections[1].querySelector(".references-count")?.textContent).toBe("1"));
    expect(backlinks.mock.calls.map(([name]) => name)).toEqual([pages[1].name]);
    expect(sections[1].querySelector(".ref-collapse.collapsed")).not.toBeNull();
    expect(sections[1].querySelector(".live-ref-group")).toBeNull();
    (sections[1].querySelector(".references-header") as HTMLElement).click();
    expect(sections[1].querySelector(".live-ref-group")).not.toBeNull();
    approach(sections[0]); approach(sections[2]);
    await vi.waitFor(() => expect(backlinks).toHaveBeenCalledTimes(3));
    await vi.waitFor(() => expect(sections[0].textContent).toContain("Linked References"));
    expect(sections[2].querySelector(".linked-references")).toBeNull();
    approach(sections[1]);
    expect(backlinks).toHaveBeenCalledTimes(3);
  } finally { dispose(); }
  expect([...observed].some(el => el.classList.contains("journal-linked-references"))).toBe(false);
});

it("the feed displays a counted reference header on the mentioned day, even with empty journal content", async () => {
  vi.stubGlobal("IntersectionObserver", class {
    constructor(private callback: IntersectionObserverCallback) {}
    observe(target: Element) { if (target.classList.contains("journal-linked-references")) this.callback([{ target, isIntersecting: true } as IntersectionObserverEntry], this as unknown as IntersectionObserver); }
    unobserve() {} disconnect() {}
  });
  const today = journalTitle(new Date());
  const pages: PageRead[] = [today, "Sep 20th, 2026"].map((name, i) => ({
    id: `journals/qc2-outcome-${i}.md`, name, title: name, kind: "journal", pre_block: null, blocks: [],
  }));
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages, as_of_day: currentDayKey(), next_before_day: null, done: true });
  vi.spyOn(backend(), "getBacklinks").mockImplementation(async name => name === today ? [] : [{
    page: "Planning", kind: "page", blocks: [{ id: "reminder", raw: `Reminder [[${pages[1].name}]]`, collapsed: false, children: [] }],
  }]);
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <PageView />, host);
  try {
    await vi.waitFor(() => expect(host.querySelector(".references-count")?.textContent).toBe("1"));
    const days = host.querySelectorAll(".page-section");
    expect(days).toHaveLength(2);
    expect(days[0].querySelector(".linked-references")).toBeNull();
    expect(days[1].textContent).toContain("Sep 20th, 2026");
    expect(days[1].textContent).toContain("Linked References");
  } finally { dispose(); }
});

it("GH #710: fileless mentions render without writes and Upcoming is opt-in, keeping today first", async () => {
  setGraphMeta({ linked_references_collapsed_threshold: 0 } as GraphMeta);
  vi.stubGlobal("IntersectionObserver", class {
    constructor(private callback: IntersectionObserverCallback) {}
    observe(target: Element) { if (target.classList.contains("journal-linked-references")) this.callback([{ target, isIntersecting: true } as IntersectionObserverEntry], this as unknown as IntersectionObserver); }
    unobserve() {} disconnect() {}
  });
  const day = (name: string): PageRead => ({ id: `journals/${name}.md`, name, title: name, kind: "journal", pre_block: null, blocks: [], rev: null });
  const today = journalTitle(new Date());
  const past = day("Jan 3rd, 2000");
  const future = day("Jan 3rd, 2099");
  const feed = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages: [past], upcoming: [future], as_of_day: currentDayKey(), next_before_day: null, done: true } as JournalFeedPage);
  vi.spyOn(backend(), "getBacklinks").mockImplementation(async name => name === today ? [] : [{ page: "Planning", kind: "page", blocks: [{ id: `mention-${name}`, raw: `Remember [[${name}]]`, children: [], collapsed: false }] }]);
  const save = vi.spyOn(backend(), "pageSubmit");
  const locate = vi.spyOn(backend(), "queryRun").mockResolvedValue({ anchor: "page", pages: [] } as never);
  const sectionsPage: PageRead = { id: "pages/Sections.md", name: "Sections", title: "Sections", kind: "page", pre_block: "tine/journal-sections:: true", blocks: [{ id: "upcoming", raw: "{{upcoming}}", children: [], collapsed: false }] };
  vi.spyOn(backend(), "getPage").mockImplementation(async name => name === "Sections" ? sectionsPage : null);
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <PageView />, host);
  try {
    await vi.waitFor(() => expect(host.querySelectorAll(".page-section")).toHaveLength(2));
    expect(host.querySelector(".page-section")?.textContent).toContain(today);
    expect(host.querySelector(".journal-upcoming")).toBeNull();
    locate.mockResolvedValue({ anchor: "page", pages: [{ name: "Sections" }] } as never);
    bumpDataRev();
    await vi.waitFor(() => expect(host.querySelector(".journal-upcoming")).not.toBeNull());
    const upcoming = host.querySelector<HTMLDetailsElement>(".journal-upcoming")!;
    expect(upcoming.open).toBe(false);
    expect(upcoming.textContent).toContain("Upcoming");
    upcoming.open = true; upcoming.dispatchEvent(new Event("toggle"));
    await vi.waitFor(() => expect(upcoming.textContent).toContain(future.name));
    const entry = upcoming.querySelector<HTMLDetailsElement>("details")!;
    entry.open = true; entry.dispatchEvent(new Event("toggle"));
    await vi.waitFor(() => expect(entry.querySelector(".references-count")?.textContent).toBe("1"));
    expect(entry.querySelector(".ref-collapse.collapsed")).not.toBeNull();
    expect(entry.querySelector("a")).not.toBeNull();
    // A direct today route also has the opted-in section, independently of
    // the Journals route's cached feed metadata.
    openPage(today, "journal");
    await vi.waitFor(() => expect(host.querySelectorAll(".page-section")).toHaveLength(1));
    await vi.waitFor(() => expect(host.querySelector(".journal-upcoming")).not.toBeNull());
    resetTabsToJournals();
    await flushAll();
    expect(save).not.toHaveBeenCalled();
    feed.mockResolvedValue({ pages: [], upcoming: [], as_of_day: currentDayKey(), next_before_day: null, done: true } as JournalFeedPage);
    bumpDataRev();
    await vi.waitFor(() => expect(host.querySelectorAll(".page-section")).toHaveLength(1));
    expect(host.querySelector(".journal-upcoming")).toBeNull();
  } finally { dispose(); }
});
