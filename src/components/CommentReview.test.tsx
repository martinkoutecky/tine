import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createRoot, For } from "solid-js";
import { CommentSurface } from "./CommentSurface";
import { Block } from "./Block";
import { initParser, blockRegions } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { node, pageByName, resetStore, setRaw } from "../document";
import { backend } from "../backend";
import { answerOpensFromDocument } from "../document/host/documentHost.test.support";
import { createCommentIndex } from "../commentIndex";
import { setDoc } from "../document/model";
import { bindTestHost } from "../document/host/wiring.test.support";
import { changeCommentsInPopup, commentMarginHidden, persistCommentMarginHidden } from "../commentMarginPref";
import { commandDefaults, paletteCommands, runGlobalCommand } from "../keybindings";
import { endEdit } from "../editorController";
import type { BlockDto } from "../types";

let dispose: (() => void) | undefined;
beforeAll(async () => { await initParser(); if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList; });
beforeEach(async () => { answerOpensFromDocument(await bindTestHost()); persistCommentMarginHidden(false); });
afterEach(() => { endEdit("page-navigation"); dispose?.(); resetStore(); changeCommentsInPopup(false); document.body.replaceChildren(); vi.restoreAllMocks(); });
const b = (id: string, raw: string, children: BlockDto[] = [], collapsed = false): BlockDto => ({ id, raw, children, collapsed,
  properties: blockRegions(raw, "md").properties.filter(p => p.primary).map(p => [p.key, p.value]) });
const stepPageComment = (step: number) => { expect(runGlobalCommand(step > 0 ? "comment/next" : "comment/previous")).toBe(true); };
function mount(width: number, popup: boolean, blocks?: BlockDto[]) {
  changeCommentsInPopup(popup);
  loadSingle({ name: "Review", title: "Review", id: "/graph/pages/Review.md", rev: "fixture", kind: "page", format: "md", pre_block: null, blocks: blocks ?? [
    b("p", "first then last", [b("last", "Last\ntine.quote:: last"), b("first", "First\ntine.quote:: first"),
      b("tie", "Tie\ntine.quote:: first"), b("closed", "Closed\ntine.quote:: then\ntine.resolved:: true")]),
    b("q", "another passage", [b("third", "Third\ntine.quote:: passage")]),
  ] });
  const pane = document.createElement("main"); pane.className = "main-content"; document.body.append(pane);
  Object.defineProperty(pane, "clientWidth", { value: width });
  dispose = render(() => <CommentSurface main page={pageByName("Review")}><For each={pageByName("Review")?.roots}>{id => <Block id={id} />}</For></CommentSurface>, pane);
  return pane;
}
const focused = () => (document.activeElement as HTMLElement)?.dataset.replyRoot;
it("counts backend requests and scrolls for first, later and collapsed-target steps", async () => {
  mount(1400, true, [b("p", "Parent", [b("q", "Child", [b("child", "Child thread\ntine.quote:: Child")]),
    b("own", "Own thread\ntine.quote:: Parent"), b("tie", "Second thread\ntine.quote:: Parent")], true)]);
  // Instrument every backend method, so an added read or write cannot evade
  // a hand-maintained list of the calls navigation currently makes.
  const api = backend() as unknown as Record<string, (...args: unknown[]) => unknown>;
  const spies = Object.keys(api).filter(key => key !== "graphBindingGeneration" && typeof api[key] === "function")
    .map(key => ({ key, spy: vi.spyOn(api, key) }));
  const scroll = vi.spyOn(HTMLElement.prototype, "scrollIntoView").mockImplementation(() => {});
  for (const [id, requests] of [["own", []], ["tie", []], ["child", ["pageOpen", "pageSubmit", "pageWait", "pageClose"]]] as const) {
    spies.forEach(({ spy }) => spy.mockClear()); scroll.mockClear();
    stepPageComment(1); await vi.waitFor(() => expect(focused()).toBe(id));
    if (id === "child") await vi.waitFor(() => expect(spies.find(({ key }) => key === "pageClose")!.spy).toHaveBeenCalledOnce());
    expect(spies.flatMap(({ key, spy }) => spy.mock.calls.map(() => key)).sort()).toEqual([...requests].sort());
    expect(scroll).toHaveBeenCalledTimes(1);
  }
});

