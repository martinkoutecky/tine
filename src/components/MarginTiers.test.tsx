import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
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
import { AnnotationVisibilitySettings } from "./AnnotationVisibilitySettings";
import { hoverHighlight, leaveHighlight, openCommentThread, tapHighlight } from "../tests/commentEntry";
import { editingId } from "../editorController";
import { consumeReplyFocus, focusCommentReply } from "../commentReplyFocus";

let dispose: (() => void) | undefined;
let keys: (() => void) | undefined;
let disposeSettings: (() => void) | undefined;
beforeAll(async () => { await initParser(); keys = installKeybindings(); });
afterAll(() => keys?.());
beforeEach(async () => { answerOpensFromDocument(await bindTestHost()); persistCommentMarginHidden(false); resetCommentAuthorNameForTests("Ada"); });
afterEach(() => { endEdit("page-navigation"); dispose?.(); if (disposeSettings) chooseComments("margin"); disposeSettings?.(); disposeSettings = undefined; resetStore(); document.body.replaceChildren(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
function chooseComments(value: "margin" | "popup") {
  if (!disposeSettings) {
    const root = document.createElement("div"); document.body.append(root);
    disposeSettings = render(() => <AnnotationVisibilitySettings />, root);
  }
  const select = document.querySelector<HTMLSelectElement>('[data-setting-id="settings-comments-display"] select');
  expect(select, "Settings offers the Comments display choice").not.toBeNull();
  select!.value = value; select!.dispatchEvent(new Event("change", { bubbles: true }));
}
function b(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  return { id, raw, children, collapsed: false, properties: blockRegions(raw, "md").properties.filter(p => p.primary).map(p => [p.key, p.value]) };
}
const TIERS = () => [
  b("p", "First passage then second passage", [b("one", "First thread\ntine.quote:: First", [b("reply", "Existing reply")]), b("two", "Second thread\ntine.quote:: second")]),
];
function mount(width: number, surface = "main", blocks: BlockDto[] = TIERS()) {
  loadSingle({ name: "Tiers", title: "Tiers", kind: "page", pre_block: null, format: "md", blocks });
  const pane = document.createElement("main"); pane.className = "main-content";
  Object.defineProperty(pane, "clientWidth", { configurable: true, get: () => width }); document.body.append(pane);
  if (surface !== "main") vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function (this: HTMLElement) { return this.classList.contains("comment-surface") ? width : 0; });
  dispose = render(() => {
    if (surface !== "main") return <SurfaceContext.Provider value={surface}><CommentSurface page={pageByName("Tiers")}><BlockList ids={pageByName("Tiers")?.roots ?? []} /></CommentSurface></SurfaceContext.Provider>;
    let outline: HTMLDivElement | undefined;
    const margin = createMarginSurface({ page: () => pageByName("Tiers"), eligible: () => true, outline: () => outline });
    return <SurfaceContext.Provider value={surface}><CommentResolutionScope roots={pageByName("Tiers")?.roots ?? []}><MarginContext.Provider value={margin.placement}>
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
it.each([800, 390])("width %s removes inline threads, shows no count, opens while folded and replies live", width => {
  const pane = mount(width);
  expect(pane.querySelector(".page-blocks .comment-card")).toBeNull();
  // D-20261108-MC8: no count anywhere; phones keep a glyph chip as a tap target.
  if (width === 800) expect(chip(pane)).toBeNull(); else expect(chip(pane).textContent).toBe("");
  expect(pane.querySelector(".comment-rail-marker,.comment-rail")).toBeNull();
  setCollapsed("p", true); if (width === 390) chip(pane).click(); else openCommentThread(pane);
  expect(popup()?.getAttribute("role")).toBe("dialog");
  expect(popup().classList.contains("comment-bottom-sheet")).toBe(width === 390);
  expect(popup().textContent).toContain("First thread"); send("Live reply");
  const id = node("one")!.children.at(-1)!;
  expect(node(id)!.raw).toContain("Live reply"); expect(node(id)!.parent).toBe("one");
});
it("medium highlights open the selected live editable thread on tap", async () => {
  const pane = mount(800); openCommentThread(pane, "two");
  expect(popup().textContent).toContain("Second thread"); startEditing("two", 0); await Promise.resolve();
  const editor = editorIn(popup()); editor.input("Changed thread", 14); editor.keydown({ key: "Escape" });
  expect(node("two")!.raw).toContain("Changed thread\ntine.quote:: second");
  popup().querySelector<HTMLButtonElement>(".comment-popup-close")!.click();
  tapHighlight(pane.querySelector<HTMLElement>('[data-comment-id="one"]')!); expect(popup().textContent).toContain("First thread");
});
it("the wide margin appears for the first quote property and disappears for the last", () => {
  const pane = mount(1400, "main", [b("p", "First passage", [b("one", "Ordinary child")])]);
  expect(pane.querySelector(".margin-column")).toBeNull();
  setRaw("one", "First thread\ntine.quote:: First");
  expect(pane.querySelector('.margin-column [data-block-id="one"]')?.textContent).toContain("First thread");
  expect(pane.querySelector('.page-blocks [data-block-id="one"]')).toBeNull();
  setRaw("one", "Ordinary child again");
  expect(pane.querySelector(".margin-column")).toBeNull();
  expect(pane.querySelector('.page-blocks [data-block-id="one"]')?.textContent).toContain("Ordinary child again");
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
  // Review round 3 (PC-4): the passage not shown is not mounted at all.
  expect(popup().querySelector('[data-comment-group="one"]')).toBeNull();
});
it("resolved roots and a reply's quote metadata add no highlight, marker or count", () => {
  const pane = mount(800);
  setRaw("two", "Second thread\ntine.quote:: second\ntine.resolved:: true", { timetracking: false });
  setRaw("reply", "Existing reply\ntine.quote:: First", { timetracking: false });
  expect([...pane.querySelectorAll<HTMLElement>(".comment-quote-anchor")].map(mark => mark.dataset.commentId)).toEqual(["one"]);
  expect(chip(pane)).toBeNull();
});
it("deleting the selected root leaves another thread visible and replyable", () => {
  const pane = mount(800); openCommentThread(pane);
  popup().querySelector<HTMLButtonElement>(".comment-delete")!.click();
  expect(node("one")).toBeUndefined();
  expect(popup().querySelector('[data-comment-group="two"]')?.parentElement?.hidden).toBe(false);
  send("Remaining thread reply", "two");
  expect(node(node("two")!.children[0])!.raw).toContain("Remaining thread reply");
});
it("releasing a source block closes its compact thread surface", () => {
  const pane = mount(800); openCommentThread(pane);
  loadSingle({ name: "Tiers", title: "Tiers", kind: "page", pre_block: null, format: "md", blocks: [] });
  expect(popup()).toBeNull(); expect(pane.querySelector(".margin-marker")).toBeNull();
});
it("reply Escape cancels the draft before dismissing the preview, and closing retains other unsent drafts", () => {
  const pane = mount(800); openCommentThread(pane);
  const draft = popup().querySelector<HTMLTextAreaElement>(".comment-reply-draft")!;
  draft.focus(); draft.value = "Discard me"; draft.dispatchEvent(new Event("input", { bubbles: true }));
  draft.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
  expect(popup()).not.toBeNull(); expect(draft.value).toBe("");
  draft.focus(); draft.value = "Retain me"; draft.dispatchEvent(new Event("input", { bubbles: true }));
  popup().querySelector<HTMLButtonElement>(".comment-popup-close")!.click(); openCommentThread(pane);
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
  const pane = mount(800); openCommentThread(pane); startEditing("one", 0); await Promise.resolve();
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
// The entry span is empty without chips or rails (MC8): a thread opened by a
// request (new comment, Comment on existing, a link to it) sits below the
// commented block's text instead of covering it.
it.each([{ width: 800, display: "margin" }, { width: 1400, display: "popup" }] as const)("width $width ($display): a requested thread opens below the commented block's text", async ({ width, display }) => {
  const pane = mount(width); if (display === "popup") chooseComments("popup");
  const text = pane.querySelector<HTMLElement>('.ls-block[data-block-id="p"] > .block-main > .block-content-wrapper')!;
  vi.spyOn(text, "getBoundingClientRect").mockReturnValue({ x: 40, y: 100, top: 100, bottom: 140, left: 40, right: 600, width: 560, height: 40, toJSON() {} });
  focusCommentReply("two", null);
  await vi.waitFor(() => expect(popup()?.style.top).not.toBe(""));
  expect(parseFloat(popup().style.top)).toBeGreaterThanOrEqual(140);
  expect(popup().textContent).toContain("Second thread");
  consumeReplyFocus();
});
it.each(["sidebar:item", "ref:item", "embed:item"].flatMap(surface => [390, 800, 1400].map(width => ({ surface, width }))))("$surface uses its own width $width for the same tiers", ({ surface, width }) => {
  const pane = mount(width, surface);
  expect(pane.querySelector(".page-blocks > .ls-block > .block-children-container .comment-card")).toBeNull();
  if (width >= 960) expect(pane.querySelectorAll(".margin-column .comment-card")).toHaveLength(2);
  else {
    openCommentThread(pane); expect(popup()).not.toBeNull();
    expect(popup().classList.contains("comment-bottom-sheet")).toBe(width <= 480);
  }
});
it.each([800, 390])("Show/Hide comments removes every entry point and open surface at %s", width => {
  const pane = mount(width); openCommentThread(pane); persistCommentMarginHidden(true);
  expect(document.querySelector(".comment-thread-popup,.margin-marker,.comment-rail-marker,.comment-quote-anchor")).toBeNull();
});
it("wide expanded blocks have no count badge beside visible margin threads", () => {
  const pane = mount(1400); expect(pane.querySelectorAll(".margin-column .comment-card")).toHaveLength(2);
  expect(pane.querySelector("button.margin-marker")).toBeNull();
});

it.each([800, 390])("width %s reveals closed-only threads with dates and Reopen without open entry points", width => {
  const pane = mount(width);
  setRaw("one", "First thread\ntine.quote:: First\ntine.date:: 2026-11-07T13:37Z", { timetracking: false });
  openCommentThread(pane);
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
  if (width === 390) expect(chip(pane).textContent).toBe(""); else expect(chip(pane)).toBeNull();
  expect(pane.querySelectorAll(".comment-quote-anchor")).toHaveLength(1);
  persistCommentMarginHidden(true);
  expect(document.querySelector(".comment-thread-popup,.comment-resolved-toggle,.comment-resolved-entry,.margin-marker,.comment-rail-marker,.comment-quote-anchor")).toBeNull();
});

it.each(["sidebar:item", "ref:item", "embed:item"].flatMap(surface => [800, 390].map(width => ({ surface, width }))))("$surface width $width reveals closed threads and replying reopens them", ({ surface, width }) => {
  const pane = mount(width, surface);
  setRaw("one", "First thread\ntine.quote:: First\ntine.resolved:: true\ntine.date:: 2026-11-07T13:37Z", { timetracking: false });
  expect(pane.querySelectorAll(".comment-quote-anchor")).toHaveLength(1);
  if (width === 390) expect(chip(pane).textContent).toBe(""); else expect(chip(pane)).toBeNull();
  pane.querySelector<HTMLButtonElement>(".comment-resolved-toggle")?.click();
  const entry = pane.querySelector<HTMLButtonElement>(".comment-resolved-entry");
  expect(entry).not.toBeNull(); entry!.click();
  expect(popup().querySelector('.comment-resolved .comment-date')?.getAttribute("datetime")).toBe("2026-11-07T13:37Z");
  send("Reply reopens");
  expect(node("one")!.raw).not.toContain("tine.resolved"); expect(pane.querySelectorAll(".comment-quote-anchor")).toHaveLength(2);
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
it("In a popup replaces the wide margin live, opens highlights, and retains drafts when switched back", () => {
  const pane = mount(1400);
  expect(pane.querySelectorAll(".margin-column .comment-card")).toHaveLength(2);
  chooseComments("popup");
  expect(pane.querySelector(".margin-column,.page-blocks .comment-card,.margin-marker,.comment-rail-marker")).toBeNull();
  tapHighlight(pane.querySelector<HTMLElement>('[data-comment-id="two"]')!);
  expect(popup().textContent).toContain("Second thread");
  expect(popup().querySelector('[data-comment-group="two"]')!.closest("[hidden]")).toBeNull();
  popup().querySelector<HTMLButtonElement>(".comment-popup-close")!.click();
  tapHighlight(pane.querySelector<HTMLElement>('[data-comment-id="one"]')!);
  expect(popup().textContent).toContain("First thread");
  expect(popup().querySelector('[data-comment-group="one"]')!.closest("[hidden]")).toBeNull();
  const draft = popup().querySelector<HTMLTextAreaElement>(".comment-reply-draft")!;
  draft.value = "Keep across display choices"; draft.dispatchEvent(new Event("input", { bubbles: true }));
  chooseComments("margin");
  expect(popup()).toBeNull(); expect(pane.querySelectorAll(".margin-column .comment-card")).toHaveLength(2);
  expect(pane.querySelector<HTMLTextAreaElement>('[data-reply-root="one"]')!.value).toBe("Keep across display choices");
  chooseComments("popup"); openCommentThread(pane, "one");
  expect(popup().querySelector<HTMLTextAreaElement>('[data-reply-root="one"]')!.value).toBe("Keep across display choices");
  persistCommentMarginHidden(true);
  expect(document.querySelector(".comment-thread-popup,.margin-column,.margin-marker,.comment-rail-marker,.comment-quote-anchor")).toBeNull();
});
it.each(["main", "sidebar:item", "ref:item", "embed:item"])("In a popup applies to %s at wide width, including Wide mode", surface => {
  const pane = mount(1400, surface); pane.classList.add("wide-mode");
  chooseComments("popup");
  expect(pane.querySelector(".margin-column")).toBeNull(); openCommentThread(pane);
  expect(popup()).not.toBeNull(); expect(popup().classList.contains("comment-bottom-sheet")).toBe(false);
});
it("In a popup keeps the phone sheet and switching the choice preserves its open draft", () => {
  const pane = mount(390); chip(pane).click();
  const draft = popup().querySelector<HTMLTextAreaElement>(".comment-reply-draft")!;
  draft.value = "Phone draft"; draft.dispatchEvent(new Event("input", { bubbles: true }));
  chooseComments("popup");
  expect(popup().classList.contains("comment-bottom-sheet")).toBe(true);
  expect(popup().querySelector<HTMLTextAreaElement>(".comment-reply-draft")!.value).toBe("Phone draft");
  chooseComments("margin");
  expect(popup().querySelector<HTMLTextAreaElement>(".comment-reply-draft")!.value).toBe("Phone draft");
});

// D-20261108-MC7: in the popup display and the medium tier a highlight opens its
// thread through the hover preview's own trigger, delay and close rules.
describe("hovering a highlight", () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });
  const mark = (pane: HTMLElement, id: string) => pane.querySelector<HTMLElement>(`.comment-quote-anchor[data-comment-id="${id}"]`)!;
  async function advance(ms: number) { await vi.advanceTimersByTimeAsync(ms); await Promise.resolve(); }
  it.each([{ width: 800, display: "margin" }, { width: 1400, display: "popup" }] as const)("width $width ($display) opens after the preview delay and closes on leave", async ({ width, display }) => {
    const pane = mount(width); if (display === "popup") chooseComments("popup");
    hoverHighlight(mark(pane, "two"));
    await advance(999); expect(popup()).toBeNull();
    await advance(1); expect(popup()).not.toBeNull();
    expect(popup().querySelector('[data-comment-group="two"]')!.closest("[hidden]")).toBeNull();
    expect(popup().classList.contains("comment-bottom-sheet")).toBe(false);
    leaveHighlight(mark(pane, "two"));
    await advance(99); expect(popup()).not.toBeNull();
    await advance(1); expect(popup()).toBeNull();
  });
  it("leaving before the delay opens nothing, and entering the popover keeps it open", async () => {
    const pane = mount(800);
    hoverHighlight(mark(pane, "one")); await advance(500); leaveHighlight(mark(pane, "one"));
    await advance(2000); expect(popup()).toBeNull();
    hoverHighlight(mark(pane, "one")); await advance(1000);
    leaveHighlight(mark(pane, "one")); popup().dispatchEvent(new MouseEvent("mouseenter"));
    await advance(500); expect(popup()).not.toBeNull();
    popup().dispatchEvent(new MouseEvent("mouseleave")); await advance(100); expect(popup()).toBeNull();
  });
  it("a reply draft with the caret holds the hovered popover open and stays live", async () => {
    const pane = mount(800);
    hoverHighlight(mark(pane, "one")); await advance(1000);
    const draft = popup().querySelector<HTMLTextAreaElement>('[data-reply-root="one"]')!;
    draft.focus(); draft.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    draft.value = "Hovered reply"; draft.dispatchEvent(new Event("input", { bubbles: true }));
    leaveHighlight(mark(pane, "one")); popup().dispatchEvent(new MouseEvent("mouseleave"));
    await advance(1000); expect(popup()).not.toBeNull();
    draft.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true }));
    expect(node("one")!.children.map(id => node(id)!.raw.split("\n")[0])).toContain("Hovered reply");
    expect(popup().textContent).toContain("Hovered reply");
  });
  it.each(["focused composer", "unfocused draft", "live block editor"])("hovering another passage preserves the %s", async held => {
    const pane = mount(800);
    hoverHighlight(mark(pane, "one")); await advance(1000);
    const draft = popup().querySelector<HTMLTextAreaElement>('[data-reply-root="one"]')!;
    let editor: ReturnType<typeof editorIn> | undefined;
    if (held === "live block editor") {
      startEditing("one", 0); await Promise.resolve();
      editor = editorIn(popup()); editor.input("Editing the first thread", 24);
    } else {
      draft.focus();
      if (held === "unfocused draft") {
        draft.value = "Keep this reply"; draft.dispatchEvent(new Event("input", { bubbles: true })); draft.blur();
      }
    }
    const focused = document.activeElement;
    hoverHighlight(mark(pane, "two")); await advance(1000);
    expect(draft.isConnected, "the first passage's composer stays mounted").toBe(true);
    expect(popup().querySelector('[data-reply-root="one"]')).toBe(draft);
    expect(popup().textContent).not.toContain("Second thread");
    if (held === "unfocused draft") expect(draft.value).toBe("Keep this reply");
    else expect(document.activeElement).toBe(focused);
    if (editor) expect(editor.value()).toContain("Editing the first thread");
  });
  it("a mouse press during the dwell places the caret and the preview never opens", async () => {
    const pane = mount(800);
    const two = mark(pane, "two");
    hoverHighlight(two); await advance(400);
    const down = new (window.PointerEvent ?? MouseEvent)("pointerdown", { bubbles: true, cancelable: true });
    Object.defineProperty(down, "pointerType", { value: "mouse" });
    two.dispatchEvent(down);
    two.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    expect(editingId()).toBe("p");
    await advance(2000);
    expect(popup()).toBeNull();
  });
  it("phones and the wide margin do not open on hover", async () => {
    for (const width of [390, 1400]) {
      const pane = mount(width);
      hoverHighlight(mark(pane, "one")); await advance(2000);
      expect(popup()).toBeNull();
      dispose?.(); dispose = undefined; resetStore(); document.body.replaceChildren();
    }
  });
});

it.each([{ width: 800, display: "margin" }, { width: 1400, display: "popup" }] as const)("width $width ($display): a mouse click on a highlight places the caret and opens nothing", ({ width, display }) => {
  const pane = mount(width); if (display === "popup") chooseComments("popup");
  const mark = pane.querySelector<HTMLElement>('.comment-quote-anchor[data-comment-id="two"]')!;
  const down = new (window.PointerEvent ?? MouseEvent)("pointerdown", { bubbles: true, cancelable: true });
  Object.defineProperty(down, "pointerType", { value: "mouse" });
  mark.dispatchEvent(down);
  mark.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
  mark.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 0 }));
  mark.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
  expect(popup()).toBeNull();
  expect(editingId()).toBe("p");
});

