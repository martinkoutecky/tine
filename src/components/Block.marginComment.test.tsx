import { maybeEditorIn, type EditorDriver } from "../tests/editorDriver";
// Margin dialogue, slice 1 (vision §3.7): Ctrl/Cmd+R in the block editor
// creates a child comment that quotes the selection, and comment blocks render
// as a card with a quote header, an author chip and a marked passage in the
// parent. The comment is an ordinary child block written through the document
// door, so undo removes it like any other insert.
import { openCommentThread, showThread } from "../tests/commentEntry";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { editingId, endEdit, startEditing } from "../editorController";
import { blockRegions, initParser, editBlock } from "../render/parse";
import { node as docNode, resetStore, setRaw, undo } from "../document";
import { loadSingle } from "../document/workingSet";
import { pageByName } from "../document/model";
import type { BlockDto, PageDto } from "../types";
import { installKeybindings } from "../keybindings";
import { commentOnPaletteSelection, rememberEditorSelectionForPalette, submitReply } from "../commentActions";
import { invalidateBinding } from "../binding";
import { setToasts, toasts } from "../toasts";
import { Block } from "./Block";
import { PageView } from "./Page";
import { paneRouter } from "../panes";
import { resetTabsToJournals } from "../router";

const PAGE = "MarginComments";

// As the backend ships them: header facets (here the properties) ride on the DTO.
function block(id: string, raw: string, children: BlockDto[] = [], format: "md" | "org" = "md"): BlockDto {
  const properties = blockRegions(raw, format).properties.filter((p) => p.primary).map((p): [string, string] => [p.key, p.value]);
  return { id, raw, collapsed: false, children, ...(properties.length ? { properties } : {}) };
}

function page(blocks: BlockDto[]): PageDto {
  return { name: PAGE, kind: "page", title: PAGE, pre_block: null, blocks };
}

function mount(blocks: BlockDto[], edit?: string, format: "md" | "org" = "md"): { root: HTMLElement; dispose: () => void } {
  loadSingle({ ...page(blocks), format });
  if (edit) startEditing(edit, 0);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render((): JSX.Element => (
    <For each={pageByName(PAGE)?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ), root);
  // Existing card assertions now exercise the live popover reached by its chip.
  if (!edit) openCommentThread(root);
  return { root: document.body, dispose };
}

function ctrlR(ta: EditorDriver | HTMLTextAreaElement): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { key: "r", ctrlKey: true, bubbles: true, cancelable: true });
  if ("kind" in ta) ta.keydown(event); else ta.dispatchEvent(event);
  return event;
}

const editor = (root: HTMLElement) => maybeEditorIn(root) as EditorDriver;

let disposeKeys: (() => void) | null = null;
beforeAll(async () => {
  await initParser();
  disposeKeys = installKeybindings();
});
afterAll(() => disposeKeys?.());
afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  resetTabsToJournals();
  document.body.innerHTML = "";
});

