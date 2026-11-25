import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

// The keymap reads the platform once, when it loads: be Linux from the start.
vi.hoisted(() => Object.defineProperty(navigator, "platform", { configurable: true, value: "Linux x86_64" }));

import { render } from "solid-js/web";
import { maybeEditorIn, type EditorKind } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { endEdit, startEditing } from "../editorController";
import { installKeybindings } from "../keybindings";
import * as router from "../router";
import { Block } from "./Block";

// GH #1040 changes macOS only: elsewhere Alt+Left / Alt+Right go Back and
// Forward from inside a block editor, as on develop (GH #948 sends them with no
// editor focused).

beforeAll(() => initParser());

let dispose: (() => void) | null = null;
afterEach(() => {
  dispose?.(); dispose = null;
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
});

const ID = "10400000-4444-4444-8444-444444444444";
const routeName = () => (router.route() as { name?: string }).name;

describe.each<EditorKind>(["textarea", "cm6"])("Linux block editor (%s)", kind => {
  useEditorKind(kind);

  it("Alt+Left goes Back and Alt+Right Forward while a block is being edited", () => {
    const disposeKeys = installKeybindings();
    loadSingle({ name: "Linux keys", kind: "page", title: "Linux keys", pre_block: null, blocks: [
      { id: ID, raw: "alpha beta", collapsed: false, children: [] },
    ] });
    router.openPage("Earlier page", "page");
    router.openPage("Linux keys", "page");
    startEditing(ID, 6);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const disposeView = render(() => <Block id={ID} />, root);
    dispose = () => { disposeView(); disposeKeys(); };
    const editor = maybeEditorIn(root, ID)!;
    editor.focus();
    expect(navigator.platform).toBe("Linux x86_64");
    expect(editor.keydown({ key: "ArrowLeft", code: "ArrowLeft", altKey: true }).defaultPrevented).toBe(true);
    expect(routeName()).toBe("Earlier page");
    expect(editor.keydown({ key: "ArrowRight", code: "ArrowRight", altKey: true }).defaultPrevented).toBe(true);
    expect(routeName()).toBe("Linux keys");
    expect(doc.byId[ID].raw).toBe("alpha beta");
  });
});