// D-20261108-MC8: only a thread with no resolvable highlight keeps a marker.
it.each([
  { why: "changed", raw: "Second thread\ntine.quote:: words no longer here" },
  { why: "ambiguous", raw: "Second thread\ntine.quote:: passage" },
])("a $why quote gets the one small marker, and it opens that thread", ({ raw }) => {
  const pane = mount(800);
  expect(pane.querySelector(".margin-marker")).toBeNull();
  setRaw("two", raw, { timetracking: false });
  const marker = pane.querySelector<HTMLButtonElement>(".comment-unanchored-marker")!;
  expect(marker).not.toBeNull(); expect(marker.textContent).toBe("");
  expect(marker.getAttribute("aria-label")).toBe("Open the comment whose quoted text has no highlight");
  expect(pane.querySelectorAll(".margin-marker")).toHaveLength(1);
  marker.click();
  expect(popup().querySelector('[data-comment-group="two"]')!.closest("[hidden]")).toBeNull();
  persistCommentMarginHidden(true);
  expect(pane.querySelector(".margin-marker,.comment-quote-anchor")).toBeNull(); expect(popup()).toBeNull();
});

// Review round 2 (AR-1, UI-4): a quote that resolves in the source can still
// have no highlight, because the renderer marks only passages inside one plain
// text run. Marker eligibility reads what the renderer drew, so such a thread
// keeps an entry point, also while its block is being edited.
it.each([
  { why: "inside inline code", parent: "Some `codeword` text and an ordinary passage", quote: "codeword", width: 800, display: "margin" },
  { why: "across bold markup", parent: "Plain then **bold** words and an ordinary passage", quote: "then **bold**", width: 800, display: "margin" },
  { why: "inside inline code", parent: "Some `codeword` text and an ordinary passage", quote: "codeword", width: 1400, display: "popup" },
  { why: "across bold markup", parent: "Plain then **bold** words and an ordinary passage", quote: "then **bold**", width: 1400, display: "popup" },
] as const)("a quote that resolves $why but cannot be highlighted keeps the marker (width $width, $display)", async ({ parent, quote, width, display }) => {
  const pane = mount(width, "main", [b("p", parent, [b("code", `Code thread\ntine.quote:: ${quote}`), b("plain", "Plain thread\ntine.quote:: ordinary passage")])]);
  if (display === "popup") chooseComments("popup");
  expect(pane.querySelector('.comment-quote-anchor[data-comment-id="code"]'), "the renderer draws no highlight here").toBeNull();
  expect(pane.querySelector('.comment-quote-anchor[data-comment-id="plain"]'), "control: an ordinary passage is highlighted").not.toBeNull();
  const marker = () => pane.querySelector<HTMLButtonElement>(".comment-unanchored-marker");
  expect(marker(), "the unhighlighted thread keeps an entry point").not.toBeNull();
  expect(pane.querySelectorAll(".margin-marker")).toHaveLength(1);
  marker()!.click();
  expect(popup().querySelector('[data-comment-group="code"]')!.closest("[hidden]")).toBeNull();
  popup().querySelector<HTMLButtonElement>(".comment-popup-close")!.click();
  // Editing the block replaces its rendering; the marker keeps the last answer.
  startEditing("p", 0); await Promise.resolve();
  expect(editorIn(pane, "p")).toBeTruthy();
  expect(marker()).not.toBeNull();
  endEdit("blur"); await Promise.resolve();
  expect(marker()).not.toBeNull();
  // Once the passage becomes highlightable the marker goes.
  setRaw("p", "Some codeword text and an ordinary passage then bold words", { timetracking: false });
  setRaw("code", "Code thread\ntine.quote:: codeword", { timetracking: false });
  expect(pane.querySelector('.comment-quote-anchor[data-comment-id="code"]')).not.toBeNull();
  expect(marker()).toBeNull();
});

