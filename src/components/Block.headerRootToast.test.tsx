// GH #210 F1 follow-up (UI-GH210-HEADER-MOVE-TOAST, D-20261019-H210F2): the
// keys a user presses on the block under a page's properties. The kernel
// refuses to put a block above them or under them; the key now says why (an
// info toast) instead of doing nothing. The mobile toolbar's move and indent
// buttons dispatch the same editor commands, so these are their path too.
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { beginPageHeaderEdit, pageByName, resetStore } from "../document";
import { doc } from "../document/model";
import { loadSingle } from "../document/workingSet";
import { endEdit, startEditing } from "../editorController";
import { installKeybindings } from "../keybindings";
import { setToasts, toasts } from "../toasts";
import { Block } from "./Block";

beforeAll(() => initParser());
let disposeKeys: () => void;
beforeEach(() => { setToasts([]); disposeKeys = installKeybindings(); });
afterEach(() => { disposeKeys(); endEdit("page-navigation"); resetStore(); setToasts([]); document.body.innerHTML = ""; });

const leaf = (id: string, raw: string) => ({ id, raw, collapsed: false, children: [] });
const WHY = "Page properties stay at the top of the page";

function mount() {
  loadSingle({ name: "Reading list", kind: "page", title: "Reading list", pre_block: "alias:: book\ntags:: reading", format: "md", blocks: [leaf("o1", "one"), leaf("o2", "two")] });
  const header = beginPageHeaderEdit("Reading list")!;
  endEdit("page-navigation");
  startEditing("o1", 3);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <For each={pageByName("Reading list")!.roots}>{(id) => <Block id={id} />}</For>, root);
  const editor = root.querySelector<HTMLElement>('[data-block-id="o1"] textarea.block-editor') as HTMLTextAreaElement;
  editor.setSelectionRange(3, 3);
  return { header, editor, dispose };
}

describe("keys that would put a block above or under the page properties", () => {
  it("Alt+Shift+Up on the first block under the properties says why and changes nothing", () => {
    const { header, editor, dispose } = mount();
    try {
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", altKey: true, shiftKey: true, bubbles: true, cancelable: true }));
      expect(toasts().map((t) => [t.kind, t.message])).toEqual([["info", WHY]]);
      expect(pageByName("Reading list")!.roots).toEqual([header, "o1", "o2"]);
    } finally { dispose(); }
  });

  it("Tab on the first block under the properties says why and does not nest it under them", () => {
    const { header, editor, dispose } = mount();
    try {
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
      expect(toasts().map((t) => [t.kind, t.message])).toEqual([["info", "A block can't be indented under the page properties"]]);
      expect(pageByName("Reading list")!.roots).toEqual([header, "o1", "o2"]);
      expect(doc.byId[header].children).toEqual([]);
    } finally { dispose(); }
  });

  it("Alt+Shift+Down on a later block moves it with no toast", () => {
    const { header, editor, dispose } = mount();
    try {
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", altKey: true, shiftKey: true, bubbles: true, cancelable: true }));
      expect(toasts()).toEqual([]);
      expect(pageByName("Reading list")!.roots).toEqual([header, "o2", "o1"]);
    } finally { dispose(); }
  });
});
