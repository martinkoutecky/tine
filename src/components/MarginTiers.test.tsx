import { afterAll, afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { SurfaceContext } from "./Block";
import { BlockList } from "./BlockList";
import { createMarginSurface } from "./MarginColumn";
import { MarginContext } from "./marginContext";
import { CommentSurface } from "./CommentSurface";
import { CommentResolutionScope } from "./CommentResolutionScope";
import { blockRegions, initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { flushPage, node, resetStore, setCollapsed, setRaw } from "../document";
import { commentTier } from "../margin";
import { pageByName } from "../document/model";
import { persistCommentMarginHidden } from "../commentMarginPref";
import { resetCommentAuthorNameForTests } from "../commentIdentity";
import { endEdit, startEditing } from "../editorController";
import { editorIn } from "../tests/editorDriver";
import { installKeybindings } from "../keybindings";
import { bindTestHost } from "../document/host/wiring.test.support";
import { answerOpensFromDocument } from "../document/host/documentHost.test.support";
import type { BlockDto } from "../types";
import { backend } from "../backend";

let dispose: (() => void) | undefined;
let keys: (() => void) | undefined;
beforeAll(async () => { await initParser(); keys = installKeybindings(); });
afterAll(() => keys?.());
beforeEach(async () => { answerOpensFromDocument(await bindTestHost()); persistCommentMarginHidden(false); resetCommentAuthorNameForTests("Ada"); });
afterEach(() => { endEdit("page-navigation"); dispose?.(); resetStore(); document.body.replaceChildren(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
function b(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  return { id, raw, children, collapsed: false, properties: blockRegions(raw, "md").properties.filter(p => p.primary).map(p => [p.key, p.value]) };
}
function mount(width: number, surface = "main") {
  loadSingle({ name: "Tiers", title: "Tiers", kind: "page", pre_block: null, format: "md", blocks: [
    b("p", "First passage then second passage", [b("one", "First thread\ntine.quote:: First", [b("reply", "Existing reply")]), b("two", "Second thread\ntine.quote:: second")]),
  ] });
  const pane = document.createElement("main"); pane.className = "main-content";
  Object.defineProperty(pane, "clientWidth", { configurable: true, get: () => width }); document.body.append(pane);
  if (surface !== "main") vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function (this: HTMLElement) { return this.classList.contains("comment-surface") ? width : 0; });
  dispose = render(() => {
    if (surface !== "main") return <SurfaceContext.Provider value={surface}><CommentSurface page={pageByName("Tiers")}><BlockList ids={pageByName("Tiers")?.roots ?? []} /></CommentSurface></SurfaceContext.Provider>;
    let outline: HTMLDivElement | undefined;
    const margin = createMarginSurface({ page: () => pageByName("Tiers"), eligible: () => true, outline: () => outline });
    return <SurfaceContext.Provider value={surface}><CommentResolutionScope roots={pageByName("Tiers")?.roots ?? []} format="md"><MarginContext.Provider value={margin.placement}>
      <div class="page-section"><div class="page-blocks" ref={outline}><BlockList ids={pageByName("Tiers")?.roots ?? []} /></div><margin.Column /></div>
    </MarginContext.Provider></CommentResolutionScope></SurfaceContext.Provider>;
  }, pane);
  return pane;
}
const popup = () => document.querySelector<HTMLElement>(".comment-thread-popup")!;
const chip = (pane: HTMLElement) => pane.querySelector<HTMLButtonElement>(".margin-marker")!;
function send(text: string, id = "one") {
  const draft = popup().querySelector<HTMLTextAreaElement>(`[data-reply-root="${id}"]`)!;
  draft.focus(); draft.value = text; draft.dispatchEvent(new Event("input", { bubbles: true }));
  draft.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true }));
}
it.each([800, 390])("width %s removes inline threads, counts roots, opens while folded and replies live", width => {
  const pane = mount(width);
  expect(pane.querySelector(".page-blocks .comment-card")).toBeNull(); expect(chip(pane)?.textContent).toBe("2");
  setCollapsed("p", true); chip(pane).click();
  expect(popup()?.getAttribute("role")).toBe("dialog");
  expect(popup().classList.contains("comment-bottom-sheet")).toBe(width === 390);
  expect(popup().textContent).toContain("First thread"); send("Live reply");
  const id = node("one")!.children.at(-1)!;
  expect(node(id)!.raw).toContain("Live reply"); expect(node(id)!.parent).toBe("one");
});
it("medium markers and highlights open the selected live editable thread", async () => {
  const pane = mount(800); pane.querySelector<HTMLButtonElement>('[data-comment-marker="two"]')!.click();
  expect(popup().textContent).toContain("Second thread"); startEditing("two", 0); await Promise.resolve();
  const editor = editorIn(popup()); editor.input("Changed thread", 14); editor.keydown({ key: "Escape" });
  expect(node("two")!.raw).toContain("Changed thread\ntine.quote:: second");
  popup().querySelector<HTMLButtonElement>(".comment-popup-close")!.click();
  pane.querySelector<HTMLElement>('[data-comment-id="one"]')!.click(); expect(popup().textContent).toContain("First thread");
});
it("phone navigation preserves unsent drafts while moving between threads", () => {
  const pane = mount(390); chip(pane).click();
  const draft = popup().querySelector<HTMLTextAreaElement>(".comment-reply-draft")!;
  draft.value = "Keep my draft"; draft.dispatchEvent(new Event("input", { bubbles: true }));
  popup().querySelector<HTMLButtonElement>(".comment-next-thread")!.click(); expect(popup().textContent).toContain("Second thread");
  popup().querySelector<HTMLButtonElement>(".comment-prev-thread")!.click();
  expect(popup().querySelector<HTMLTextAreaElement>(".comment-reply-draft")!.value).toBe("Keep my draft");
});
it("phone swipes change passages and keep the selected source's threads reachable", () => {
  const pane = mount(390); chip(pane).click();
  const content = popup().querySelector(".comment-popup-content")!;
  for (const [type, x] of [["touchstart", 180], ["touchend", 50]] as const) {
    const event = new Event(type, { bubbles: true });
    Object.defineProperties(event, { touches: { value: [{ clientX: x, clientY: 30 }] }, changedTouches: { value: [{ clientX: x, clientY: 32 }] } }); content.dispatchEvent(event);
  }
  expect(popup().querySelector('[data-comment-group="two"]')?.parentElement?.hidden).toBe(false);
  expect(popup().querySelector('[data-comment-group="one"]')?.parentElement?.hidden).toBe(true);
});
it("open counts omit resolved roots and a reply's quote metadata", () => {
  const pane = mount(800);
  setRaw("two", "Second thread\ntine.quote:: second\ntine.resolved:: true", { timetracking: false });
  setRaw("reply", "Existing reply\ntine.quote:: First", { timetracking: false });
  expect(chip(pane).textContent).toBe("1"); expect(pane.querySelectorAll(".comment-rail-marker")).toHaveLength(1);
});
it("deleting the selected root leaves another thread visible and replyable", () => {
  const pane = mount(800); chip(pane).click();
  popup().querySelector<HTMLButtonElement>(".comment-delete")!.click();
  expect(node("one")).toBeUndefined();
  expect(popup().querySelector('[data-comment-group="two"]')?.parentElement?.hidden).toBe(false);
  send("Remaining thread reply", "two");
  expect(node(node("two")!.children[0])!.raw).toContain("Remaining thread reply");
});
it("releasing a source block closes its compact thread surface", () => {
  const pane = mount(800); chip(pane).click();
  loadSingle({ name: "Tiers", title: "Tiers", kind: "page", pre_block: null, format: "md", blocks: [] });
  expect(popup()).toBeNull(); expect(pane.querySelector(".margin-marker")).toBeNull();
});
it("reply Escape cancels the draft before dismissing the preview, and closing retains other unsent drafts", () => {
  const pane = mount(800); chip(pane).click();
  const draft = popup().querySelector<HTMLTextAreaElement>(".comment-reply-draft")!;
  draft.focus(); draft.value = "Discard me"; draft.dispatchEvent(new Event("input", { bubbles: true }));
  draft.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
  expect(popup()).not.toBeNull(); expect(draft.value).toBe("");
  draft.focus(); draft.value = "Retain me"; draft.dispatchEvent(new Event("input", { bubbles: true }));
  popup().querySelector<HTMLButtonElement>(".comment-popup-close")!.click(); chip(pane).click();
  expect(popup().querySelector<HTMLTextAreaElement>(".comment-reply-draft")!.value).toBe("Retain me");
});
it("the same width measurement selects the tiers at their boundaries", () => {
  expect([390, 480, 481, 959, 960, 1600].map(commentTier)).toEqual(["phone", "phone", "medium", "medium", "wide", "wide"]);
});
it("a held editor survives medium/phone transitions while the preview changes placement", async () => {
  const callbacks: ResizeObserverCallback[] = [];
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: ResizeObserverCallback) { callbacks.push(callback); }
    observe() {} unobserve() {} disconnect() {}
  });
  const pane = mount(800); chip(pane).click(); startEditing("one", 0); await Promise.resolve();
  const editor = editorIn(popup()); editor.focus(); editor.input("Keep editing");
  Object.defineProperty(pane, "clientWidth", { configurable: true, get: () => 390 });
  for (const callback of callbacks) callback([], {} as ResizeObserver);
  await Promise.resolve(); await Promise.resolve();
  expect(popup().classList.contains("comment-bottom-sheet")).toBe(true);
  expect(popup().style.bottom).toBe("0px"); expect(popup().style.top).toBe("");
  expect(editorIn(popup()).element).toBe(editor.element);
  Object.defineProperty(pane, "clientWidth", { configurable: true, get: () => 800 });
  for (const callback of callbacks) callback([], {} as ResizeObserver);
  await Promise.resolve(); await Promise.resolve();
  expect(popup().classList.contains("comment-bottom-sheet")).toBe(false);
  expect(popup().style.top).not.toBe(""); expect(popup().style.bottom).toBe("");
  expect(editorIn(popup()).element).toBe(editor.element); expect(editor.value()).toBe("Keep editing");
});
it("a held phone editor stays above a shrinking visual viewport", async () => {
  const visible = new EventTarget() as EventTarget & { height: number; width: number; offsetTop: number };
  Object.assign(visible, { height: innerHeight, width: 390, offsetTop: 0 }); vi.stubGlobal("visualViewport", visible);
  const pane = mount(390); chip(pane).click(); startEditing("one", 0); await Promise.resolve();
  const editor = editorIn(popup()); editor.focus();
  visible.height = 300; visible.dispatchEvent(new Event("resize"));
  expect(popup()).not.toBeNull(); expect(parseFloat(popup().style.maxHeight)).toBeLessThan(visible.height);
  expect(parseFloat(popup().style.bottom)).toBeGreaterThan(0); expect(editorIn(popup()).element).toBe(editor.element);
});
it("opening a phone thread reveals its source in the visible text strip above an existing keyboard", () => {
  const visible = Object.assign(new EventTarget(), { height: 300, width: 390, offsetTop: 0 }); vi.stubGlobal("visualViewport", visible);
  const pane = mount(390); pane.style.overflowY = "auto";
  Object.defineProperties(pane, { clientHeight: { configurable: true, value: 300 }, scrollHeight: { configurable: true, value: 1000 } });
  const anchor = chip(pane);
  vi.spyOn(anchor, "getBoundingClientRect").mockReturnValue({ x: 0, y: 200, top: 200, bottom: 218, left: 0, right: 20, width: 20, height: 18, toJSON() {} });
  anchor.click(); expect(pane.scrollTop).toBeGreaterThan(0);
  visible.height = innerHeight; visible.dispatchEvent(new Event("resize")); expect(popup()).not.toBeNull();
});
it.each(["sidebar:item", "ref:item", "embed:item"].flatMap(surface => [390, 800, 1400].map(width => ({ surface, width }))))("$surface uses its own width $width for the same tiers", ({ surface, width }) => {
  const pane = mount(width, surface);
  expect(pane.querySelector(".page-blocks > .ls-block > .block-children-container .comment-card")).toBeNull();
  if (width >= 960) expect(pane.querySelectorAll(".margin-column .comment-card")).toHaveLength(2);
  else {
    chip(pane).click(); expect(popup()).not.toBeNull();
    expect(popup().classList.contains("comment-bottom-sheet")).toBe(width <= 480);
  }
});
it.each([800, 390])("Show/Hide comments removes every entry point and open surface at %s", width => {
  const pane = mount(width); chip(pane).click(); persistCommentMarginHidden(true);
  expect(document.querySelector(".comment-thread-popup,.margin-marker,.comment-rail-marker,.comment-quote-anchor")).toBeNull();
});
it("wide expanded blocks have no count badge beside visible margin threads", () => {
  const pane = mount(1400); expect(pane.querySelectorAll(".margin-column .comment-card")).toHaveLength(2);
  expect(pane.querySelector("button.margin-marker")).toBeNull();
});

