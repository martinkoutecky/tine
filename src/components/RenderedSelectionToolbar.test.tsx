import { maybeEditorIn } from "../tests/editorDriver";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, createRoot, createSignal, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { readFileSync } from "node:fs";
import { editingId, startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { isDirty, node as docNode, resetStore, undo } from "../document";
import { loadSingle } from "../document/workingSet";
import { pageByName } from "../document/model";
import type { BlockDto } from "../types";
import { Block } from "./Block";
import { RenderedSelectionToolbar } from "./RenderedSelectionToolbar";
import { setGraphMeta } from "../graphSession";
import { setTypographyModeSig, typographyMode } from "../ui";

// GH #375 (D-20261018-LP2): text selected in a RENDERED block is formatted in
// place. The block never enters edit mode, so the platform keeps its own
// selection handles and menu, the keyboard does not restart, nothing scrolls.
// Touch-first is the primary pointer (D-20261018-W1), reactive so a flip after
// mount re-registers; the mock is a signal for that reason.
const capability = vi.hoisted(() => ({ touch: null as null | (() => boolean), setTouch: null as null | ((v: boolean) => void) }));
createRoot(() => {
  const [touch, setTouch] = createSignal(true);
  capability.touch = touch;
  capability.setTouch = setTouch;
});
vi.mock("../inputCapability", async (original) => ({
  ...(await original<typeof import("../inputCapability")>()),
  touchPrimary: () => capability.touch!(),
  touchCapable: () => capability.touch!(),
  touchGestures: () => false,
}));

beforeAll(() => initParser());

afterEach(() => {
  document.getSelection()?.removeAllRanges();
  resetStore();
  document.body.innerHTML = "";
  capability.setTouch!(true);
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function load(format: "md" | "org", blocks: BlockDto[], readOnly = false) {
  loadSingle({ name: "Sel", kind: "page", title: "Sel", format, pre_block: null, blocks, ...(readOnly ? { read_only: true } : {}) });
}

const blocks = (second: string): BlockDto[] => [
  { id: "first", raw: "first block", collapsed: false, children: [] },
  { id: "second", raw: second, collapsed: false, children: [] },
];

/** `surfaces` copies of the page, like the main view plus a sidebar pane. */
function mountPage(surfaces = 1) {
  return mount(() => (
    <>
      <For each={Array.from({ length: surfaces })}>{() => (
        <div class="surface"><For each={pageByName("Sel")?.roots ?? []}>{(id) => <Block id={id} />}</For></div>
      )}</For>
      <RenderedSelectionToolbar />
    </>
  ));
}

function textNode(within: Element, needle: string): Text {
  const walker = document.createTreeWalker(within, NodeFilter.SHOW_TEXT);
  let text = walker.nextNode() as Text | null;
  while (text && !text.textContent?.includes(needle)) text = walker.nextNode() as Text | null;
  if (!text) throw new Error(`no text node with ${needle}`);
  return text;
}

/** A native selection of `needle` inside `within`, as a long press makes it. */
function selectText(within: Element, needle: string, backward = false) {
  const text = textNode(within, needle);
  const at = text.textContent!.indexOf(needle);
  const selection = document.getSelection()!;
  if (backward) selection.setBaseAndExtent(text, at + needle.length, text, at);
  else selection.setBaseAndExtent(text, at, text, at + needle.length);
  document.dispatchEvent(new Event("selectionchange"));
}

const row = (root: Element, id: string, surface = 0) =>
  root.querySelectorAll(".surface")[surface].querySelector(`[data-block-id="${id}"]`)!;
const toolbar = () => document.querySelector<HTMLElement>("[data-rendered-selection-toolbar]");
const press = (id: string) => {
  const button = toolbar()!.querySelector<HTMLButtonElement>(`[data-selection-action="${id}"]`)!;
  button.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true }));
  button.click();
};
const selectedText = () => document.getSelection()!.toString();

