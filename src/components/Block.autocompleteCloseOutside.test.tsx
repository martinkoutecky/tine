import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

// OG parity: editor.cljs `close-autocomplete-if-outside` (on click and keyup)
// clears page/hashtag/block search once the caret leaves the reference. A stale
// popup otherwise keeps ArrowUp/Down/Enter, and Enter rewrites the earlier link.

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function page(raw: string): PageDto {
  const block: BlockDto = { id: "ac-outside", raw, collapsed: false, children: [] };
  return { name: "Ac outside", kind: "page", title: "Ac outside", pre_block: null, blocks: [block] };
}

function inputAt(textarea: HTMLTextAreaElement, value: string, caret: number) {
  textarea.focus();
  textarea.value = value;
  textarea.setSelectionRange(caret, caret);
  textarea.dispatchEvent(new InputEvent("input", {
    bubbles: true,
    inputType: "insertText",
    data: value[caret - 1] ?? null,
  }));
}

function moveCaretByClick(textarea: HTMLTextAreaElement, caret: number) {
  textarea.setSelectionRange(caret, caret);
  textarea.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
}

async function openPopup(value: string, caret: number) {
  loadSingle(page(value));
  startEditing("ac-outside", caret);
  const mounted = mount(() => (
    <For each={pageByName("Ac outside")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ));
  const textarea = mounted.root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
  inputAt(textarea, value, caret);
  await vi.waitFor(() => expect(document.body.querySelector(".autocomplete")).not.toBeNull());
  return { ...mounted, textarea };
}

const VALUE = "see [[Alpha]] then\nnext line";
const INSIDE = VALUE.indexOf("Alpha") + 3;

describe("page completion closes when the caret leaves the reference", () => {
  it("closes on a click outside the [[...]], and Enter then acts at the new caret", async () => {
    const { textarea, dispose } = await openPopup(VALUE, INSIDE);
    try {
      moveCaretByClick(textarea, VALUE.length);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete")).toBeNull());
      const enter = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
      textarea.dispatchEvent(enter);
      expect(doc.byId["ac-outside"].raw.startsWith("see [[Alpha]] then")).toBe(true);
    } finally {
      dispose();
    }
  });

  it("closes when a selectionchange moves the caret out (keyboard / programmatic moves)", async () => {
    const { textarea, dispose } = await openPopup(VALUE, INSIDE);
    try {
      textarea.setSelectionRange(1, 1);
      document.dispatchEvent(new Event("selectionchange"));
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete")).toBeNull());
    } finally {
      dispose();
    }
  });

  it.each([
    ["tag", "a #proj then\nmore", "a #proj".length],
  ])("closes %s search when the caret leaves it", async (_kind, value, caret) => {
    const { textarea, dispose } = await openPopup(value, caret);
    try {
      moveCaretByClick(textarea, value.length);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete")).toBeNull());
    } finally {
      dispose();
    }
  });

  it("stays open while the caret moves inside the same reference", async () => {
    const { textarea, dispose } = await openPopup(VALUE, INSIDE);
    try {
      moveCaretByClick(textarea, INSIDE - 1);
      await new Promise((resolve) => setTimeout(resolve, 150));
      expect(document.body.querySelector(".autocomplete")).not.toBeNull();
    } finally {
      dispose();
    }
  });
});