it.each([800, 390])("width %s reveals closed-only threads with dates and Reopen without open entry points", width => {
  const pane = mount(width);
  setRaw("one", "First thread\ntine.quote:: First\ntine.date:: 2026-11-07T13:37Z", { timetracking: false });
  chip(pane).click();
  popup().querySelector<HTMLButtonElement>('[data-block-id="one"] .comment-resolve')!.click();
  setRaw("two", "Second thread\ntine.quote:: second\ntine.resolved:: true", { timetracking: false });
  expect(document.querySelector(".comment-thread-popup,.margin-marker,.comment-rail-marker,.comment-quote-anchor")).toBeNull();
  pane.querySelector<HTMLButtonElement>(".comment-resolved-toggle")!.click();
  expect(pane.querySelector(".margin-marker,.comment-rail-marker,.comment-quote-anchor")).toBeNull();
  const entry = pane.querySelector<HTMLButtonElement>(".comment-resolved-entry")!;
  expect(entry?.textContent).toContain("2"); entry.click();
  expect(popup().classList.contains("comment-bottom-sheet")).toBe(width === 390);
  expect(popup().querySelector(".comment-resolved time")?.getAttribute("datetime")).toBe("2026-11-07T13:37Z");
  popup().querySelector<HTMLButtonElement>('[data-block-id="one"] .comment-resolve')!.click();
  expect(node("one")!.raw).not.toContain("tine.resolved");
  expect(chip(pane)?.textContent).toBe("1");
  expect(pane.querySelectorAll(".comment-quote-anchor")).toHaveLength(1);
  expect(pane.querySelectorAll(".comment-rail-marker")).toHaveLength(width === 390 ? 0 : 1);
  persistCommentMarginHidden(true);
  expect(document.querySelector(".comment-thread-popup,.comment-resolved-toggle,.comment-resolved-entry,.margin-marker,.comment-rail-marker,.comment-quote-anchor")).toBeNull();
});