describe("rendered-selection toolbar: exact, owned mapping (GH #375 review r1)", () => {
  it("never rewrites a code block when its code is selected", () => {
    const raw = "```js\nconst answer = 42\n```";
    load("md", blocks(raw));
    const page = mountPage();
    try {
      selectText(row(page.root, "second"), "const answer = 42");
      expect(toolbar()).toBeNull();
      // Even a stale toolbar must not write: nothing maps byte-exactly here.
      if (toolbar()) press("bold");
      expect(docNode("second").raw).toBe(raw);
    } finally { page.dispose(); }
  });

  it("never rewrites the host block when a block-level macro's output is selected", () => {
    setGraphMeta({ macros: { m: "macro selected\n\nsecond paragraph" } } as never);
    const raw = "prefix {{m}} suffix";
    load("md", blocks(raw));
    const page = mountPage();
    try {
      selectText(row(page.root, "second"), "selected");
      expect(toolbar()).toBeNull();
      if (toolbar()) press("bold");
      expect(docNode("second").raw).toBe(raw);
    } finally { page.dispose(); setGraphMeta({ macros: {} } as never); }
  });

  it("keeps the selection on the formatted prose, not in a macro's output", () => {
    setGraphMeta({ macros: { m: "macro selected\n\nsecond paragraph" } } as never);
    load("md", blocks("prefix {{m}} suffix"));
    const page = mountPage();
    try {
      selectText(row(page.root, "second"), "suffix");
      press("bold");
      expect(docNode("second").raw).toBe("prefix {{m}} **suffix**");
      expect(selectedText()).toBe("suffix");
      expect(row(page.root, "second").querySelector(".macro-blocks")!.contains(document.getSelection()!.anchorNode)).toBe(false);
    } finally { page.dispose(); setGraphMeta({ macros: {} } as never); }
  });

  it("ends editing in A when text is selected in a block B that offers no formatting", () => {
    load("md", blocks("```js\nconst answer = 42\n```"));
    startEditing("first", 0);
    const page = mountPage();
    const hasFocus = vi.spyOn(document, "hasFocus").mockReturnValue(true);
    try {
      const editor = maybeEditorIn(page.root)!;
      editor.element.focus();
      editor.input("first block edited", undefined, { inputType: "", data: null });
      selectText(row(page.root, "second"), "answer");
      expect(selectedText()).toBe("answer");
      expect(editingId()).toBeNull();
      expect(docNode("first").raw).toBe("first block edited");
      expect(toolbar()).toBeNull();
    } finally { hasFocus.mockRestore(); page.dispose(); }
  });

  it("commits A's text still being composed before ending its edit", () => {
    load("md", blocks("alpha selected omega"));
    startEditing("first", 0);
    const page = mountPage();
    const hasFocus = vi.spyOn(document, "hasFocus").mockReturnValue(true);
    try {
      const editor = maybeEditorIn(page.root)!;
      editor.element.focus();
      editor.compositionStart(new CompositionEvent("compositionstart", { bubbles: true }));
      editor.input("draft still composing", undefined, { inputType: "", data: null, isComposing: true });
      expect(docNode("first").raw).toBe("first block");
      selectText(row(page.root, "second"), "selected");
      expect(docNode("first").raw).toBe("draft still composing");
      expect(editingId()).toBeNull();
      expect(selectedText()).toBe("selected");
    } finally { hasFocus.mockRestore(); page.dispose(); }
  });

  it.each([
    ["md", "title\nid:: 6500a1b2-0000-4000-8000-000000000001\nnext selected words", "title\nid:: 6500a1b2-0000-4000-8000-000000000001\nnext **selected** words"],
    ["org", "title\n:PROPERTIES:\n:ID: 6500a1b2-0000-4000-8000-000000000001\n:END:\nnext selected words", "title\n:PROPERTIES:\n:ID: 6500a1b2-0000-4000-8000-000000000001\n:END:\nnext *selected* words"],
    ["md", "title\nsecond selected words\nthird line", "title\nsecond **selected** words\nthird line"],
    ["md", "title\n| a | b |\n|---|---|\n| x | selected |", "title\n| a | b |\n|---|---|\n| x | **selected** |"],
  ] as const)("formats exactly the selected word (%s): %j", (format, raw, expected) => {
    load(format, blocks(raw));
    const page = mountPage();
    try {
      selectText(row(page.root, "second"), "selected");
      expect(toolbar()).not.toBeNull();
      press("bold");
      expect(docNode("second").raw).toBe(expected);
      expect(selectedText()).toBe("selected");
      expect(editingId()).toBeNull();
    } finally { page.dispose(); }
  });

  it("leaves no selection listener behind across capability flips and unmount", () => {
    const add = vi.spyOn(document, "addEventListener");
    const remove = vi.spyOn(document, "removeEventListener");
    load("md", blocks("alpha selected omega"));
    const page = mountPage();
    try {
      for (let i = 0; i < 3; i++) {
        selectText(row(page.root, "second"), "selected");
        expect(toolbar()).not.toBeNull();
        capability.setTouch!(false);
        expect(toolbar()).toBeNull();
        capability.setTouch!(true);
      }
      page.dispose();
      const added = add.mock.calls.filter(([type]) => type === "selectionchange").map(([, fn]) => fn);
      const removed = remove.mock.calls.filter(([type]) => type === "selectionchange").map(([, fn]) => fn);
      expect(added).toHaveLength(4);
      expect(removed).toHaveLength(4);
      expect(new Set(removed)).toEqual(new Set(added));
    } finally { add.mockRestore(); remove.mockRestore(); }
  });
});

