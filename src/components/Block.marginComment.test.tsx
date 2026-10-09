// Margin dialogue, slice 1 (vision §3.7): Ctrl/Cmd+R in the block editor
// creates a child comment that quotes the selection, and comment blocks render
// as a card with a quote header, an author chip and a marked passage in the
// parent. The comment is an ordinary child block written through the document
// door, so undo removes it like any other insert.
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { editingId, endEdit, startEditing } from "../editorController";
import { blockRegions, initParser } from "../render/parse";
import { node as docNode, resetStore, setRaw, undo } from "../document";
import { loadSingle } from "../document/workingSet";
import { pageByName } from "../document/model";
import type { BlockDto, PageDto } from "../types";
import { installKeybindings } from "../keybindings";
import { commentOnPaletteSelection, rememberEditorSelectionForPalette } from "../commentActions";
import { invalidateBinding } from "../binding";
import { setToasts, toasts } from "../toasts";
import { Block } from "./Block";

const PAGE = "MarginComments";

// As the backend ships them: header facets (here the properties) ride on the DTO.
function block(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  const properties = blockRegions(raw, "md").properties.filter((p) => p.primary).map((p): [string, string] => [p.key, p.value]);
  return { id, raw, collapsed: false, children, ...(properties.length ? { properties } : {}) };
}

function page(blocks: BlockDto[]): PageDto {
  return { name: PAGE, kind: "page", title: PAGE, pre_block: null, blocks };
}

function mount(blocks: BlockDto[], edit?: string): { root: HTMLDivElement; dispose: () => void } {
  loadSingle(page(blocks));
  if (edit) startEditing(edit, 0);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render((): JSX.Element => (
    <For each={pageByName(PAGE)?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ), root);
  return { root, dispose };
}

function ctrlR(ta: HTMLTextAreaElement): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { key: "r", ctrlKey: true, bubbles: true, cancelable: true });
  ta.dispatchEvent(event);
  return event;
}

const editor = (root: HTMLElement) => root.querySelector("textarea.block-editor") as HTMLTextAreaElement;

let disposeKeys: (() => void) | null = null;
beforeAll(async () => {
  await initParser();
  disposeKeys = installKeybindings();
});
afterAll(() => disposeKeys?.());
afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
});

