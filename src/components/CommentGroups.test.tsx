import { afterAll, afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { openCommentThread } from "../tests/commentEntry";
import { render } from "solid-js/web";
import { Block, SurfaceContext } from "./Block";
import { blockRegions, initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { node, resetStore, setRaw } from "../document";
import { commentGroups } from "../commentGroups";
import { editorIn } from "../tests/editorDriver";
import { startEditing } from "../editorController";
import { backend } from "../backend";
import { installKeybindings } from "../keybindings";
import { editingId, endEdit } from "../editorController";
import { commentAndEdit } from "../commentActions";
import { bindTestHost } from "../document/host/wiring.test.support";
import { answerOpensFromDocument } from "../document/host/documentHost.test.support";
import { resetCommentAuthorNameForTests } from "../commentIdentity";
import { persistCommentMarginHidden } from "../commentMarginPref";
import type { BlockDto } from "../types";

let dispose: (() => void) | undefined;
let disposeKeys: (() => void) | undefined;
beforeAll(async () => { await initParser(); disposeKeys = installKeybindings(); });
afterAll(() => disposeKeys?.());
beforeEach(async () => { const host = await bindTestHost(); answerOpensFromDocument(host); vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => null)); resetCommentAuthorNameForTests("Ada"); persistCommentMarginHidden(false); });
afterEach(() => { endEdit("page-navigation"); dispose?.(); dispose = undefined; resetStore(); document.body.replaceChildren(); vi.restoreAllMocks(); });
function b(id: string, raw: string, children: BlockDto[] = [], collapsed = false): BlockDto {
  return { id, raw, children, collapsed, properties: blockRegions(raw, "md").properties.filter(p => p.primary).map(p => [p.key, p.value]) };
}
function mount(blocks: BlockDto[]) {
  loadSingle({ name: "Groups", title: "Groups", id: "/graph/pages/Groups.md", rev: "fixture", kind: "page", format: "md", pre_block: null, blocks });
  const el = document.createElement("div"); document.body.append(el);
  dispose = render(() => <Block id="p" />, el);
  openCommentThread(el);
  return document.body;
}
it("groups equal resolved ranges, quotes once, and keeps each thread's replies independent", () => {
  const el = mount([b("p", "some words", [b("a", "One\ntine.quote:: words", [b("r", "Reply")]), b("z", "Two\ntine.quote:: words")])]);
  const groups = el.querySelectorAll(".comment-group");
  expect(groups).toHaveLength(1);
  expect(groups[0].querySelectorAll(".comment-quote")).toHaveLength(1);
  expect(groups[0].querySelectorAll(".comment-reply-draft")).toHaveLength(2);
  const draft = groups[0].querySelector<HTMLTextAreaElement>('[data-reply-root="z"]')!;
  draft.value = "For second thread"; draft.dispatchEvent(new Event("input", { bubbles: true }));
  draft.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true }));
  expect(node("a")!.children).toEqual(["r"]);
  expect(node("z")!.children).toHaveLength(1);
});

it("deleting one grouped thread preserves its neighbor and that neighbor's replies", () => {
  const el = mount([b("p", "some words", [b("a", "One\ntine.quote:: words", [b("r", "First reply")]), b("z", "Two\ntine.quote:: words", [b("keep", "Second reply")])])]);
  const draft = el.querySelector<HTMLTextAreaElement>('[data-reply-root="z"]')!;
  draft.value = "Keep this unsent reply"; draft.dispatchEvent(new Event("input", { bubbles: true }));
  el.querySelector<HTMLButtonElement>('[data-block-id="a"] > .comment-header .comment-delete')!.click();
  expect(node("p")!.children).toEqual(["z"]);
  expect(node("z")!.children).toEqual(["keep"]);
  expect(el.querySelector('[data-block-id="keep"]')?.textContent).toContain("Second reply");
  expect(el.querySelectorAll(".comment-reply-draft")).toHaveLength(1);
  expect(el.querySelector<HTMLTextAreaElement>('[data-reply-root="z"]')!.value).toBe("Keep this unsent reply");
});
it("commenting again focuses the existing reply without writing; New thread explicitly appends a separate root", async () => {
  const el = mount([b("p", "some words", [b("a", "One\ntine.quote:: words")])]);
  expect(commentAndEdit("p", "some words", 5, 10)).toBe("a");
  expect(node("p")!.children).toEqual(["a"]);
  await Promise.resolve();
  expect(document.activeElement).toBe(el.querySelector(".comment-reply-draft"));
  el.querySelector<HTMLButtonElement>(".comment-new-thread")!.click();
  expect(node("p")!.children).toHaveLength(2);
  const created = node("p")!.children[1];
  expect(editingId()).toBe(created);
  expect(node(created)!.raw).toContain("tine.quote:: words");
  expect(node(created)!.raw).toContain("tine.quote-prefix:: some");
  expect(node(created)!.raw).toContain("tine.quote-suffix:: ");
  expect(node("a")!.raw).toBe("One\ntine.quote:: words");
});

it("New thread retains an unsent reply in the existing group", () => {
  const el = mount([b("p", "some words", [b("a", "One\ntine.quote:: words")])]);
  const draft = el.querySelector<HTMLTextAreaElement>(".comment-reply-draft")!;
  draft.focus(); draft.value = "Keep my unsent text"; draft.dispatchEvent(new Event("input", { bubbles: true }));
  el.querySelector<HTMLButtonElement>(".comment-new-thread")!.click();
  expect(el.querySelector('[data-reply-root="a"]')).toBe(draft);
  expect(draft.value).toBe("Keep my unsent text");
  expect(node("a")!.children).toEqual([]);
});