it("the phone chip is a tap target without a count and opens the bottom sheet", () => {
  const pane = mount(390);
  expect(chip(pane).textContent).toBe(""); expect(chip(pane).querySelector("svg")).not.toBeNull();
  chip(pane).click();
  expect(popup().classList.contains("comment-bottom-sheet")).toBe(true);
});

// GH #891 finding 2: grouped presentation and the arrow keys consume one
// surface order. Interleaved threads A (alpha), B (beta), C (alpha) are shown
// as [A, C] then [B]; the arrows walk the same order.
const INTERLEAVED = () => [
  b("p", "The alpha words, then the beta words", [
    b("A", "Thread A\ntine.quote:: alpha"), b("B", "Thread B\ntine.quote:: beta"), b("C", "Thread C\ntine.quote:: alpha"),
  ]),
  b("after", "After the threads"),
];
/** The visible thread roots of each grouped card, in presentation order. */
function groupsIn(root: Element) {
  return [...root.querySelectorAll<HTMLElement>(".comment-group")].filter(group => !group.closest("[hidden]"))
    .map(group => [...new Set([...group.querySelectorAll<HTMLElement>("[data-block-id]")].map(el => el.dataset.blockId!))].filter(id => /^[ABC]$/.test(id)));
}
/** Moves the caret to the edge of the edited block and presses the arrow, read
 * synchronously the way the margin arrow tests read it: one session walks the
 * whole order, so no intermediate save reloads the page under it. */
