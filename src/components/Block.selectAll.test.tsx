// GH #262: Ctrl/Cmd+A ladder from editing mode — first press is the native
// select-all-text gesture, a press with the text fully selected escalates to
// a block subtree selection; later presses live in selection mode and are
// covered by src/selectionExpand.test.ts.
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing, editingId } from "../editorController";
import { installKeybindings } from "../keybindings";
import { initParser } from "../render/parse";
import { hasSelection, pageByName, resetStore, selectedIds } from "../document";
import { doc } from "../document/model";
import { loadSingle } from "../document/workingSet";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

let disposeKeys: (() => void) | null = null;
beforeAll(async () => {
  await initParser();
  disposeKeys = installKeybindings();
});

afterAll(() => {
  disposeKeys?.();
});

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

let counter = 0;
function block(raw: string, children: BlockDto[] = []): BlockDto {
  return { id: `t${counter++}`, raw, collapsed: false, children };
}

function page(blocks: BlockDto[]): PageDto {
  return { name: "Sel", kind: "page", title: "Sel", pre_block: null, blocks };
}

const editor = (root: HTMLElement) => root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
const modA = (el: HTMLTextAreaElement) =>
  el.dispatchEvent(new KeyboardEvent("keydown", { key: "a", ctrlKey: true, bubbles: true, cancelable: true }));

describe("Ctrl+A ladder from the block editor (GH #262)", () => {
  it("first press leaves text selection to the browser; a press on fully selected text selects the block subtree", () => {
    loadSingle(page([
      block("P", [block("c1"), block("c2", [block("g1"), block("g2")]), block("c3")]),
      block("S"),
    ]));
    const c2Id = findByRaw("c2");
    startEditing(c2Id, 0);
    const m = mount(() => <For each={pageByName("Sel")?.roots ?? []}>{(id) => <Block id={id} />}</For>);
    try {
      const ta = editor(m.root);
      expect(ta.value).toBe("c2");

      // Press 1 with the caret collapsed: falls through (native select-all).
      ta.setSelectionRange(0, 0);
      modA(ta);
      expect(hasSelection()).toBe(false);
      expect(editingId()).toBe(c2Id);

      // Browser applied select-all: whole text selected.
      ta.setSelectionRange(0, ta.value.length);
      // Press 2: escalate to the block's whole visible subtree.
      modA(ta);
      expect(hasSelection()).toBe(true);
      expect(editingId()).toBeNull();
      expect(selectedIds()).toEqual([c2Id, findByRaw("g1"), findByRaw("g2")]);
    } finally {
      m.dispose();
    }
  });

  it("Ctrl+A on an empty block selects the block immediately", () => {
    loadSingle(page([block("before"), block(""), block("after")]));
    const emptyId = findByRaw("");
    startEditing(emptyId, 0);
    const m = mount(() => <For each={pageByName("Sel")?.roots ?? []}>{(id) => <Block id={id} />}</For>);
    try {
      const ta = editor(m.root);
      expect(ta.value).toBe("");
      modA(ta);
      expect(hasSelection()).toBe(true);
      expect(selectedIds()).toEqual([emptyId]);
    } finally {
      m.dispose();
    }
  });
});

function findByRaw(raw: string): string {
  for (const id of pageByName("Sel")!.roots) {
    const hit = findIn(id, raw);
    if (hit) return hit;
  }
  throw new Error(`block not found: ${raw}`);
}
function findIn(id: string, raw: string): string | null {
  const n = doc.byId[id];
  if (n.raw === raw) return id;
  for (const c of n.children) {
    const hit = findIn(c, raw);
    if (hit) return hit;
  }
  return null;
}
