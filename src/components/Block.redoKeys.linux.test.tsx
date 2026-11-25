import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// The keymap reads the platform once, when it loads: be Linux from the start.
vi.hoisted(() => Object.defineProperty(navigator, "platform", { configurable: true, value: "Linux x86_64" }));

import { render } from "solid-js/web";
import { editorIn, type EditorDriver, type EditorKind } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { endEdit, startEditing } from "../editorController";
import { installKeybindings } from "../keybindings";
import type { ShortcutBinding } from "../shortcutBindings";
import { Block } from "./Block";

// GH #1044 off macOS: Ctrl+Y stays Redo's second key (GH #491), and like the
// Mac's Cmd+Y (Block.redoKeys.mac.test.tsx) it yields to a command the user
// bound to it. Before, Redo beat the user's editor-scope binding.

beforeAll(() => initParser());

let dispose: (() => void) | null = null;
afterEach(() => {
  dispose?.(); dispose = null;
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
});

const ID = "10440000-2222-4222-8222-222222222222";
const ctrl = (key: string) => ({ key, code: `Key${key.toUpperCase()}`, ctrlKey: true });

/** A block being edited, " beta" typed after "alpha" and taken back by Ctrl+Z.
 *  Returns the block's editor as mounted now: Undo can remount it. */
function typedThenUndone(overrides: Record<string, ShortcutBinding> = {}): () => EditorDriver {
  const disposeKeys = installKeybindings(overrides);
  loadSingle({ name: "Redo keys", kind: "page", title: "Redo keys", pre_block: null, blocks: [
    { id: ID, raw: "alpha", collapsed: false, children: [] },
  ] });
  startEditing(ID, 5);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const disposeView = render(() => <Block id={ID} />, root);
  dispose = () => { disposeView(); disposeKeys(); };
  const editor = () => editorIn(root, ID);
  editor().focus();
  editor().select(5, 5);
  editor().type(" beta");
  expect(editor().keydown(ctrl("z")).defaultPrevented).toBe(true);
  expect(doc.byId[ID].raw).toBe("alpha");
  return editor;
}

describe.each<EditorKind>(["textarea", "cm6"])("GH #1044 Redo keys off macOS (%s)", kind => {
  useEditorKind(kind);
  beforeEach(() => { expect(navigator.platform).toBe("Linux x86_64"); });

  it("Ctrl+Y brings back the typing Undo took back", () => {
    const editor = typedThenUndone();
    expect(editor().keydown(ctrl("y")).defaultPrevented).toBe(true);
    expect(editor().value()).toBe("alpha beta");
    expect(doc.byId[ID].raw).toBe("alpha beta");
  });

  it("an editor command the user bound to Ctrl+Y runs instead of Redo", () => {
    const editor = typedThenUndone({ "editor/insert-link": "ctrl+y" });
    editor().select(0, 5);
    expect(editor().keydown(ctrl("y")).defaultPrevented).toBe(true);
    expect(editor().value()).toBe("[alpha]()");
  });

  it("outside the block editor, Ctrl+Y still redoes although the user bound it to an editor command", () => {
    typedThenUndone({ "editor/insert-link": "ctrl+y" });
    endEdit("page-navigation");
    expect(doc.byId[ID].raw).toBe("alpha");
    const e = new KeyboardEvent("keydown", { ...ctrl("y"), bubbles: true, cancelable: true });
    document.body.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(true);
    expect(doc.byId[ID].raw).toBe("alpha beta");
  });

  it("a binding of a command Tine does not know leaves Ctrl+Y to Redo", () => {
    const editor = typedThenUndone({ "editor/copy-text": "ctrl+y" });
    expect(editor().keydown(ctrl("y")).defaultPrevented).toBe(true);
    expect(editor().value()).toBe("alpha beta");
  });
});
