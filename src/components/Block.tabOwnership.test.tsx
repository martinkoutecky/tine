import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { installKeybindings } from "../keybindings";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, resetStore } from "../store";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(async () => {
  await initParser();
});

let disposeKeys: (() => void) | null = null;

afterEach(() => {
  disposeKeys?.();
  disposeKeys = null;
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function block(id: string, raw: string): BlockDto {
  return { id, raw, collapsed: false, children: [] };
}

function page(name: string, blocks: BlockDto[]): PageDto {
  return { name, kind: "page", title: name, pre_block: null, blocks };
}

function keydown(target: EventTarget, key: string, init: Partial<KeyboardEvent> = {}): KeyboardEvent {
  const event = new KeyboardEvent("keydown", {
    key,
    code: init.code ?? (key === "Tab" ? "Tab" : ""),
    bubbles: true,
    cancelable: true,
    shiftKey: init.shiftKey ?? false,
    ctrlKey: init.ctrlKey ?? false,
    metaKey: init.metaKey ?? false,
    altKey: init.altKey ?? false,
  });
  target.dispatchEvent(event);
  return event;
}

function activeEditor(root: HTMLElement): HTMLTextAreaElement {
  const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement | null;
  if (!textarea) throw new Error("missing active block editor");
  return textarea;
}

function openCompletion(textarea: HTMLTextAreaElement) {
  textarea.focus();
  textarea.setSelectionRange(textarea.value.length, textarea.value.length);
  textarea.dispatchEvent(new InputEvent("input", {
    bubbles: true,
    inputType: "insertText",
    data: textarea.value.at(-1) ?? null,
  }));
}

describe("outline Tab ownership (GH #157)", () => {
  it("indents and outdents an actual editor target, but declines every modified Tab", async () => {
    loadSingle(page("Tabs", [block("previous", "Previous"), block("current", "Current")]));
    startEditing("current", 2);
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => (
      <For each={pageByName("Tabs")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      let editor = activeEditor(root);
      const indent = keydown(editor, "Tab");
      await vi.waitFor(() => expect(doc.byId.current.parent).toBe("previous"));
      expect(doc.byId.previous.children).toEqual(["current"]);
      expect(indent.defaultPrevented).toBe(true);

      editor = activeEditor(root);
      const outdent = keydown(editor, "Unidentified", { code: "Tab", shiftKey: true });
      await vi.waitFor(() => expect(doc.byId.current.parent).toBeNull());
      expect(doc.pages[0].roots).toEqual(["previous", "current"]);
      expect(outdent.defaultPrevented).toBe(true);

      for (const init of [
        { ctrlKey: true },
        { ctrlKey: true, shiftKey: true },
        { altKey: true },
        { metaKey: true },
      ]) {
        const declined = keydown(activeEditor(root), "Tab", init);
        expect(declined.defaultPrevented).toBe(false);
        expect(doc.pages[0].roots).toEqual(["previous", "current"]);
      }

      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Super", code: "SuperLeft", bubbles: true }));
      const trackedSuper = keydown(activeEditor(root), "Tab");
      expect(trackedSuper.defaultPrevented).toBe(false);
      expect(doc.pages[0].roots).toEqual(["previous", "current"]);
      window.dispatchEvent(new KeyboardEvent("keyup", { key: "Super", code: "SuperLeft", bubbles: true }));
    } finally {
      dispose();
    }
  });

  it("accepts only permitted Tab while code-language autocomplete is open", async () => {
    loadSingle(page("Code", [block("code", "```js")]));
    startEditing("code", 5);
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => (
      <For each={pageByName("Code")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const editor = activeEditor(root);
      openCompletion(editor);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-item")).not.toBeNull());

      for (const init of [
        { ctrlKey: true },
        { ctrlKey: true, shiftKey: true },
        { altKey: true },
        { metaKey: true },
      ]) {
        const declined = keydown(editor, "Tab", init);
        expect(declined.defaultPrevented).toBe(false);
        expect(doc.byId.code.raw).toBe("```js");
        expect(document.body.querySelector(".autocomplete .ac-item")).not.toBeNull();
      }

      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Super", code: "SuperLeft", bubbles: true }));
      const trackedSuper = keydown(editor, "Tab");
      expect(trackedSuper.defaultPrevented).toBe(false);
      expect(doc.byId.code.raw).toBe("```js");
      window.dispatchEvent(new KeyboardEvent("keyup", { key: "Super", code: "SuperLeft", bubbles: true }));

      const accepted = keydown(editor, "Tab");
      expect(accepted.defaultPrevented).toBe(true);
      await vi.waitFor(() => expect(doc.byId.code.raw).toBe("```javascript"));
    } finally {
      dispose();
    }
  });
});