it.each(["sidebar:item", "ref:item", "embed:item"].flatMap(surface => [800, 390].map(width => ({ surface, width }))))("$surface width $width reveals closed threads and replying reopens them", ({ surface, width }) => {
  const pane = mount(width, surface);
  setRaw("one", "First thread\ntine.quote:: First\ntine.resolved:: true\ntine.date:: 2026-11-07T13:37Z", { timetracking: false });
  expect(chip(pane)?.textContent).toBe("1");
  pane.querySelector<HTMLButtonElement>(".comment-resolved-toggle")?.click();
  const entry = pane.querySelector<HTMLButtonElement>(".comment-resolved-entry");
  expect(entry).not.toBeNull(); entry!.click();
  expect(popup().querySelector('.comment-resolved .comment-date')?.getAttribute("datetime")).toBe("2026-11-07T13:37Z");
  send("Reply reopens");
  expect(node("one")!.raw).not.toContain("tine.resolved"); expect(chip(pane).textContent).toBe("2");
  expect(node(node("one")!.children.at(-1)!)!.raw).toContain("tine.date::");
});

it.each([800, 390])("width %s retains a closed thread's unsent draft when its compact view is closed", width => {
  const pane = mount(width);
  setRaw("one", node("one")!.raw + "\ntine.resolved:: true", { timetracking: false });
  const reveal = pane.querySelector<HTMLButtonElement>(".comment-resolved-toggle")!;
  reveal.click(); pane.querySelector<HTMLButtonElement>(".comment-resolved-entry")!.click();
  const draft = popup().querySelector<HTMLTextAreaElement>('[data-reply-root="one"]')!;
  draft.value = "Retain closed draft"; draft.dispatchEvent(new Event("input", { bubbles: true }));
  popup().querySelector<HTMLButtonElement>(".comment-popup-close")!.click();
  reveal.click(); expect(reveal.getAttribute("aria-pressed")).toBe("true");
  pane.querySelector<HTMLButtonElement>(".comment-resolved-entry")!.click();
  expect(popup().querySelector<HTMLTextAreaElement>('[data-reply-root="one"]')!.value).toBe("Retain closed draft");
});

