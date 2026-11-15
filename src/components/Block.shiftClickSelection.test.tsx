import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { Block, SurfaceContext } from "./Block";
import { editingId, endEdit } from "../editorController";
import { clearSelection, hasSelection, pageByName, resetStore, selectedIds, selectionMarkdown } from "../document";
import { loadFeed, loadSingle } from "../document/workingSet";
import { initParser } from "../render/parse";
import { editorIn, maybeEditorIn } from "../tests/editorDriver";
import { installKeybindings } from "../keybindings";
import type { PageDto } from "../types";

beforeAll(initParser);
afterEach(() => {
  endEdit("page-navigation");
  clearSelection();
  resetStore();
  document.body.replaceChildren();
});

function mount(kind: "page" | "journal", surface: string) {
  const page: PageDto = {
    name: kind === "page" ? "Shift click" : "Nov 10th, 2026", title: "Shift click", kind, pre_block: null,
    blocks: ["A", "B", "C", "D"].map(id => ({
      id, raw: `Block ${id} has text after the click`, collapsed: id === "D",
      children: id === "D" ? [{ id: "child", raw: "Last block child", collapsed: false, children: [] }] : [],
    })),
  };
  if (kind === "journal") loadFeed([page]);
  else loadSingle(page);
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <SurfaceContext.Provider value={surface}>
    <For each={pageByName(page.name)?.roots ?? []}>{id => <Block id={id} />}</For>
  </SurfaceContext.Provider>, host);
  return { host, dispose };
}

function press(host: HTMLElement, id: string, shiftKey = false, padding = false) {
  const target = host.querySelector(`[data-block-id="${id}"] > .block-main .${padding ? "block-content-wrapper" : "block-content"}`)!;
  const event = new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, shiftKey, clientX: 30 });
  target.dispatchEvent(event);
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  return event;
}

describe("Shift+click selects whole blocks from the editing caret (GH #924)", () => {
  for (const kind of ["page", "journal"] as const) for (const surface of ["main", "sidebar"]) {
    it.each([false, true])(`${kind}, ${surface}, padding=%s: closes the editor and selects A through D`, padding => {
      const { host, dispose } = mount(kind, surface);
      try {
        press(host, "A");
        editorIn(host).select(7);
        const event = press(host, "D", true, padding);
        expect(selectedIds()).toEqual(["A", "B", "C", "D", "child"]);
        expect(hasSelection()).toBe(true);
        expect(editingId()).toBeNull();
        expect(maybeEditorIn(host)).toBeNull();
        expect(event.defaultPrevented).toBe(true);
        expect(host.querySelectorAll(".block-main.selected")).toHaveLength(4);
        // Whole-block operations retain all text and the collapsed endpoint's
        // children, regardless of the character under the pointer.
        expect(selectionMarkdown()).toContain("Block D has text after the click\n\t- Last block child");
        press(host, "B", true, padding);
        expect(selectedIds()).toEqual(["A", "B"]);
      } finally { dispose(); }
    });
  }

  it("selects upwards from the editing block too", () => {
    const { host, dispose } = mount("page", "main");
    try {
      press(host, "D");
      editorIn(host).select(7);
      press(host, "A", true);
      expect(selectedIds()).toEqual(["A", "B", "C", "D", "child"]);
      expect(editingId()).toBeNull();
    } finally { dispose(); }
  });

  it("keeps Shift+click within the active editor a native text selection", () => {
    const { host, dispose } = mount("page", "main");
    try {
      press(host, "A");
      const editor = editorIn(host);
      editor.select(7);
      const event = new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, shiftKey: true });
      editor.element.dispatchEvent(event);
      // The textarea keeps the browser's own Shift+click text extension; CodeMirror
      // extends its selection itself and cancels the default (view handlers.mousedown).
      if (editor.kind === "textarea") expect(event.defaultPrevented).toBe(false);
      expect(editingId()).toBe("A");
      expect(hasSelection()).toBe(false);
    } finally { dispose(); }
  });

  it("keeps an unanchored Shift+click outside editing unchanged", () => {
    const { host, dispose } = mount("page", "main");
    try {
      const event = press(host, "D", true);
      expect(event.defaultPrevented).toBe(false);
      expect(editingId()).toBeNull();
      expect(hasSelection()).toBe(false);
    } finally { dispose(); }
  });

  it.each(["ArrowUp", "ArrowDown"])("keyboard %s first selects the current block, as Logseq does", key => {
    const disposeKeys = installKeybindings();
    const { host, dispose } = mount("page", "main");
    try {
      press(host, "B");
      const editor = editorIn(host);
      editor.select(key === "ArrowUp" ? 0 : editor.value().length);
      editor.keydown({ key, shiftKey: true });
      expect(selectedIds()).toEqual(["B"]);
      expect(editingId()).toBeNull();
    } finally { dispose(); disposeKeys(); }
  });
});
