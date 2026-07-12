import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { installKeybindings } from "../keybindings";
import { initParser } from "../render/parse";
import { loadSingle, pageByName, resetStore } from "../store";
import type { BlockDto } from "../types";
import { Block } from "./Block";

beforeAll(() => initParser());

afterEach(() => {
  installKeybindings()();
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function mountEditor(raw = "before selected after") {
  const block: BlockDto = { id: "selection-wrap", raw, collapsed: false, children: [] };
  loadSingle({ name: "Wrap", kind: "page", title: "Wrap", pre_block: null, blocks: [block] });
  startEditing(block.id, 0);
  const mounted = mount(() => (
    <For each={pageByName("Wrap")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ));
  return {
    ...mounted,
    textarea: mounted.root.querySelector("textarea.block-editor") as HTMLTextAreaElement,
  };
}

function keydown(textarea: HTMLTextAreaElement, init: KeyboardEventInit) {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  textarea.dispatchEvent(event);
  return event;
}

describe("selection wrapping with Alt-modified literal delimiters (GH #83)", () => {
  it("wraps twice with a literal Alt+[ and opens page completion", async () => {
    const { textarea, dispose } = mountEditor();
    try {
      textarea.setSelectionRange(7, 15);

      const first = keydown(textarea, { key: "[", code: "BracketLeft", altKey: true });
      expect(first.defaultPrevented).toBe(true);
      expect(textarea.value).toBe("before [selected] after");
      await vi.waitFor(() =>
        expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([8, 16])
      );

      const second = keydown(textarea, { key: "[", code: "BracketLeft", altKey: true });
      expect(second.defaultPrevented).toBe(true);
      expect(textarea.value).toBe("before [[selected]] after");
      await vi.waitFor(() =>
        expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([17, 17])
      );
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete")).not.toBeNull());
    } finally {
      dispose();
    }
  });

  it("does not reinterpret a layout-produced non-delimiter by physical key code", () => {
    const { textarea, dispose } = mountEditor();
    try {
      textarea.setSelectionRange(7, 15);
      const event = keydown(textarea, { key: "“", code: "BracketLeft", altKey: true });
      expect(event.defaultPrevented).toBe(false);
      expect(textarea.value).toBe("before selected after");
    } finally {
      dispose();
    }
  });

  it("gives an explicit Alt+[ editor binding precedence over incidental wrapping", () => {
    const disposeKeys = installKeybindings({ "editor/bold": "alt+[" });
    const { textarea, dispose } = mountEditor("selected");
    try {
      textarea.setSelectionRange(0, 8);
      const event = keydown(textarea, { key: "[", code: "BracketLeft", altKey: true });
      expect(event.defaultPrevented).toBe(true);
      expect(textarea.value).toBe("**selected**");
    } finally {
      dispose();
      disposeKeys();
    }
  });
});