describe("Ctrl/Cmd+R creates a margin comment", () => {
  it("keeps an ambiguous or missing quote's thread visible without guessing a highlight", () => {
    const { root, dispose } = mount([block("p", "word repeated word", [
      block("c", "Ambiguous response\ntine.quote:: word"),
      block("s", "Missing response\ntine.quote:: vanished"),
    ])]);
    try {
      // Separate passages: only the shown one is mounted (review round 3).
      expect(showThread("c")?.querySelector(".comment-quote")?.textContent).toContain("ambiguous");
      expect(root.textContent).toContain("Ambiguous response");
      expect(showThread("s")?.querySelector(".comment-quote")?.textContent).toContain("quoted text changed");
      expect(root.textContent).toContain("Missing response");
      expect(root.querySelector(".comment-quote-anchor")).toBeNull();
    } finally { dispose(); }
  });

  it("preserves hand-nested response order and writes a reply to the outer root", () => {
    const { root, dispose } = mount([block("p", "words", [
      block("c", "Root\ntine.quote:: words", [
        { ...block("a", "First", [block("deep", "Deep\ntine.quote:: First")]), collapsed: true },
        block("b", "Last"),
      ]),
    ])]);
    try {
      const raw = docNode("deep")!.raw;
      const reply = submitReply("deep", "New response")!;
      expect(docNode(reply)!.parent).toBe("c");
      expect(docNode("a")!.children).toEqual(["deep"]);
      expect(docNode("deep")!.raw).toBe(raw);
      expect([...root.querySelectorAll(".comment-header")].map(el => el.closest<HTMLElement>(".ls-block")!.dataset.blockId))
        .toEqual(["c", "a", "deep", "b", reply]);
    } finally { dispose(); }
  });
  it.each(["p", "ancestor"])("keeps the margin thread inline and replyable when zoomed into %s", async (zoomRoot) => {
    loadSingle(page([block("ancestor", "Parent", [block("p", "Discuss these words", [
      block("c", "My view\ntine.quote:: words", [block("r", "Earlier reply")]),
    ])])]));
    paneRouter("main").replaceActiveRoute({ kind: "page", name: PAGE, pageKind: "page", block: zoomRoot });
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <PageView />, root);
    try {
      const zoom = await vi.waitFor(() => {
        const element = root.querySelector(".zoomed-block");
        expect(element).not.toBeNull();
        return element!;
      });
      expect(root.querySelector(".margin-column")).toBeNull();
      openCommentThread(zoom);
      const card = document.querySelector('[data-block-id="c"]')!;
      expect(card.textContent).toContain("My view");
      expect(card.textContent).toContain("Earlier reply");
      const draft = card.querySelector<HTMLTextAreaElement>(".comment-composer textarea")!;
      expect(draft).not.toBeNull();
      draft.value = "Reply from zoom";
      draft.dispatchEvent(new InputEvent("input", { bubbles: true }));
      draft.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true }));
      const reply = docNode(docNode("c")!.children.at(-1)!)!;
      expect(reply.parent).toBe("c");
      expect(reply.raw).toContain("Reply from zoom");
      expect(document.body.textContent).toContain("Reply from zoom");
    } finally { dispose(); }
  });

  it("identifies each response separately from its quote and body, including your own nested replies (GH #110)", () => {
    const { root, dispose } = mount([block("p", "words", [block("c", "My view\ntine.quote:: words", [block("r", "Evidence\ntine.author:: Ada", [block("own", "Agreed")])])])]);
    try {
      const card = root.querySelector('[data-block-id="c"]')!;
      expect(card.querySelector(":scope > .comment-quote")?.textContent).toBe("words");
      for (const [id, author, body] of [["c", "you", "My view"], ["r", "Ada", "Evidence"], ["own", "you", "Agreed"]]) {
        const response = root.querySelector(`[data-block-id="${id}"]`)!;
        const own = (selector: string) => [...response.querySelectorAll(selector)].find(el => el.closest(".ls-block") === response);
        expect(own(".author-chip")?.textContent).toBe(author);
        expect(own(".comment-delete")).toBeDefined();
        expect(response.querySelector(":scope > .block-main .block-content")?.textContent).toBe(body);
        expect(response.querySelector(":scope > .block-main .author-chip")).toBeNull();
      }
    } finally { dispose(); }
  });
  it.each(["md", "org"] as const)("hides ordinary %s author but keeps quote-suffix editable", (format) => {
    const raw = format === "md"
      ? "The obstacle is the way.\ntine.author:: Marcus Aurelius\ntine.quote-suffix:: after"
      : "The obstacle is the way.\n:PROPERTIES:\n:tine.author: Marcus Aurelius\n:tine.quote-suffix: after\n:END:";
    const { root, dispose } = mount([block("p", "Literature", [block("ordinary", raw)])], "ordinary", format);
    try {
      expect(editor(root).value()).toBe(raw.replace(format === "md" ? "\ntine.author:: Marcus Aurelius" : "\n:tine.author: Marcus Aurelius", ""));
    } finally { dispose(); }
  });

  it.each(["md", "org"] as const)("lets handwritten %s quote metadata finish before hiding it on the next edit session", (format) => {
    const metadata = format === "md" ? "\ntine.quote:: selected words" : "\n:PROPERTIES:\n:tine.quote: selected words\n:END:";
    const { root, dispose } = mount([block("p", "selected words", [block("c", "My view")])], "c", format);
    try {
      const ta = editor(root);
      for (const character of metadata) {
        ta.input(ta.value() + character, undefined, { inputType: "insertText", data: character });
      }
      endEdit("page-navigation");
      expect(docNode("c")!.raw).toBe(`My view${metadata}`);
      startEditing("c", 0);
      expect(editor(root).value()).toBe("My view");
    } finally { dispose(); }
  });

  it("offers Comment in the selection toolbar and uses the selected words", () => {
    const { root, dispose } = mount([block("p", "Discuss these words here", [block("existing", "An existing child")])], "p");
    try {
      const ta = editor(root);
      ta.select(8, 19);
      ta.reportSelection(new Event("select", { bubbles: true }));
      const button = root.querySelector<HTMLButtonElement>('[data-selection-action="comment"]');
      expect(button, "the selection toolbar needs Comment").not.toBeNull();
      expect(button!.querySelector("svg")).not.toBeNull();
      expect(button!.closest(".sel-toolbar")!.querySelector(":scope > button:last-of-type")).toBe(button);
      button!.click();
      const id = docNode("p")!.children[1];
      expect(docNode(id)!.raw).toContain("tine.quote:: these words");
      expect(editingId()).toBe(id);
      expect(docNode("p")!.children).toEqual(["existing", id]); // ADR 0075 file order
      expect([...root.querySelectorAll('[data-block-id="p"] .ls-block')].map(el => el.getAttribute("data-block-id"))).toEqual(["existing"]);
      expect(root.querySelector(`.comment-thread-popup [data-block-id="${id}"]`)).not.toBeNull();
    } finally { dispose(); }
  });

  it.each(["md", "org"] as const)("hides %s comment metadata when editing and preserves every hidden line when typing", (format) => {
    const hidden = format === "md"
      ? "tine.quote:: these words  \ntine.quote-prefix:: Discuss\ntine.quote-suffix:: here\ntine.author:: claude\ncollapsed:: false"
      : ":PROPERTIES:\n:tine.quote: these words  \n:tine.quote-prefix: Discuss\n:tine.quote-suffix: here\n:tine.author: claude\n:collapsed: false\n:END:";
    const { root, dispose } = mount([block("p", "Discuss these words here", [block("c", `My view\n${hidden}`)])], "c", format);
    try {
      const ta = editor(root);
      expect(ta.value()).toBe("My view");
      ta.input("My revised view", undefined, { inputType: "insertText" });
      endEdit("page-navigation");
      expect(docNode("c")!.raw).toBe(`My revised view\n${hidden}`);
    } finally { dispose(); }
  });

  it("offers Reply without a selection and Delete removes a subtree as one undo step", () => {
    const { root, dispose } = mount([block("p", "Discuss these words", [block("c", "My view\ntine.quote:: words", [block("r", "Evidence", [block("nested", "More")])])])]);
    try {
      const row = () => root.querySelector('[data-block-id="c"]')!;
      const draft = row().querySelector<HTMLTextAreaElement>(":scope > .comment-composer textarea");
      expect(draft, "a comment needs a reply draft").not.toBeNull();
      draft!.focus();
      expect(docNode("c")!.children).toEqual(["r"]);
      draft!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      expect(docNode("c")!.children).toEqual(["r"]);
      draft!.value = "A considered reply";
      draft!.dispatchEvent(new InputEvent("input", { bubbles: true }));
      draft!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true }));
      const id = docNode("c")!.children.at(-1)!;
      expect(docNode(id)!.raw).toContain("A considered reply");
      expect(docNode(id)!.parent).toBe("c");
      endEdit("page-navigation");
      const del = row().querySelector<HTMLButtonElement>(":scope > .comment-header .comment-delete");
      expect(del, "a comment needs Delete").not.toBeNull();
      del!.click();
      expect(docNode("p")!.children).toHaveLength(0);
      expect(docNode("r")).toBeUndefined();
      undo();
      expect(docNode("p")!.children).toEqual(["c"]);
      expect(docNode("r")!.children).toEqual(["nested"]);
    } finally { dispose(); }
  });

  it.each(["md", "org"] as const)("Ctrl+R in a %s reply draft quotes its selection without reloading or losing its body", format => {
    const quote = format === "md" ? "tine.quote:: words" : ":PROPERTIES:\n:tine.quote: words\n:END:";
    const { root, dispose } = mount([block("p", "words", [block("c", `My view\n${quote}`, [], format)])], undefined, format);
    try {
      const draft = root.querySelector<HTMLTextAreaElement>(".comment-composer textarea")!;
      draft.focus();
      expect(ctrlR(draft).defaultPrevented).toBe(true);
      expect(docNode("c")!.children).toHaveLength(0);
      draft.value = "This deserves thought";
      draft.dispatchEvent(new InputEvent("input", { bubbles: true }));
      draft.setSelectionRange(5, 13);
      expect(ctrlR(draft).defaultPrevented).toBe(true);
      const reply = docNode(docNode("c")!.children[0])!;
      expect(editBlock(reply.raw, format, { kind: "property", key: "tine.date", value: null })).toBe("This deserves thought");
      const child = docNode(docNode("c")!.children[1])!;
      expect(reply.children).toEqual([]);
      expect(child.parent).toBe("c");
      expect(blockRegions(child.raw, format).properties.find(p => p.key === "tine.quote")?.value).toBe("deserves");
      expect(editingId()).toBe(child.id);
      endEdit("page-navigation");
      undo();
      expect(docNode("c")!.children).toHaveLength(0);
    } finally { dispose(); }
  });

  it("quoted replies stay inside their thread with a single quoted passage and readable body", () => {
    const { root, dispose } = mount([block("p", "words", [block("c", "Discuss this claim\ntine.quote:: words", [block("r", "My view\ntine.quote:: this claim", [block("n", "Evidence")])])])], "r");
    try {
      const ta = editor(root);
      ta.select(0, 2);
      ctrlR(ta);
      const id = docNode("c")!.children.at(-1)!;
      expect(docNode(id)!.parent).toBe("c");
      expect(docNode("r")!.children).toEqual(["n"]);
      expect(docNode(id)!.raw).toContain("tine.quote:: My");
      expect(root.querySelectorAll(".comment-card")).toHaveLength(1);
      expect(root.querySelector('[data-block-id="r"] > .comment-quote .comment-quote-text')?.textContent).toBe("this claim");
      expect(root.querySelector('[data-block-id="r"] > .block-main')?.textContent).toContain("My view");
    } finally { dispose(); }
  });

  it("appends a child after the existing children with tine.quote:: and puts the caret in its empty body", async () => {
    const { root, dispose } = mount([
      block("p", "An agent wrote a phrase worth disputing here.", [block("old", "an earlier child")]),
    ], "p");
    try {
      const ta = editor(root);
      const at = ta.value().indexOf("a phrase");
      ta.select(at, at + "a phrase worth disputing".length);
      const event = ctrlR(ta);

      expect(event.defaultPrevented).toBe(true); // never reaches the webview's reload
      const children = docNode("p")!.children;
      expect(children).toHaveLength(2);
      expect(children[0]).toBe("old");
      const comment = children[1];
      expect(editBlock(docNode(comment)!.raw, "md", { kind: "property", key: "tine.date", value: null })).toBe("\ntine.quote:: a phrase worth disputing\ntine.quote-prefix:: An agent wrote\ntine.quote-suffix:: here.");
      expect(docNode("p")!.raw).toBe("An agent wrote a phrase worth disputing here.");
      expect(editingId()).toBe(comment);
      await Promise.resolve();
      const commentEditor = editor(root);
      expect(commentEditor.element.closest<HTMLElement>(".ls-block")!.dataset.blockId).toBe(comment);
      expect(commentEditor.selection()[0]).toBe(0);
      expect(commentEditor.selection()[1]).toBe(0);
    } finally {
      dispose();
    }
  });

  it("writes prefix and suffix when the selected phrase repeats in the block", () => {
    const { root, dispose } = mount([block("p", "the cat sat; then the cat ran")], "p");
    try {
      const ta = editor(root);
      const second = ta.value().lastIndexOf("the cat");
      ta.select(second, second + 7);
      ctrlR(ta);
      const comment = docNode("p")!.children[0];
      expect(editBlock(docNode(comment)!.raw, "md", { kind: "property", key: "tine.date", value: null })).toBe("\ntine.quote:: the cat\ntine.quote-prefix:: the cat sat; then\ntine.quote-suffix:: ran");
    } finally {
      dispose();
    }
  });

  it("is one undo step: undo removes the comment and leaves the parent as it was", () => {
    const { root, dispose } = mount([block("p", "Some text to discuss")], "p");
    try {
      const ta = editor(root);
      ta.select(5, 9);
      ctrlR(ta);
      expect(docNode("p")!.children).toHaveLength(1);
      endEdit("page-navigation");
      undo();
      expect(docNode("p")!.children).toHaveLength(0);
      expect(docNode("p")!.raw).toBe("Some text to discuss");
    } finally {
      dispose();
    }
  });

  it("with no selection comments on the whole block (an empty tine.quote::)", () => {
    const { root, dispose } = mount([block("p", "Whole-block remark target")], "p");
    try {
      const ta = editor(root);
      ta.select(3, 3);
      const event = ctrlR(ta);
      expect(event.defaultPrevented).toBe(true);
      const comment = docNode("p")!.children[0];
      expect(editBlock(docNode(comment)!.raw, "md", { kind: "property", key: "tine.date", value: null })).toBe("\ntine.quote:: ");
      expect(editingId()).toBe(comment);
    } finally {
      dispose();
    }
  });

  it("the palette command uses the selection remembered as the palette opened", () => {
    const { root, dispose } = mount([block("p", "Palette path for a phrase")], "p");
    try {
      const ta = editor(root);
      ta.element.focus();
      const at = ta.value().indexOf("a phrase");
      ta.select(at, at + 8);
      rememberEditorSelectionForPalette();
      endEdit("page-navigation"); // opening the palette blurs the editor
      commentOnPaletteSelection();
      const comment = docNode("p")!.children[0];
      expect(editBlock(docNode(comment)!.raw, "md", { kind: "property", key: "tine.date", value: null })).toBe("\ntine.quote:: a phrase\ntine.quote-prefix:: Palette path for\ntine.quote-suffix:: ");
    } finally {
      dispose();
    }
  });

  it("the palette command refuses a selection whose graph was switched away (I-20)", () => {
    const { root, dispose } = mount([block("p", "Palette path for a phrase")], "p");
    try {
      setToasts([]);
      const ta = editor(root);
      ta.element.focus();
      ta.select(0, 7);
      rememberEditorSelectionForPalette();
      endEdit("page-navigation");
      invalidateBinding();
      commentOnPaletteSelection();
      expect(docNode("p")?.children ?? []).toHaveLength(0);
      expect(toasts().map((t) => t.message).join("\n")).toContain("graph that is no longer open");
    } finally {
      dispose();
    }
  });
});

