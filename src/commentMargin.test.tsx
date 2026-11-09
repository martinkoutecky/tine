// GH #845: the comment margin's visibility is a device preference that survives
// a restart, hides only the view (never the graph), and comes back whenever a
// comment is written or a link leads to one.
import { openCommentThread } from "./tests/commentEntry";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { blockRegions, initParser } from "./render/parse";
import { node as docNode, resetStore } from "./document";
import { loadSingle } from "./document/workingSet";
import type { BlockDto } from "./types";
import { endEdit } from "./editorController";
import { commentMarginHidden, forceCommentMarginShown } from "./commentMarginPref";
import { registerReplyDraft, revealCommentMargin, revealCommentMarginFor, setCommentMarginVisible, toggleCommentMargin } from "./commentMargin";
import { commentAndEdit, createComment } from "./commentActions";
import { setToasts, toasts } from "./toasts";
import { PageView } from "./components/Page";
import { paneRouter } from "./panes";
import { openPageAtBlock, resetTabsToJournals } from "./router";

const PAGE = "MarginToggle";
const KEY = "logseq-claude.comment-margin-hidden";

function block(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  const properties = blockRegions(raw, "md").properties.filter((p) => p.primary).map((p): [string, string] => [p.key, p.value]);
  return { id, raw, collapsed: false, children, ...(properties.length ? { properties } : {}) };
}
const load = () => loadSingle({
  name: PAGE, kind: "page", title: PAGE, pre_block: null,
  blocks: [block("p", "Discuss these words", [block("kid", "plain"), block("c", "My view\ntine.quote:: words", [block("r", "Earlier")])])],
});

beforeAll(async () => { await initParser(); });
beforeEach(() => { localStorage.clear(); forceCommentMarginShown(); setToasts([]); load(); });
afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  resetTabsToJournals();
  document.body.innerHTML = "";
  localStorage.clear();
  forceCommentMarginShown();
  vi.restoreAllMocks();
});
afterAll(() => localStorage.clear());

describe("persistence", () => {
  it("is shown by default, stores only the opt-out, and is read back after a restart", async () => {
    expect(commentMarginHidden()).toBe(false);
    expect(localStorage.getItem(KEY)).toBeNull();
    toggleCommentMargin();
    expect(localStorage.getItem(KEY)).toBe("1");
    vi.resetModules();
    const restarted = await import("./commentMarginPref");
    expect(restarted.commentMarginHidden()).toBe(true);
    localStorage.removeItem(KEY);
    vi.resetModules();
    expect((await import("./commentMarginPref")).commentMarginHidden()).toBe(false);
  });

  it("changes nothing when storage refuses, and says so", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("quota"); });
    expect(setCommentMarginVisible(false)).toBe(false);
    expect(commentMarginHidden()).toBe(false);
    expect(toasts().some((t) => t.kind === "error")).toBe(true);
  });

  it("never writes to the graph: the comment blocks are untouched while hidden", () => {
    const before = ["p", "kid", "c", "r"].map((id) => [id, docNode(id).raw, [...docNode(id).children]]);
    setCommentMarginVisible(false);
    expect(["p", "kid", "c", "r"].map((id) => [id, docNode(id).raw, [...docNode(id).children]])).toEqual(before);
  });
});

describe("a comment is never written into something invisible", () => {
  it("writing one shows the margin again, from every route (createComment is the one door)", () => {
    setCommentMarginVisible(false);
    const id = createComment("p", "Discuss these words", 8, 13);
    expect(id).not.toBeNull();
    expect(commentMarginHidden()).toBe(false);
    expect(localStorage.getItem(KEY)).toBeNull();
    setCommentMarginVisible(false);
    expect(commentAndEdit("p", "Discuss these words", 0, 7)).not.toBeNull();
    expect(commentMarginHidden()).toBe(false);
  });

  it("shows it even when storage refuses to remember", () => {
    setCommentMarginVisible(false);
    vi.spyOn(Storage.prototype, "removeItem").mockImplementation(() => { throw new Error("denied"); });
    revealCommentMargin();
    expect(commentMarginHidden()).toBe(false);
  });

  it("a link or search result that targets a comment or a reply in a thread shows it; an ordinary block does not", () => {
    setCommentMarginVisible(false);
    revealCommentMarginFor("kid");
    revealCommentMarginFor("p");
    expect(commentMarginHidden()).toBe(true);
    revealCommentMarginFor("r");
    expect(commentMarginHidden()).toBe(false);
    setCommentMarginVisible(false);
    revealCommentMarginFor("c");
    expect(commentMarginHidden()).toBe(false);
  });

  it("opening a search result or block link that lands on a comment shows it", async () => {
    setCommentMarginVisible(false);
    openPageAtBlock({ name: PAGE, pageKind: "page", block: "kid" });
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(commentMarginHidden()).toBe(true);
    openPageAtBlock({ name: PAGE, pageKind: "page", block: "r" });
    await vi.waitFor(() => expect(commentMarginHidden()).toBe(false), { timeout: 2000 });
  });

  it("zooming into a comment shows it", async () => {
    setCommentMarginVisible(false);
    paneRouter("main").replaceActiveRoute({ kind: "page", name: PAGE, pageKind: "page", block: "c" });
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <PageView />, root);
    try {
      await vi.waitFor(() => expect(root.querySelector(".zoomed-block")).not.toBeNull());
      expect(commentMarginHidden()).toBe(false);
    } finally { dispose(); }
  });
});

describe("hiding waits for work in the thread", () => {
  it("refuses, with a toast, while a reply draft holds text, and hides once it is empty", () => {
    let draft = "half a reply";
    const unregister = registerReplyDraft(() => draft);
    expect(setCommentMarginVisible(false)).toBe(false);
    expect(commentMarginHidden()).toBe(false);
    expect(toasts().length).toBe(1);
    draft = "  ";
    expect(setCommentMarginVisible(false)).toBe(true);
    expect(commentMarginHidden()).toBe(true);
    unregister();
  });

  it("a real composer registers its draft", async () => {
    paneRouter("main").replaceActiveRoute({ kind: "page", name: PAGE, pageKind: "page", block: "p" });
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <PageView />, root);
    try {
      const area = await vi.waitFor(() => {
        if (!document.querySelector(".comment-thread-popup")) openCommentThread(root);
        const el = document.querySelector<HTMLTextAreaElement>('[data-block-id="c"] .comment-composer textarea');
        expect(el).not.toBeNull();
        return el!;
      });
      area.value = "unsent";
      area.dispatchEvent(new InputEvent("input", { bubbles: true }));
      expect(setCommentMarginVisible(false)).toBe(false);
      expect(document.querySelector('[data-block-id="c"]')).not.toBeNull();
      area.value = "";
      area.dispatchEvent(new InputEvent("input", { bubbles: true }));
      expect(setCommentMarginVisible(false)).toBe(true);
    } finally { dispose(); }
  });
});