describe("rendered-selection toolbar: typographic glyphs (GH #375 review r2)", () => {
  // Settings' render-time typography shows `->` as `→`: the glyph has no
  // byte-exact source position, so a selection end beside it maps to the edge
  // of the prose it actually covers, never across the unselected arrow.
  const initialTypography = typographyMode();
  afterEach(() => setTypographyModeSig(initialTypography));

  it("bolds only the prose selected before an arrow, not the arrow", () => {
    setTypographyModeSig("render");
    load("md", blocks("alpha -> selected omega"));
    const page = mountPage();
    try {
      expect(row(page.root, "second").textContent).toContain("alpha → selected omega");
      selectText(row(page.root, "second"), "alpha ");
      press("bold");
      expect(docNode("second").raw).toBe("**alpha** -> selected omega");
    } finally { page.dispose(); }
  });

  it("includes a selected arrow in the formatted range", () => {
    setTypographyModeSig("render");
    load("md", blocks("alpha -> selected omega"));
    const page = mountPage();
    try {
      selectText(row(page.root, "second"), "→ selected");
      press("bold");
      expect(docNode("second").raw).toBe("alpha **-> selected** omega");
    } finally { page.dispose(); }
  });

  it("bolds only the prose selected after an arrow", () => {
    setTypographyModeSig("render");
    load("md", blocks("alpha -> selected omega"));
    const page = mountPage();
    try {
      selectText(row(page.root, "second"), "selected");
      press("bold");
      expect(docNode("second").raw).toBe("alpha -> **selected** omega");
      expect(selectedText()).toBe("selected");
    } finally { page.dispose(); }
  });
});