it.each([800, 390])("width %s compact reveal and passage navigation perform zero page reads or writes", async width => {
  const pane = mount(width);
  setRaw("one", node("one")!.raw + "\ntine.resolved:: true", { timetracking: false });
  await flushPage("Tiers");
  const reads = vi.spyOn(backend(), "pageOpen"); reads.mockClear();
  const writes = vi.spyOn(backend(), "pageSubmit"); writes.mockClear();
  const started = performance.now();
  pane.querySelector<HTMLButtonElement>(".comment-resolved-toggle")!.click();
  pane.querySelector<HTMLButtonElement>(".comment-resolved-entry")!.click();
  popup().querySelector<HTMLButtonElement>(".comment-next-thread")!.click();
  popup().querySelector<HTMLButtonElement>(".comment-prev-thread")!.click();
  popup().querySelector<HTMLButtonElement>(".comment-popup-close")!.click();
  const elapsed = performance.now() - started;
  expect(elapsed).toBeLessThan(500);
  console.log(JSON.stringify({ measurement: "compact reveal, open, next, previous, close", width, milliseconds: elapsed, pageReads: 0, pageWrites: 0 }));
  await flushPage("Tiers");
  expect(reads).not.toHaveBeenCalled(); expect(writes).not.toHaveBeenCalled();
});
