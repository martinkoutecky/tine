import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// The keymap reads the platform once, when it loads: be a Mac from the start.
vi.hoisted(() => Object.defineProperty(navigator, "platform", { configurable: true, value: "MacIntel" }));

import { For } from "solid-js";
import { render } from "solid-js/web";
import { maybeEditorIn, type EditorDriver, type EditorKind } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";
import { initParser } from "../render/parse";
import { blockIsGridView, emptyPage, pageByName, resetStore, selectBlock, topSelected } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { editingId, endEdit, startEditing } from "../editorController";
import { installKeybindings } from "../keybindings";
import type { ShortcutBinding } from "../shortcutBindings";
import * as router from "../router";
import { cellSel } from "../sheet/selection";
import { Block } from "./Block";
import { ContentWidthFields } from "./ContentWidthFields";

// GH #1040: on a Mac, Option with a left/right arrow or a letter is the text
// system's (word motion, typed symbols such as ¬ or ∑). Option+Left/Right keep
// going Back/Forward everywhere except in a field with a text caret, where the
// field moves by a word; Cmd+[ / Cmd+] (Logseq's Mac keys) go Back/Forward
// everywhere. The Emacs motions are on Logseq's Control chords. Move block keeps
// Option+Shift+Up/Down, so Cmd+Shift+Up/Down (select to the start or end) stays
// with the editor.

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
const optionLeft = { key: "ArrowLeft", code: "ArrowLeft", altKey: true };
const optionRight = { key: "ArrowRight", code: "ArrowRight", altKey: true };

/** History on both sides of "Mac keys", so Back and Forward each have somewhere to go. */
function visitPages() {
  router.openPage("Earlier page", "page");
  router.openPage("Mac keys", "page");
  router.openPage("Later page", "page");
  router.goBack();
}

function show(view: () => ReturnType<typeof Block>): HTMLElement {
  const root = document.createElement("div");
  document.body.appendChild(root);
  disposeView = render(view, root);
  return root;
}

function mount(overrides: Record<string, ShortcutBinding> = {}): EditorDriver {
  disposeKeys = installKeybindings(overrides);
  loadSingle({ name: "Mac keys", kind: "page", title: "Mac keys", pre_block: null, blocks: [
    { id: FIRST, raw: "first", collapsed: false, children: [] },
    { id: SECOND, raw: TEXT, collapsed: false, children: [] },
    { id: THIRD, raw: "third", collapsed: false, children: [] },
  ] });
  visitPages();
  startEditing(SECOND, 6);
  const root = show(() => <For each={pageByName("Mac keys")?.roots ?? []}>{id => <Block id={id} />}</For>);
  const editor = maybeEditorIn(root, SECOND)!;
  editor.focus();
  editor.select(6, 6);
  return editor;
}

function press(target: EventTarget, init: KeyboardEventInit): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(event);
  return event;
}

const routeName = () => (router.route() as { name?: string }).name;