describe("rendered-selection toolbar (GH #375)", () => {
  it.each([
    ["md", "alpha **selected** omega"],
    ["org", "alpha *selected* omega"],
  ] as const)("formats a %s selection in another block without entering edit mode", async (format, bolded) => {
    load(format, blocks("alpha selected omega"));
    startEditing("first", 0);
    const page = mountPage();
    // The app window has focus (jsdom answers false, which reads as "switched apps").
    const hasFocus = vi.spyOn(document, "hasFocus").mockReturnValue(true);
    try {
      // Block A is being edited, with typed text not yet saved to disk.
      const editor = maybeEditorIn(page.root)!;
      editor.element.focus();
      editor.input("first block edited", undefined, { inputType: "", data: null });

      selectText(row(page.root, "second"), "selected");

      // (b) A's edit ends through its normal exit, with its text kept.
      expect(editingId()).toBeNull();
      expect(maybeEditorIn(page.root)).toBeNull();
      expect(docNode("first").raw).toBe("first block edited");
      expect(isDirty("Sel")).toBe(true);
      // The selection stays native, in the rendered block.
      expect(toolbar()).not.toBeNull();
      expect(selectedText()).toBe("selected");

      press("bold");
      expect(docNode("second").raw).toBe(bolded);
      expect(maybeEditorIn(page.root)).toBeNull();
      expect(editingId()).toBeNull();
      // Re-selected in the re-rendered block, so a second tap toggles it back.
      expect(selectedText()).toBe("selected");
      expect(row(page.root, "second").contains(document.getSelection()!.anchorNode)).toBe(true);
      expect(toolbar()).not.toBeNull();

      press("bold");
      expect(docNode("second").raw).toBe("alpha selected omega");
      expect(selectedText()).toBe("selected");
    } finally { hasFocus.mockRestore(); page.dispose(); }
  });

  it("goes through the normal edit path: one undo step restores the text", () => {
    load("md", blocks("alpha selected omega"));
    const page = mountPage();
    try {
      selectText(row(page.root, "second"), "selected");
      press("italic");
      expect(docNode("second").raw).toBe("alpha *selected* omega");
      expect(isDirty("Sel")).toBe(true);
      expect(undo()).toBe(true);
      expect(docNode("second").raw).toBe("alpha selected omega");
    } finally { page.dispose(); }
  });

  it("keeps hidden properties and maps through rendered markup", () => {
    load("md", blocks("see [[Target]] and selected words\nid:: 6500a1b2-0000-4000-8000-000000000001"));
    const page = mountPage();
    try {
      selectText(row(page.root, "second"), "selected", true);
      press("inline-code");
      expect(docNode("second").raw).toBe("see [[Target]] and `selected` words\nid:: 6500a1b2-0000-4000-8000-000000000001");
      expect(selectedText()).toBe("selected");
      // A backward selection stays backward.
      const selection = document.getSelection()!;
      expect(selection.anchorOffset).toBeGreaterThan(selection.focusOffset);
    } finally { page.dispose(); }
  });

  it("formats the copy the selection is in when the block shows in two panes", () => {
    load("md", blocks("alpha selected omega"));
    const page = mountPage(2);
    try {
      selectText(row(page.root, "second", 1), "selected");
      press("strikethrough");
      expect(docNode("second").raw).toBe("alpha ~~selected~~ omega");
      expect(row(page.root, "second", 1).contains(document.getSelection()!.anchorNode)).toBe(true);
      expect(maybeEditorIn(page.root)).toBeNull();
    } finally { page.dispose(); }
  });

  it("drops the selection when the result has no exact rendered position (page link)", () => {
    load("md", blocks("alpha selected omega"));
    const page = mountPage();
    try {
      selectText(row(page.root, "second"), "selected");
      press("page-link");
      expect(docNode("second").raw).toBe("alpha [[selected]] omega");
      expect(toolbar()).toBeNull();
      expect(maybeEditorIn(page.root)).toBeNull();
    } finally { page.dispose(); }
  });

  it("offers no toolbar for a selection across blocks, on a read-only page, or without a touch-first pointer", () => {
    load("md", blocks("alpha selected omega"));
    let page = mountPage();
    try {
      const first = textNode(row(page.root, "first"), "first block");
      const second = textNode(row(page.root, "second"), "alpha");
      document.getSelection()!.setBaseAndExtent(first, 2, second, 3);
      document.dispatchEvent(new Event("selectionchange"));
      expect(toolbar()).toBeNull();

      capability.setTouch!(false);
      selectText(row(page.root, "second"), "selected");
      expect(toolbar()).toBeNull();
      // Capability flips after mount: the listener follows it.
      capability.setTouch!(true);
      selectText(row(page.root, "second"), "selected");
      expect(toolbar()).not.toBeNull();
    } finally { page.dispose(); }

    resetStore();
    document.getSelection()?.removeAllRanges();
    load("md", blocks("alpha selected omega"), true);
    page = mountPage();
    try {
      selectText(row(page.root, "second"), "selected");
      expect(toolbar()).toBeNull();
    } finally { page.dispose(); }
  });

  it("offers no toolbar inside a code block, which has no byte-exact span mapping", () => {
    load("md", blocks("```js\nconst answer = 42\n```"));
    const page = mountPage();
    try {
      selectText(row(page.root, "second"), "answer");
      expect(selectedText()).toBe("answer");
      expect(toolbar()).toBeNull();
      expect(maybeEditorIn(page.root)).toBeNull();
    } finally { page.dispose(); }
  });

  it("refuses to write when the block changed after the selection was read", () => {
    load("md", blocks("alpha selected omega"));
    const page = mountPage();
    try {
      selectText(row(page.root, "second"), "selected");
      const button = toolbar()!.querySelector<HTMLButtonElement>('[data-selection-action="bold"]')!;
      // The press clears the native selection (the platform may), then another
      // surface rewrites the block before the click lands.
      button.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true }));
      document.getSelection()!.removeAllRanges();
      document.dispatchEvent(new Event("selectionchange"));
      expect(toolbar()).not.toBeNull();
      loadSingle({ name: "Sel", kind: "page", title: "Sel", format: "md", pre_block: null, blocks: blocks("changed elsewhere") });
      button.click();
      expect(docNode("second").raw).toBe("changed elsewhere");
    } finally { page.dispose(); }
  });

  it("docks above the bottom navigation row, or above the keyboard dock when one is up", () => {
    load("md", blocks("alpha selected omega"));
    const page = mountPage();
    try {
      selectText(row(page.root, "second"), "selected");
      const style = document.createElement("style");
      style.textContent = readFileSync("src/styles/touchGestures.css", "utf8");
      document.head.append(style);
      const bar = toolbar()!;
      expect(getComputedStyle(bar).position).toBe("fixed");
      expect(getComputedStyle(bar).top).toBe("auto");
      expect(getComputedStyle(bar).bottom).toContain("--mobile-navigation-height");
      expect(getComputedStyle(bar).bottom).toContain("--mobile-kb-toolbar-lift");
      style.remove();
    } finally { page.dispose(); }
  });
});
