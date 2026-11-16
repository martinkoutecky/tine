import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { JournalFeedDays } from "./JournalFeedDays";
import { initParser } from "../render/parse";
import { loadFeed, node, pageByName, resetStore } from "../document";
import { backend } from "../backend";
import type { PageDto } from "../types";

const day = (i: number, raw = `day ${i}`): PageDto => ({ name: `Day${i}`, title: `Day${i}`, kind: "journal", format: "md",
  pre_block: null, blocks: [{ id: `day-${i}-${raw}`, raw, collapsed: false, children: [] }] });
beforeAll(initParser);
afterEach(() => { resetStore(); vi.restoreAllMocks(); vi.unstubAllGlobals(); document.body.innerHTML = ""; });
const content = (page: { roots: readonly string[] }) => <div>{page.roots.map(id => node(id)?.raw).join(" ")}</div>;

it("replaces a loaded day instance when a feed refresh publishes new roots", () => {
  loadFeed([day(0, "before")]);
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <JournalFeedDays>{content}</JournalFeedDays>, host);
  try {
    expect(host.textContent).toBe("before");
    loadFeed([day(0, "after")]);
    expect(host.textContent).toBe("after");
  } finally { dispose(); }
});

it("requires every surface to be away, preserves measured placeholders and reloads current content", async () => {
  const observers: FakeObserver[] = [];
  class FakeObserver {
    elements = new Set<Element>();
    constructor(readonly callback: IntersectionObserverCallback) { observers.push(this); }
    observe(el: Element) { this.elements.add(el); }
    unobserve(el: Element) { this.elements.delete(el); }
    disconnect() { this.elements.clear(); }
    emit(near: (el: Element) => boolean) {
      this.callback([...this.elements].map(target => ({ target, isIntersecting: near(target) })) as IntersectionObserverEntry[], this as unknown as IntersectionObserver);
    }
  }
  vi.stubGlobal("IntersectionObserver", FakeObserver);
  let bodyHeight = 100;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    return { height: this.classList.contains("journal-feed-day") ? 100 : bodyHeight } as DOMRect;
  });
  loadFeed(Array.from({ length: 20 }, (_, i) => day(i)));
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <><JournalFeedDays>{content}</JournalFeedDays><JournalFeedDays>{content}</JournalFeedDays></>, host);
  let disposeReopened: (() => void) | undefined;
  try {
    observers[0].emit(() => false);
    observers[1].emit(el => el.getAttribute("data-feed-day") === "Day0");
    await vi.waitFor(() => expect(host.querySelectorAll("[data-feed-retired]")).toHaveLength(16));
    expect(pageByName("Day0")).toBeDefined();
    expect(pageByName("Day1")).toBeUndefined();
    expect(host.querySelector<HTMLElement>('[data-feed-day="Day1"]')?.style.minHeight).toBe("100px");
    const reopened = document.createElement("div"); document.body.append(reopened);
    disposeReopened = render(() => <JournalFeedDays>{content}</JournalFeedDays>, reopened);
    expect(reopened.querySelector<HTMLElement>('[data-feed-day="Day1"]')?.style.minHeight).toBe("100px");
    disposeReopened(); disposeReopened = undefined;
    bodyHeight = 60; // async sections have not filled their previous space yet
    vi.spyOn(backend(), "getPage").mockResolvedValue({ ...day(1, "fresh disk content"), id: "journals/Day1.md" });
    observers[0].emit(el => el.getAttribute("data-feed-day") === "Day1");
    await vi.waitFor(() => expect(host.querySelector('[data-feed-day="Day1"]')?.textContent).toBe("fresh disk content"));
    expect(pageByName("Day0")).toBeDefined();
    expect(host.querySelector('[data-feed-day="Day1"]')?.hasAttribute("data-feed-retired")).toBe(false);
    const reloaded = host.querySelector<HTMLElement>('[data-feed-day="Day1"]')!;
    expect(reloaded.style.minHeight).toBe("100px");
    reloaded.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    expect(reloaded.style.minHeight).toBe("");
  } finally { disposeReopened?.(); dispose(); }
});