describe("Ctrl/Cmd+R creates a margin comment", () => {
  it("appends a child after the existing children with quote:: and puts the caret in its empty body", async () => {
    const { root, dispose } = mount([
      block("p", "An agent wrote a phrase worth disputing here.", [block("old", "an earlier child")]),
    ], "p");
    try {
      const ta = editor(root);
      const at = ta.value.indexOf("a phrase");
      ta.setSelectionRange(at, at + "a phrase worth disputing".length);
      const event = ctrlR(ta);

      expect(event.defaultPrevented).toBe(true); // never reaches the webview's reload
      const children = docNode("p")!.children;
      expect(children).toHaveLength(2);
      expect(children[0]).toBe("old");
      const comment = children[1];
      expect(docNode(comment)!.raw).toBe("\nquote:: a phrase worth disputing");
      expect(docNode("p")!.raw).toBe("An agent wrote a phrase worth disputing here.");
      expect(editingId()).toBe(comment);
      await Promise.resolve();
      const commentEditor = editor(root);
      expect(commentEditor.closest<HTMLElement>(".ls-block")!.dataset.blockId).toBe(comment);
      expect(commentEditor.selectionStart).toBe(0);
      expect(commentEditor.selectionEnd).toBe(0);
    } finally {
      dispose();
    }
  });

  it("writes prefix and suffix when the selected phrase repeats in the block", () => {
    const { root, dispose } = mount([block("p", "the cat sat; then the cat ran")], "p");
    try {
      const ta = editor(root);
      const second = ta.value.lastIndexOf("the cat");
      ta.setSelectionRange(second, second + 7);
      ctrlR(ta);
      const comment = docNode("p")!.children[0];
      expect(docNode(comment)!.raw).toBe("\nquote:: the cat\nquote-prefix:: the cat sat; then\nquote-suffix:: ran");
    } finally {
      dispose();
    }
  });

  it("is one undo step: undo removes the comment and leaves the parent as it was", () => {
    const { root, dispose } = mount([block("p", "Some text to discuss")], "p");
    try {
      const ta = editor(root);
      ta.setSelectionRange(5, 9);
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

  it("with no selection comments on the whole block (an empty quote::)", () => {
    const { root, dispose } = mount([block("p", "Whole-block remark target")], "p");
    try {
      const ta = editor(root);
      ta.setSelectionRange(3, 3);
      const event = ctrlR(ta);
      expect(event.defaultPrevented).toBe(true);
      const comment = docNode("p")!.children[0];
      expect(docNode(comment)!.raw).toBe("\nquote:: ");
      expect(editingId()).toBe(comment);
    } finally {
      dispose();
    }
  });

  it("the palette command uses the selection remembered as the palette opened", () => {
    const { root, dispose } = mount([block("p", "Palette path for a phrase")], "p");
    try {
      const ta = editor(root);
      ta.focus();
      const at = ta.value.indexOf("a phrase");
      ta.setSelectionRange(at, at + 8);
      rememberEditorSelectionForPalette();
      endEdit("page-navigation"); // opening the palette blurs the editor
      commentOnPaletteSelection();
      const comment = docNode("p")!.children[0];
      expect(docNode(comment)!.raw).toBe("\nquote:: a phrase");
    } finally {
      dispose();
    }
  });

  it("the palette command refuses a selection whose graph was switched away (I-20)", () => {
    const { root, dispose } = mount([block("p", "Palette path for a phrase")], "p");
    try {
      setToasts([]);
      const ta = editor(root);
      ta.focus();
      ta.setSelectionRange(0, 7);
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
      block("agent", `${AGENT_TEXT}\nauthor:: claude`, [
        block("c1", "I doubt this.\nquote:: converges because", [
          block("r1", "It does, see the lemma.\nauthor:: claude", [block("r2", "Fair enough.")]),
        ]),
        block("c2", "Which one?\nquote:: the step size shrinks\nquote-prefix:: the step size shrinks, and\nquote-suffix:: geometrically."),
        block("c3", "Was rewritten.\nquote:: a sentence that is gone"),
        block("plain", "An ordinary child with status:: open"),
      ]),
    ]);
  }

  const row = (root: HTMLElement, id: string) => root.querySelector(`.ls-block[data-block-id="${id}"]`) as HTMLElement;
  const ownChip = (el: HTMLElement) => el.querySelector(":scope > .block-main .author-chip")?.textContent ?? null;

  it("shows a comment as a card with its quote header and hides the quote and author rows", () => {
    const { root, dispose } = renderedFixture();
    try {
      const c1 = row(root, "c1");
      expect(c1.classList.contains("comment-card")).toBe(true);
      expect(c1.querySelector(":scope > .comment-quote")?.textContent).toBe("converges because");
      expect(ownChip(c1)).toBe("you");
      const keys = [...root.querySelectorAll(".block-property-key")].map((k) => k.textContent);
      expect(keys).not.toContain("quote");
      expect(keys).not.toContain("quote-prefix");
      expect(keys).not.toContain("author");
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
      setRaw("c1", "I doubt this.\nquote:: geometrically");
      await Promise.resolve();
      expect(marks()).toEqual(["the step size shrinks", "geometrically"]);
      setRaw("agent", "Rewritten without the phrases.\nauthor:: claude");
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

  it("leaves a top-level quote:: block alone: it is not a comment", () => {
    const { root, dispose } = mount([block("top", "Top\nquote:: not a comment")]);
    try {
      expect(row(root, "top").querySelector(".comment-quote")).toBeNull();
      expect([...root.querySelectorAll(".block-property-key")].map((k) => k.textContent)).toContain("quote");
    } finally {
      dispose();
    }
  });
});
