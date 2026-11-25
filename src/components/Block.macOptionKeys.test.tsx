import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// The keymap reads the platform once, when it loads: be a Mac from the start.
vi.hoisted(() => Object.defineProperty(navigator, "platform", { configurable: true, value: "MacIntel" }));

import { For } from "solid-js";
import { render } from "solid-js/web";
import { maybeEditorIn, type EditorDriver, type EditorKind } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { endEdit, startEditing } from "../editorController";
import { installKeybindings } from "../keybindings";
import type { ShortcutBinding } from "../shortcutBindings";
import * as router from "../router";
import { Block } from "./Block";

// GH #1040: on a Mac, Option with an arrow, a letter or Shift+Up/Down is the
// text system's (word motion, paragraph selection, typed symbols such as ¬ or ∑).
// The default keymap follows Logseq's Mac chords for the commands that used to
// take those keys: Back/Forward are Cmd+[ / Cmd+], the Emacs motions are on
// Control, and Move block up/down is Cmd+Shift+Up/Down.

beforeAll(() => initParser());

let disposeKeys: (() => void) | null = null;
let disposeView: (() => void) | null = null;
afterEach(() => {
  disposeView?.(); disposeView = null;
  disposeKeys?.(); disposeKeys = null;
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
});

const FIRST = "10400000-1111-4111-8111-111111111111";
const SECOND = "10400000-2222-4222-8222-222222222222";
const THIRD = "10400000-3333-4333-8333-333333333333";
const TEXT = "alpha beta gamma";

function mount(overrides: Record<string, ShortcutBinding> = {}): EditorDriver {
  disposeKeys = installKeybindings(overrides);
  loadSingle({ name: "Mac keys", kind: "page", title: "Mac keys", pre_block: null, blocks: [
    { id: FIRST, raw: "first", collapsed: false, children: [] },
    { id: SECOND, raw: TEXT, collapsed: false, children: [] },
    { id: THIRD, raw: "third", collapsed: false, children: [] },
  ] });
  // History on both sides, so Back and Forward each have somewhere to go.
  router.openPage("Earlier page", "page");
  router.openPage("Mac keys", "page");
  router.openPage("Later page", "page");
  router.goBack();
  startEditing(SECOND, 6);
  const root = document.createElement("div");
  document.body.appendChild(root);
  disposeView = render(() => <For each={pageByName("Mac keys")?.roots ?? []}>{id => <Block id={id} />}</For>, root);
  const editor = maybeEditorIn(root, SECOND)!;
  editor.focus();
  editor.select(6, 6);
  return editor;
}

const routeName = () => (router.route() as { name?: string }).name;

describe.each<EditorKind>(["textarea", "cm6"])("GH #1040 macOS Option keys stay with the editor (%s)", kind => {
  useEditorKind(kind);
  beforeEach(() => { expect(navigator.platform).toBe("MacIntel"); });

  it.each([
    ["Option+Left", { key: "ArrowLeft", code: "ArrowLeft", altKey: true }],
    ["Option+Right", { key: "ArrowRight", code: "ArrowRight", altKey: true }],
    ["Option+Shift+Left", { key: "ArrowLeft", code: "ArrowLeft", altKey: true, shiftKey: true }],
    ["Option+Shift+Up", { key: "ArrowUp", code: "ArrowUp", altKey: true, shiftKey: true }],
    ["Option+Shift+Down", { key: "ArrowDown", code: "ArrowDown", altKey: true, shiftKey: true }],
    // US layout symbols; a non-Latin symbol is matched by its physical letter.
    ["Option+L (¬)", { key: "¬", code: "KeyL", altKey: true }],
    ["Option+K (˚)", { key: "˚", code: "KeyK", altKey: true }],
    ["Option+B (∫)", { key: "∫", code: "KeyB", altKey: true }],
    ["Option+W (∑)", { key: "∑", code: "KeyW", altKey: true }],
    ["Option+D (∂)", { key: "∂", code: "KeyD", altKey: true }],
  ] as const)("%s runs no Tine command: the page, the block and its text stay", (_name, init) => {
    const editor = mount();
    const event = editor.keydown({ ...init });
    expect(routeName()).toBe("Mac keys");
    expect(editor.value()).toBe(TEXT);
    expect(doc.byId[SECOND].raw).toBe(TEXT);
    expect(pageByName("Mac keys")!.roots).toEqual([FIRST, SECOND, THIRD]);
    if (!init.key.startsWith("Arrow")) expect(editor.selection()).toEqual([6, 6]);
    // The textarea leaves the key to the system; CodeMirror runs its own Mac
    // word motion and selection for Option+arrows, which consumes the event.
    if (kind === "textarea") expect(event.defaultPrevented).toBe(false);
  });

  it("Cmd+[ goes back and Cmd+] forward, even mid-edit (Logseq's Mac keys)", () => {
    const editor = mount();
    expect(editor.keydown({ key: "[", code: "BracketLeft", metaKey: true }).defaultPrevented).toBe(true);
    expect(routeName()).toBe("Earlier page");
    expect(editor.keydown({ key: "]", code: "BracketRight", metaKey: true }).defaultPrevented).toBe(true);
    expect(routeName()).toBe("Mac keys");
    expect(editor.keydown({ key: "]", code: "BracketRight", metaKey: true }).defaultPrevented).toBe(true);
    expect(routeName()).toBe("Later page");
    // CodeMirror's own Cmd+[ / Cmd+] (indent less / more) never runs.
    expect(editor.value()).toBe(TEXT);
  });

  it("Control+Shift+B moves back a word, Cmd+Shift+Up moves the block up, and Control+L clears it (Logseq's Mac keys)", async () => {
    const editor = mount();
    editor.keydown({ key: "B", code: "KeyB", ctrlKey: true, shiftKey: true });
    await vi.waitFor(() => expect(editor.selection()).toEqual([0, 0]));
    editor.keydown({ key: "ArrowUp", code: "ArrowUp", metaKey: true, shiftKey: true });
    await vi.waitFor(() => expect(pageByName("Mac keys")!.roots).toEqual([SECOND, FIRST, THIRD]));
    editor.keydown({ key: "l", code: "KeyL", ctrlKey: true });
    await vi.waitFor(() => expect(doc.byId[SECOND].raw).toBe(""));
  });

  it("a user's own Option+Left binding for Back keeps working", () => {
    const editor = mount({ "go/backward": "alt+left" });
    expect(editor.keydown({ key: "ArrowLeft", code: "ArrowLeft", altKey: true }).defaultPrevented).toBe(true);
    expect(routeName()).toBe("Earlier page");
  });
});