it.each([[1400, false], [800, false], [1400, true], [390, false]] as const)("reveals the shared passage for either thread at width %s, popup %s", async (width, popup) => {
  const pane = mount(width, popup);
  const scroll = vi.spyOn(HTMLElement.prototype, "scrollIntoView").mockImplementation(() => {});
  for (const id of ["first", "tie"]) {
    stepPageComment(1);
    await vi.waitFor(() => expect(focused()).toBe(id));
    const revealed = scroll.mock.instances.at(-1) as unknown as HTMLElement;
    expect(revealed.textContent).toBe("first");
    expect(pane.contains(revealed)).toBe(true);
  }
});

it("reveals a resolved passage without a painted highlight", async () => {
  const pane = mount(1400, true);
  pane.style.overflowY = "auto";
  vi.spyOn(pane, "getBoundingClientRect").mockReturnValue({ top: 100, height: 600 } as DOMRect);
  vi.spyOn(Range.prototype, "getClientRects").mockImplementation(() => [{ top: 1100, height: 20 }] as unknown as DOMRectList);
  vi.spyOn(HTMLElement.prototype, "scrollIntoView").mockImplementation(() => {});
  pane.querySelector<HTMLButtonElement>(".comment-resolved-toggle")!.click();
  for (const id of ["first", "tie", "closed"]) {
    stepPageComment(1); await vi.waitFor(() => expect(focused()).toBe(id));
  }
  const passageTop = 1100 - pane.scrollTop;
  expect(passageTop).toBeGreaterThanOrEqual(100);
  expect(passageTop + 20).toBeLessThanOrEqual(700);
});

it.each([[800, false], [1400, true], [390, false]] as const)("popup controls follow focus within a passage at width %s, popup %s", async (width, popup) => {
  mount(width, popup);
  stepPageComment(1); await vi.waitFor(() => expect(focused()).toBe("first"));
  document.querySelector<HTMLElement>('[data-reply-root="tie"]')!.focus();
  const toolbar = document.querySelector(".comment-popup-toolbar")!;
  expect(toolbar.textContent).toContain("2 of 4");
  expect(toolbar.querySelector<HTMLButtonElement>(".comment-prev-thread")!.disabled).toBe(false);
  toolbar.querySelector<HTMLButtonElement>(".comment-next-thread")!.click();
  await vi.waitFor(() => expect(focused()).toBe("last"));
  document.querySelector<HTMLButtonElement>(".comment-prev-thread")!.click();
  await vi.waitFor(() => expect(focused()).toBe("tie"));
  document.querySelector<HTMLElement>('[data-reply-root="first"]')!.focus();
  expect(document.querySelector(".comment-popup-toolbar")!.textContent).toContain("1 of 4");
  expect(document.querySelector<HTMLButtonElement>(".comment-prev-thread")!.disabled).toBe(true);
});

it.each([[1400, false, ".margin-column"], [800, false, ".comment-thread-popup"], [1400, true, ".comment-thread-popup"], [390, false, ".comment-bottom-sheet"]] as const)("reviews in margin order at width %s, popup %s", async (width, popup, target) => {
  const pane = mount(width, popup);
  const scroll = vi.spyOn(HTMLElement.prototype, "scrollIntoView").mockImplementation(() => {});
  const drawn = () => [...pane.querySelectorAll<HTMLElement>(".margin-column .comment-card")].map(card => card.dataset.blockId);
  if (target === ".margin-column") expect(drawn()).toEqual(["first", "tie", "last", "third"]);
  for (const id of ["first", "tie", "last", "third"]) {
    stepPageComment(1);
    await vi.waitFor(() => expect(focused()).toBe(id));
    expect(document.activeElement?.closest(target)).not.toBeNull();
  }
  stepPageComment(1); expect(focused()).toBe("third");
  stepPageComment(-1); await vi.waitFor(() => expect(focused()).toBe("last"));
  expect(scroll).toHaveBeenCalledTimes(5);
  pane.querySelector<HTMLButtonElement>(".comment-resolved-toggle")!.click();
  if (target === ".margin-column") expect(drawn()).toEqual(["first", "tie", "closed", "last", "third"]);
  stepPageComment(-1); await vi.waitFor(() => expect(focused()).toBe("closed"));
  if (width < 1100 || popup) {
    document.querySelector<HTMLButtonElement>(".comment-next-thread")!.click();
    await vi.waitFor(() => expect(focused()).toBe("last"));
    document.querySelector<HTMLButtonElement>(".comment-next-thread")!.click();
    await vi.waitFor(() => expect(focused()).toBe("third"));
  }
});

