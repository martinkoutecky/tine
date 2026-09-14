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
  it.each([
    [0, 0, "none"],
    [6, 6, "none"],
    [11, 11, "none"],
    [2, 8, "forward"],
    [2, 8, "backward"],
  ] as const)("preserves selection %i..%i (%s) across reparenting (GH #519)", async (start, end, direction) => {
    loadSingle(page("Tabs", [block("previous", "parent"), block("current", "hello world")]));
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => (
      <For each={pageByName("Tabs")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    startEditing("current", 0);
    try {
      for (const shiftKey of [false, true]) {
        const editor = activeEditor(root);
        editor.focus();
        editor.setSelectionRange(start, end, direction);
        const event = keydown(editor, "Tab", { shiftKey });
        expect(event.defaultPrevented).toBe(true);
        await vi.waitFor(() => {
          expect(doc.byId.current.parent).toBe(shiftKey ? null : "previous");
          const next = activeEditor(root);
          expect(document.activeElement).toBe(next);
          expect(next.value).toBe("hello world");
          expect([next.selectionStart, next.selectionEnd, next.selectionDirection]).toEqual([start, end, direction]);
        });
      }
    } finally {
      dispose();
    }
  });

  it.each([
    ["hello world", "hello world", false],
    ["hello world", "hello world", true],
    ["```js\nhello world\n```", "hello world", false],
    ["```js\nhello world\n```", "hello world", true],
    ["hello 🌍\nsecond line", "hello 🌍\nsecond line", false],
    ["hello world\nid:: current", "hello world", false],
  ] as const)("keeps a clicked editor's selection for %s (visible=%s, outdent=%s)", async (raw, visible, outdent) => {
    const current = block("current", raw);
    const previous = block("previous", "parent");
    if (outdent) previous.children = [current];
    loadSingle(page("Tabs", outdent ? [previous] : [previous, current]));
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => (
      <For each={pageByName("Tabs")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const content = root.querySelector('[data-block-id="current"] .block-content')!;
      content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      const editor = activeEditor(root);
      expect(editor.value).toBe(visible);
      editor.setSelectionRange(2, 8, "backward");
      keydown(editor, "Tab", { shiftKey: outdent });
      await vi.waitFor(() => {
        const next = activeEditor(root);
        expect(document.activeElement).toBe(next);
        expect([next.selectionStart, next.selectionEnd, next.selectionDirection]).toEqual([2, 8, "backward"]);
        expect(next.value).toBe(visible);
        expect(doc.byId.current.parent).toBe(outdent ? null : "previous");
        expect(doc.byId.current.raw).toBe(raw);
      });
    } finally {
      dispose();
    }
  });

  it("keeps selection when there is no parent or previous sibling to move to", () => {
    loadSingle(page("Tabs", [block("current", "hello world")]));
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => <Block id="current" />);
    startEditing("current", 0);
    try {
      const editor = activeEditor(root);
      editor.setSelectionRange(2, 8, "backward");
      for (const shiftKey of [false, true]) {
        keydown(editor, "Tab", { shiftKey });
        expect(activeEditor(root)).toBe(editor);
        expect([editor.selectionStart, editor.selectionEnd, editor.selectionDirection]).toEqual([2, 8, "backward"]);
        expect(doc.pages[0].roots).toEqual(["current"]);
      }
    } finally {
      dispose();
    }
  });

  it("captures the selection before committing pending editor text", () => {
    loadSingle(page("Tabs", [block("previous", "parent"), block("current", "hello")]));
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => (
      <For each={pageByName("Tabs")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    startEditing("current", 0);
    try {
      const editor = activeEditor(root);
      editor.value = "hello world";
      editor.setSelectionRange(2, 8, "backward");
      keydown(editor, "Tab");
      const next = activeEditor(root);
      expect([next.selectionStart, next.selectionEnd, next.selectionDirection]).toEqual([2, 8, "backward"]);
      expect(doc.byId.current.raw).toBe("hello world");
      expect(doc.byId.current.parent).toBe("previous");
    } finally {
      dispose();
    }
  });

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
