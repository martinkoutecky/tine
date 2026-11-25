import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// The keymap reads the platform once, when it loads: be a Mac from the start.
vi.hoisted(() => Object.defineProperty(navigator, "platform", { configurable: true, value: "MacIntel" }));

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

// GH #1044: Redo answers Logseq's two keys, mod+shift+z and mod+y: on a Mac,
// Cmd+Shift+Z and Cmd+Y, while physical Ctrl+Y stays the system's yank. The
// second key is a default only: a chord the user bound to any command, in any
// scope, is theirs. Other platforms: Block.redoKeys.linux.test.tsx.

beforeAll(() => initParser());

let dispose: (() => void) | null = null;
afterEach(() => {
  dispose?.(); dispose = null;
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
});

const ID = "10440000-1111-4111-8111-111111111111";
const cmd = (key: string, shiftKey = false) => ({ key, code: `Key${key.toUpperCase()}`, metaKey: true, shiftKey });

/** A block being edited, " beta" typed after "alpha" and taken back by Cmd+Z.
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
  expect(editor().value()).toBe("alpha beta");
  expect(editor().keydown(cmd("z")).defaultPrevented).toBe(true);
  expect(editor().value()).toBe("alpha");
  expect(doc.byId[ID].raw).toBe("alpha");
  return editor;
}

describe.each<EditorKind>(["textarea", "cm6"])("GH #1044 Redo keys on macOS (%s)", kind => {
  useEditorKind(kind);
  beforeEach(() => { expect(navigator.platform).toBe("MacIntel"); });

  it("Cmd+Y brings back the typing Undo took back (the report)", () => {
    const editor = typedThenUndone();
    expect(editor().keydown(cmd("y")).defaultPrevented).toBe(true);
    expect(editor().value()).toBe("alpha beta");
    expect(doc.byId[ID].raw).toBe("alpha beta");
  });

  it("Cmd+Shift+Z still redoes", () => {
    const editor = typedThenUndone();
    expect(editor().keydown(cmd("z", true)).defaultPrevented).toBe(true);
    expect(doc.byId[ID].raw).toBe("alpha beta");
  });

  it("physical Ctrl+Y is the system's yank: no Redo, and the key is left to the field", () => {
    const editor = typedThenUndone();
    expect(editor().keydown({ key: "y", code: "KeyY", ctrlKey: true }).defaultPrevented).toBe(false);
    expect(doc.byId[ID].raw).toBe("alpha");
  });

  it("an editor command the user bound to Cmd+Y runs instead of Redo", () => {
    const editor = typedThenUndone({ "editor/insert-link": "mod+y" });
    editor().select(0, 5);
    expect(editor().keydown(cmd("y")).defaultPrevented).toBe(true);
    expect(editor().value()).toBe("[alpha]()");
  });

  it("a user's own Redo binding takes Cmd+Y with it", () => {
    const editor = typedThenUndone({ "editor/redo": "mod+alt+r" });
    editor().keydown(cmd("y"));
    expect(doc.byId[ID].raw).toBe("alpha");
    expect(editor().keydown({ ...cmd("r"), altKey: true }).defaultPrevented).toBe(true);
    expect(doc.byId[ID].raw).toBe("alpha beta");
  });
});