it("commenting from a secondary editor focuses that copy's reply box", async () => {
  loadSingle({ name: "Groups", title: "Groups", id: "/graph/pages/Groups.md", rev: "fixture", kind: "page", format: "md", pre_block: null, blocks: [b("p", "some words", [b("a", "One\ntine.quote:: words")])] });
  const el = document.createElement("div"); document.body.append(el);
  dispose = render(() => <><SurfaceContext.Provider value="main"><div data-copy="main"><Block id="p" /></div></SurfaceContext.Provider><SurfaceContext.Provider value="ref:copy"><div data-copy="secondary"><Block id="p" /></div></SurfaceContext.Provider></>, el);
  const copy = el.querySelector('[data-copy="secondary"]')!;
  startEditing("p", 5, null, "ref:copy");
  await Promise.resolve();
  const editor = editorIn(copy); editor.select(5, 10);
  editor.keydown({ key: "r", ctrlKey: true });
  await Promise.resolve();
  expect(node("p")!.children).toEqual(["a"]);
  expect(document.activeElement).toBe(document.querySelector(".comment-thread-popup .comment-reply-draft"));
});

it.each([false, true])("a Comment chord pre-cancelled by the native reload guard works (existing=%s)", async hasThread => {
  const el = mount([b("p", "some words", hasThread ? [b("a", "One\ntine.quote:: words")] : [])]);
  startEditing("p", 5);
  await Promise.resolve();
  const editor = editorIn(el); editor.select(5, 10);
  const event = new KeyboardEvent("keydown", { key: "r", code: "KeyR", ctrlKey: true, bubbles: true, cancelable: true });
  event.preventDefault();
  editor.keydown(event);
  await Promise.resolve();
  if (hasThread) {
    expect(node("p")!.children).toEqual(["a"]);
    expect(document.activeElement).toBe(el.querySelector(".comment-reply-draft"));
  } else {
    expect(node("p")!.children).toHaveLength(1);
    const created = node("p")!.children[0];
    expect(editingId()).toBe(created);
    expect(node(created)!.raw).toContain("tine.quote:: words");
  }
});

it("groups resolved ranges rather than quote strings and leaves uncertain anchors independent", () => {
  const el = mount([b("p", "same then same", [
    b("a", "First\ntine.quote:: same\ntine.quote-prefix:: \ntine.quote-suffix:: then"),
    b("z", "Last\ntine.quote:: same\ntine.quote-prefix:: then\ntine.quote-suffix:: "),
    b("ambiguous", "Uncertain\ntine.quote:: same"), b("also-ambiguous", "Uncertain too\ntine.quote:: same"),
    b("stale", "Changed\ntine.quote:: lost"), b("also-stale", "Changed too\ntine.quote:: lost"),
    b("whole", "Whole\ntine.quote:: "), b("also-whole", "Whole too\ntine.quote:: "),
  ])]);
  expect(commentGroups("p", "md")).toEqual([["ambiguous"], ["also-ambiguous"], ["stale"], ["also-stale"], ["whole", "also-whole"], ["a"], ["z"]]);
  // Review round 3 (PC-4): only the shown passage is mounted, so walk the
  // passages with the popup's own controls and ask each for New thread.
  const newThreadPerGroup = () => {
    const button = (cls: string) => el.querySelector<HTMLButtonElement>(`.comment-thread-popup ${cls}`)!;
    while (!button(".comment-prev-thread").disabled) button(".comment-prev-thread").click();
    const seen: boolean[] = [];
    for (;;) {
      expect(el.querySelectorAll(".comment-thread-popup .comment-group")).toHaveLength(1);
      seen.push(!!el.querySelector(".comment-thread-popup .comment-new-thread"));
      if (button(".comment-next-thread").disabled) return seen;
      button(".comment-next-thread").click();
    }
  };
  expect(newThreadPerGroup()).toEqual([false, false, false, false, true, true, true]);
  setRaw("p", "replacement", { timetracking: false });
  expect(commentGroups("p", "md").filter(ids => ids.includes("a") || ids.includes("z"))).toEqual([["a"], ["z"]]);
  expect(newThreadPerGroup().filter(Boolean)).toHaveLength(1);
});
it("collapsed subtree chip counts open descendant roots, excludes own threads and all replies, and expands", () => {
  const el = mount([b("p", "Parent", [b("own", "Own\ntine.quote:: Parent"), b("kid", "Child", [
    b("a", "One\ntine.quote:: Child", [b("r", "Reply", [b("deep", "Quoted reply\ntine.quote:: Reply")])]),
    b("z", "Two\ntine.quote:: Child"), b("resolved", "Closed\ntine.quote:: Child\ntine.resolved:: true"),
  ])], true)]);
  const chip = el.querySelector<HTMLButtonElement>(".comment-below-chip")!;
  expect(chip?.textContent).toBe("2 below");
  chip.click();
  expect(node("p")!.collapsed).toBe(false);
  expect(el.querySelector(".comment-below-chip")).toBeNull();
  expect(el.querySelector('[data-block-id="kid"]')).not.toBeNull();
});