describe("comment rendering", () => {
  const AGENT_TEXT = "The model converges because the step size shrinks, and the step size shrinks geometrically.";

  function renderedFixture() {
    return mount([
      block("agent", `${AGENT_TEXT}\ntine.author:: claude`, [
        block("c1", "I doubt this.\ntine.quote:: converges because", [
          block("r1", "It does, see the lemma.\ntine.author:: claude", [block("r2", "Fair enough.")]),
        ]),
        block("c2", "Which one?\ntine.quote:: the step size shrinks\ntine.quote-prefix:: the step size shrinks, and\ntine.quote-suffix:: geometrically."),
        block("c3", "Was rewritten.\ntine.quote:: a sentence that is gone"),
        block("plain", "An ordinary child with status:: open"),
      ]),
    ]);
  }

  const row = (root: HTMLElement, id: string) => root.querySelector(`.ls-block[data-block-id="${id}"]`) as HTMLElement;
  const ownChip = (el: HTMLElement) => el.querySelector(":scope > .comment-header .author-chip, :scope > .block-main .author-chip")?.textContent ?? null;

  it("shows a comment as a card with its quote header and hides the quote and author rows", () => {
    const { root, dispose } = renderedFixture();
    try {
      const c1 = row(root, "c1");
      expect(c1.classList.contains("comment-card")).toBe(true);
      expect(c1.querySelector(":scope > .comment-quote .comment-quote-text")?.textContent).toBe("converges because");
      expect(ownChip(c1)).toBe("you");
      const keys = [...root.querySelectorAll(".block-property-key")].map((k) => k.textContent);
      expect(keys).not.toContain("tine.quote");
      expect(keys).not.toContain("tine.quote-prefix");
      expect(keys).not.toContain("tine.author");
    } finally {
      dispose();
    }
  });

  it("marks an agent's block and chips each thread reply with its author", () => {
    const { root, dispose } = renderedFixture();
    try {
      const agent = row(root, "agent");
      expect(agent.classList.contains("authored")).toBe(true);
      expect(ownChip(agent)).toBe("claude");
      expect(ownChip(row(root, "r1"))).toBe("claude");
      expect(row(root, "r1").classList.contains("comment-in-thread")).toBe(true);
      expect(ownChip(row(root, "r2"))).toBe("you");
    } finally {
      dispose();
    }
  });

  it("marks the quoted passage in the parent, and the meant occurrence of a repeated phrase", () => {
    const { root, dispose } = renderedFixture();
    try {
      const marks = [...row(root, "agent").querySelectorAll(":scope > .block-main .comment-quote-anchor")];
      expect(marks.map((m) => m.textContent)).toEqual(["converges because", "the step size shrinks"]);
      // The repeated phrase is marked where its stored context says: the second one.
      expect(marks[1].previousSibling?.textContent?.endsWith(", and ")).toBe(true);
      expect(marks[1].nextSibling?.textContent).toBe(" geometrically.");
    } finally {
      dispose();
    }
  });

  it("re-marks the parent live when a comment's quote changes or the parent text changes", async () => {
    const { root, dispose } = renderedFixture();
    try {
      const marks = () => [...row(root, "agent").querySelectorAll(":scope > .block-main .comment-quote-anchor")].map((m) => m.textContent);
      setRaw("c1", "I doubt this.\ntine.quote:: geometrically");
      await Promise.resolve();
      expect(marks()).toEqual(["the step size shrinks", "geometrically"]);
      setRaw("agent", "Rewritten without the phrases.\ntine.author:: claude");
      await Promise.resolve();
      expect(marks()).toEqual([]);
      expect(row(root, "c1").querySelector(":scope > .comment-quote")!.classList.contains("stale")).toBe(true);
    } finally {
      dispose();
    }
  });

  it("keeps a comment whose quote no longer occurs, struck through and labelled", () => {
    const { root, dispose } = renderedFixture();
    try {
      showThread("c3"); // its own passage: mounted when shown (review round 3)
      const header = row(root, "c3").querySelector(":scope > .comment-quote") as HTMLElement;
      expect(header.classList.contains("stale")).toBe(true);
      expect(header.querySelector("s")?.textContent).toBe("a sentence that is gone");
      expect(header.textContent).toContain("quoted text changed");
      expect(row(root, "c3").textContent).toContain("Was rewritten.");
    } finally {
      dispose();
    }
  });

  it("renders an unmarked block exactly as before: no card, no chip, no header, its rows intact", () => {
    const { root, dispose } = renderedFixture();
    try {
      const plain = row(root, "plain");
      expect(plain.className).toBe("ls-block");
      expect(plain.querySelector(".author-chip, .comment-quote, .comment-quote-anchor")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("leaves a top-level tine.quote:: block alone: it is not a comment", () => {
    const { root, dispose } = mount([block("top", "Top\ntine.quote:: not a comment")]);
    try {
      expect(row(root, "top").querySelector(".comment-quote")).toBeNull();
      expect(row(root, "top").classList.contains("comment-card")).toBe(false);
    } finally {
      dispose();
    }
  });

  // Discussion #825 (D-20261102-PNS): before the rename, a user's own `quote::`
  // on a child bullet rendered as a margin-comment card, and any `author::` row
  // was replaced by an author chip. The bare keys are the user's again.
  it("a user's own quote:: and author:: on book notes stay ordinary property rows", () => {
    const { root, dispose } = mount([
      block("book", "Meditations\nauthor:: Marcus Aurelius", [
        block("note", "Favourite line\nquote:: The obstacle is the way.\nauthor:: Marcus Aurelius"),
      ]),
    ]);
    try {
      for (const id of ["book", "note"]) {
        const el = row(root, id);
        expect(el.classList.contains("comment-card"), id).toBe(false);
        expect(el.classList.contains("authored"), id).toBe(false);
        expect(el.querySelector(":scope > .comment-quote, :scope > .comment-header .author-chip, :scope > .block-main .author-chip"), id).toBeNull();
      }
      const keys = (id: string) => [...row(root, id).querySelectorAll(":scope > .block-main .block-property-key")].map((k) => k.textContent);
      expect(keys("book")).toContain("author");
      expect(keys("note")).toEqual(expect.arrayContaining(["quote", "author"]));
      expect(row(root, "book").querySelector(":scope > .block-main .comment-quote-anchor")).toBeNull();
    } finally {
      dispose();
    }
  });
});
