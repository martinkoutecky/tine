import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { maybeEditorIn } from "../tests/editorDriver";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { startEditing } from "../editorController";
import * as clipboard from "../clipboard";
import { toasts, setToasts } from "../toasts";
import { Block } from "./Block";

beforeAll(() => initParser());
afterEach(() => { vi.restoreAllMocks(); Reflect.deleteProperty(navigator, "platform"); resetStore(); setToasts([]); document.body.innerHTML = ""; });

describe("GH #857 plain-text paste (textarea)", () => {
  beforeEach(() => Object.defineProperty(navigator, "platform", { configurable: true, value: "MacIntel" }));

  function mountEditor() {
    const block = { id: "85700000-1111-4111-8111-111111111111", raw: "before SELECT after", collapsed: false, children: [] };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 7);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <For each={pageByName("Paste")?.roots ?? []}>{id => <Block id={id} />}</For>, root);
    const editor = maybeEditorIn(root)!;
    expect(editor.kind).toBe("textarea");
    editor.focus(); editor.select(7, 13);
    const shortcut = () => {
      const event = new KeyboardEvent("keydown", { key: "V", code: "KeyV", metaKey: true, shiftKey: true, bubbles: true, cancelable: true });
      editor.keydown(event);
      return event;
    };
    return { block, editor, dispose, shortcut };
  }

  it("pastes at the selection without a native paste event", async () => {
    const text = "Paste and match style\n- literal list";
    vi.spyOn(clipboard, "readClipboardText").mockResolvedValue(text);
    const { block, editor, dispose, shortcut } = mountEditor();
    try {
      const event = shortcut();
      await vi.waitFor(() => expect(editor.value()).toBe(`before ${text} after`), { timeout: 500 });
      expect(event.defaultPrevented).toBe(true);
      expect(doc.byId[block.id].raw).toBe(`before ${text} after`);
      expect(pageByName("Paste")!.roots).toEqual([block.id]);
      expect(editor.selection()).toEqual([7 + text.length, 7 + text.length]);
    } finally { dispose(); }
  });

  it.each(["selection", "typing", "dispose", "graph", "blur"] as const)("drops a late clipboard read after %s changes", async change => {
    let resolve!: (text: string) => void;
    vi.spyOn(clipboard, "readClipboardText").mockImplementation(() => new Promise(done => { resolve = done; }));
    const { block, editor, dispose, shortcut } = mountEditor();
    let disposed = false;
    try {
      shortcut();
      if (change === "selection") editor.select(0, 0);
      if (change === "typing") editor.input("typed meanwhile");
      if (change === "dispose") { dispose(); disposed = true; }
      if (change === "graph") resetStore();
      if (change === "blur") { editor.blur(); editor.focus(); }
      const before = editor.value();
      resolve("late clipboard content");
      await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
      if (!disposed && change !== "graph") expect(editor.value()).toBe(before);
      if (change === "dispose") expect(doc.byId[block.id].raw).toBe("before SELECT after");
      if (change === "graph") expect(doc.byId[block.id]).toBeUndefined();
      expect(document.body.textContent).not.toContain("late clipboard content");
      expect(toasts()).toEqual([]);
    } finally { if (!disposed) dispose(); }
  });

  it("keeps the selection when the clipboard has no plain text", async () => {
    vi.spyOn(clipboard, "readClipboardText").mockResolvedValue("");
    const { editor, dispose, shortcut } = mountEditor();
    try {
      shortcut();
      await Promise.resolve(); await Promise.resolve();
      expect(editor.value()).toBe("before SELECT after");
      expect(editor.selection()).toEqual([7, 13]);
    } finally { dispose(); }
  });

  it("reports an unreadable clipboard without changing the selected text", async () => {
    vi.spyOn(clipboard, "readClipboardText").mockRejectedValue(new Error("unavailable"));
    const { editor, dispose, shortcut } = mountEditor();
    try {
      shortcut();
      await vi.waitFor(() => expect(toasts()).toHaveLength(1));
      expect(editor.value()).toBe("before SELECT after");
      expect(toasts()[0].message).toBe("Couldn't read the clipboard");
    } finally { dispose(); }
  });

  it.each(["Linux x86_64", "Win32"])("keeps Ctrl/Meta+Shift+V on the native paste path on %s", platform => {
    Object.defineProperty(navigator, "platform", { configurable: true, value: platform });
    const read = vi.spyOn(clipboard, "readClipboardText");
    const { block, editor, dispose } = mountEditor();
    try {
      for (const modifier of ["ctrlKey", "metaKey"]) {
        editor.select(7, editor.value().length - 6);
        const key = new KeyboardEvent("keydown", { key: "V", code: "KeyV", [modifier]: true, shiftKey: true, bubbles: true, cancelable: true });
        editor.keydown(key);
        expect(key.defaultPrevented).toBe(false);
        editor.paste({ text: "heading\n- literal list", html: "<h1>heading</h1><ul><li>literal list</li></ul>" });
        expect(editor.value()).toBe("before heading\n- literal list after");
      }
      expect(pageByName("Paste")!.roots).toEqual([block.id]);
      expect(read).not.toHaveBeenCalled();
    } finally { dispose(); }
  });

  it("keeps formatting conversion on ordinary paste", async () => {
    const { editor, dispose } = mountEditor();
    try {
      editor.paste({ text: "Example", html: '<a href="https://example.com">Example</a>' });
      await Promise.resolve();
      expect(editor.value()).toBe("before [Example](https://example.com) after");
    } finally { dispose(); }
  });
});
