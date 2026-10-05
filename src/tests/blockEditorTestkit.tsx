import { afterEach, beforeAll } from "vitest";
import { type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import type { BlockDto, PageDto } from "../types";

/** Shared setup for the `Block.*` editor-behaviour tests: parser init before
 *  the file, store reset and DOM cleanup after every test. Call once at the top
 *  of a test file. */
export function installBlockEditorLifecycle(): void {
  beforeAll(async () => {
    await initParser();
  });

  afterEach(() => {
    resetStore();
    document.body.innerHTML = "";
  });
}

export function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

export function blk(id: string, raw: string): BlockDto {
  return { id, raw, collapsed: false, children: [] };
}

export function page(name: string, blocks: BlockDto[]): PageDto {
  return { name, kind: "page", title: name, pre_block: null, blocks };
}

export function journal(name: string, blocks: BlockDto[]): PageDto {
  return { name, kind: "journal", title: name, pre_block: null, blocks };
}

export function pressEnter(ta: HTMLTextAreaElement, caret: number) {
  ta.focus();
  ta.selectionStart = caret;
  ta.selectionEnd = caret;
  ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
}