it("offers localized review commands in the palette and shortcut settings without default collisions", () => {
  for (const [id, label] of [["comment/next", "Next comment"], ["comment/previous", "Previous comment"]]) {
    expect(paletteCommands().find(command => command.id === id)?.label).toBe(label);
    expect(commandDefaults().find(command => command.id === id)).toMatchObject({ label, binding: "" });
  }
});

it("a step shows comments that Hide comments had hidden", async () => {
  persistCommentMarginHidden(true);
  mount(1400, false);
  expect(commentMarginHidden()).toBe(true);
  stepPageComment(1);
  await vi.waitFor(() => expect(focused()).toBe("first"));
  expect(commentMarginHidden()).toBe(false);
});

it("reviews a parent's own thread before hidden descendant threads and reveals only their ancestor path", async () => {
  mount(1400, true, [b("p", "Parent", [b("q", "Child", [b("child", "Child thread\ntine.quote:: Child")]), b("own", "Own thread\ntine.quote:: Parent")], true),
    b("unrelated", "Unrelated", [b("untouched", "Hidden body")], true)]);
  stepPageComment(1); await vi.waitFor(() => expect(focused()).toBe("own"));
  expect(node("p")?.collapsed).toBe(true);
  stepPageComment(1); await vi.waitFor(() => expect(focused()).toBe("child"));
  expect(node("p")?.collapsed).toBe(false);
  expect(node("unrelated")?.collapsed).toBe(true);
});

it("keeps review inside the selected journal day", async () => {
  mount(1400, true);
  dispose?.();
  const pane = document.querySelector<HTMLElement>("main")!;
  dispose = render(() => <><section data-day="one"><CommentSurface main page={pageByName("Review")} roots={["p"]}><Block id="p" /></CommentSurface></section>
    <section data-day="two"><CommentSurface main page={pageByName("Review")} roots={["q"]}><Block id="q" /></CommentSurface></section></>, pane);
  stepPageComment(1); await vi.waitFor(() => expect(focused()).toBe("first"));
  pane.querySelector('[data-day="two"] .block-content')!.dispatchEvent(new Event("pointerdown", { bubbles: true }));
  stepPageComment(1); await vi.waitFor(() => expect(focused()).toBe("third"));
  stepPageComment(1); expect(focused()).toBe("third");
  stepPageComment(-1); expect(focused()).toBe("third");
});

it("review metadata stays current after the outline releases its shared comment view", () => {
  mount(1400, true);
  createRoot(stop => {
    const index = createCommentIndex(() => pageByName("Review")!.roots);
    let releaseOutline!: () => void, releaseReview!: () => void;
    createRoot(dispose => { releaseOutline = dispose; return index.childrenOf("p"); });
    const reviewed = createRoot(dispose => { releaseReview = dispose; return index.childrenOf("p"); });
    releaseOutline();
    setRaw("first", node("first")!.raw + "\ntine.resolved:: true", { timetracking: false });
    expect(reviewed().entries.find(entry => entry.id === "first")?.resolved).toBe(true);
    releaseReview();
    const reopenedView = createRoot(dispose => {
      const value = index.childrenOf("p")().entries.find(entry => entry.id === "first")?.resolved;
      dispose(); return value;
    });
    expect(reopenedView).toBe(true);
    stop();
  });
});

it.each([[1400, false, ".margin-column"], [1400, true, ".comment-thread-popup"], [390, false, ".comment-bottom-sheet"]] as const)("focuses a read-only thread card at width %s, popup %s", async (width, popup, target) => {
  mount(width, popup);
  setDoc("pages", 0, "readOnly", true);
  stepPageComment(1);
  await vi.waitFor(() => expect((document.activeElement as HTMLElement).dataset.blockId).toBe("first"));
  expect(document.activeElement?.closest(target)).not.toBeNull();
  expect(document.querySelector(".comment-reply-draft")).toBeNull();
});