function arrow(keyName: "ArrowDown" | "ArrowUp", root: ParentNode = document) {
  const editor = editorIn(root, editingId()!);
  const at = keyName === "ArrowDown" ? editor.value().length : 0;
  editor.select(at, at); editor.keydown({ key: keyName });
  return editingId();
}
async function from(id: string) { startEditing(id, 0); await Promise.resolve(); }
it("the wide margin presents interleaved groups in passage order and the arrows follow it", async () => {
  const pane = mount(1400, "main", INTERLEAVED());
  expect(groupsIn(pane.querySelector(".margin-column")!)).toEqual([["A", "C"], ["B"]]);
  await from("A");
  expect(arrow("ArrowDown")).toBe("C");
  expect(arrow("ArrowDown")).toBe("B");
  expect(arrow("ArrowUp")).toBe("C");
  expect(arrow("ArrowUp")).toBe("A");
  expect(arrow("ArrowUp")).toBe("p");
  await from("B");
  expect(arrow("ArrowDown")).toBe("after");
});
it("the popover walks only the group it shows, in the same order", async () => {
  const pane = mount(800, "main", INTERLEAVED());
  openCommentThread(pane, "A");
  expect(groupsIn(popup())).toEqual([["A", "C"]]);
  await from("A");
  expect(arrow("ArrowDown", popup())).toBe("C");
  expect(arrow("ArrowUp", popup())).toBe("A");
  expect(arrow("ArrowDown", popup())).toBe("C");
  // Past the shown group the caret returns to the page, never to hidden B.
  expect(arrow("ArrowDown", popup())).toBe("after");
});