describe.each<EditorKind>(["textarea", "cm6"])("GH #1040 macOS Option keys stay with the editor (%s)", kind => {
  useEditorKind(kind);
  beforeEach(() => { expect(navigator.platform).toBe("MacIntel"); });

  it.each([
    ["Option+Left", optionLeft],
    ["Option+Right", optionRight],
    ["Option+Shift+Left", { ...optionLeft, shiftKey: true }],
    ["Cmd+Shift+Up", { key: "ArrowUp", code: "ArrowUp", metaKey: true, shiftKey: true }],
    ["Cmd+Shift+Down", { key: "ArrowDown", code: "ArrowDown", metaKey: true, shiftKey: true }],
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
    // word motion and selection for these arrows, which consumes the event.
    if (kind === "textarea") expect(event.defaultPrevented).toBe(false);
  });

  it("Option+Left and Option+Right move the caret by a word in a block (the report)", () => {
    const editor = mount();
    const left = editor.keydown(optionLeft);
    expect(routeName()).toBe("Mac keys");
    // CodeMirror moves the caret itself; the textarea's move is the system's, so
    // the key must reach it unprevented.
    if (kind === "cm6") expect(editor.selection()).toEqual([0, 0]);
    else expect(left.defaultPrevented).toBe(false);
    const right = editor.keydown(optionRight);
    expect(routeName()).toBe("Mac keys");
    if (kind === "cm6") expect(editor.selection()).toEqual([5, 5]);
    else expect(right.defaultPrevented).toBe(false);
    expect(doc.byId[SECOND].raw).toBe(TEXT);
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

  it("Option+Shift+Up and Option+Shift+Down move the block, as on every platform", async () => {
    const editor = mount();
    editor.keydown({ key: "ArrowUp", code: "ArrowUp", altKey: true, shiftKey: true });
    await vi.waitFor(() => expect(pageByName("Mac keys")!.roots).toEqual([SECOND, FIRST, THIRD]));
    editor.keydown({ key: "ArrowDown", code: "ArrowDown", altKey: true, shiftKey: true });
    await vi.waitFor(() => expect(pageByName("Mac keys")!.roots).toEqual([FIRST, SECOND, THIRD]));
    editor.keydown({ key: "ArrowDown", code: "ArrowDown", altKey: true, shiftKey: true });
    await vi.waitFor(() => expect(pageByName("Mac keys")!.roots).toEqual([FIRST, THIRD, SECOND]));
    expect(doc.byId[SECOND].raw).toBe(TEXT);
  });

  it("Control+Shift+B moves back a word and Control+L clears the block (Logseq's Mac keys)", async () => {
    const editor = mount();
    editor.keydown({ key: "B", code: "KeyB", ctrlKey: true, shiftKey: true });
    await vi.waitFor(() => expect(editor.selection()).toEqual([0, 0]));
    editor.keydown({ key: "l", code: "KeyL", ctrlKey: true });
    await vi.waitFor(() => expect(doc.byId[SECOND].raw).toBe(""));
  });

  it("a user's own Option+Left binding for Back keeps working", () => {
    const editor = mount({ "go/backward": "alt+left" });
    expect(editor.keydown(optionLeft).defaultPrevented).toBe(true);
    expect(routeName()).toBe("Earlier page");
  });

  it("the Edit code language button inside an editor is no text field: Option+Left goes Back", () => {
    disposeKeys = installKeybindings();
    loadSingle({ name: "Mac keys", kind: "page", title: "Mac keys", pre_block: null, blocks: [
      { id: FIRST, raw: "```js\none\n```", collapsed: false, children: [] },
    ] });
    visitPages();
    const root = show(() => <Block id={FIRST} />);
    root.querySelector("pre.code-block > code")!.dispatchEvent(new MouseEvent("mousedown", { button: 0, bubbles: true, cancelable: true }));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    const button = root.querySelector<HTMLButtonElement>("button.code-language")!;
    expect(button.closest("[data-block-id]")).not.toBeNull();
    button.focus();
    expect(press(button, optionLeft).defaultPrevented).toBe(true);
    expect(routeName()).toBe("Earlier page");
  });
});

describe("GH #1040 macOS Option+Left/Right outside a block editor", () => {
  function install(overrides: Record<string, ShortcutBinding> = {}) {
    disposeKeys = installKeybindings(overrides);
    visitPages();
  }
  function field(type: string | null): HTMLInputElement {
    const input = document.createElement("input");
    if (type !== null) input.setAttribute("type", type);
    document.body.appendChild(input);
    input.focus();
    return input;
  }

  it.each([null, "text", "search", "url", "tel", "email", "password", "number"])(
    "an input of type %s has a text caret: Option+Left/Right reach it and the page stays", type => {
      install();
      const input = field(type);
      expect(press(input, optionLeft).defaultPrevented).toBe(false);
      expect(press(input, optionRight).defaultPrevented).toBe(false);
      expect(routeName()).toBe("Mac keys");
    });

  it("a textarea and a contenteditable element have a text caret", () => {
    install();
    const area = document.body.appendChild(document.createElement("textarea"));
    const editable = document.body.appendChild(document.createElement("div"));
    editable.setAttribute("contenteditable", "true");
    const inner = editable.appendChild(document.createElement("span"));
    for (const target of [area, editable, inner]) expect(press(target, optionLeft).defaultPrevented).toBe(false);
    expect(routeName()).toBe("Mac keys");
  });

  it.each(["range", "checkbox", "radio", "button", "color", "file"])(
    "an input of type %s has no text caret: Option+Left goes Back (as on develop)", type => {
      install();
      expect(press(field(type), optionLeft).defaultPrevented).toBe(true);
      expect(routeName()).toBe("Earlier page");
    });

  it("Settings' page width: the number field keeps Option+Left, the slider beside it goes Back", () => {
    install();
    show(() => <ContentWidthFields />);
    const number = document.querySelector<HTMLInputElement>("input.settings-width-number")!;
    const slider = document.querySelector<HTMLInputElement>("input.settings-width-range")!;
    number.focus();
    expect(press(number, optionLeft).defaultPrevented).toBe(false);
    expect(routeName()).toBe("Mac keys");
    slider.focus();
    expect(press(slider, optionLeft).defaultPrevented).toBe(true);
    expect(routeName()).toBe("Earlier page");
  });

  it("with nothing focused, Option+Left goes Back and Option+Right Forward (a layout with no typable Cmd+[)", () => {
    install();
    // A German Mac layout sends the arrows unchanged; its "[" needs Option+5.
    expect(press(document.body, optionLeft).defaultPrevented).toBe(true);
    expect(routeName()).toBe("Earlier page");
    expect(press(document.body, optionRight).defaultPrevented).toBe(true);
    expect(routeName()).toBe("Mac keys");
  });

  it("a selected grid block: Option+Right goes Forward and opens no cell", () => {
    install();
    loadSingle({ ...emptyPage("Mac keys", "page"), blocks: [
      { id: FIRST, raw: "Grid\ntine.view:: grid", properties: [["tine.view", "grid"]], collapsed: false, children: [
        { id: SECOND, raw: "", collapsed: false, children: [{ id: THIRD, raw: "Alpha", collapsed: false, children: [] }] },
      ] },
    ] });
    show(() => <Block id={FIRST} />);
    selectBlock(FIRST);
    expect(blockIsGridView(FIRST)).toBe(true);
    expect(topSelected()).toEqual([FIRST]);
    expect(press(document.body, optionRight).defaultPrevented).toBe(true);
    expect(routeName()).toBe("Later page");
    expect(cellSel()).toBeNull();
    expect(editingId()).toBeNull();
  });

  it("a Journals key saved on Cmd+[ opens Journals: Back's Cmd+[ is only an alias", () => {
    install({ "go/journals": "mod+[" });
    expect(press(document.body, { key: "[", code: "BracketLeft", metaKey: true }).defaultPrevented).toBe(true);
    expect(router.route().kind).toBe("journals");
  });

  it("Option+Left in a text field clears a pending two-key sequence, as Back did on develop", () => {
    install({ "go/journals": "ctrl+j ctrl+k" });
    const input = field("text");
    const ctrl = (key: string) => ({ key, code: `Key${key.toUpperCase()}`, ctrlKey: true });
    // The sequence works in this field without the arrow in between …
    press(input, ctrl("j"));
    press(input, ctrl("k"));
    expect(router.route().kind).toBe("journals");
    router.goBack();
    expect(routeName()).toBe("Mac keys");
    // … and Option+Left between its keys ends it.
    press(input, ctrl("j"));
    expect(press(input, optionLeft).defaultPrevented).toBe(false);
    press(input, ctrl("k"));
    expect(routeName()).toBe("Mac keys");
  });
});
