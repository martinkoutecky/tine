import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { Block, OutlineScopeContext, SurfaceContext } from "./Block";
import { loadSingle } from "../document/workingSet";
import { pageByName, resetStore, selectedIds, selectionMarkdown, type OutlineScope } from "../document";
import { endEdit } from "../editorController";
import { initParser } from "../render/parse";
import { installKeybindings } from "../keybindings";
import { editorIn } from "../tests/editorDriver";
import type { BlockDto } from "../types";

beforeAll(initParser);
afterEach(() => { endEdit("page-navigation"); resetStore(); vi.restoreAllMocks(); document.body.replaceChildren(); });
function mount(surface: string, folded = false, zoom = false) {
  const b = (id: string, children: BlockDto[] = []): BlockDto => ({ id, raw: id, children, collapsed: folded && id === "B" });
  loadSingle({ name: "Units", title: "Units", kind: "page", pre_block: null,
    blocks: [b("A"), b("B", [b("B1", [b("B1a")])]), b("D", [b("D1")]), b("E")] });
  const host = document.createElement("div"); document.body.append(host);
  const scope: OutlineScope = { roots: zoom ? ["B"] : [...pageByName("Units")!.roots], ...(zoom ? { forceExpandedRoot: "B" } : {}) };
  const dispose = render(() => <SurfaceContext.Provider value={surface}><OutlineScopeContext.Provider value={scope}>
    <For each={scope.roots}>{id => <Block id={id} />}</For>
  </OutlineScopeContext.Provider></SurfaceContext.Provider>, host);
  return { host, dispose };
}
function press(host: HTMLElement, id: string, shiftKey = false) {
  host.querySelector(`[data-block-id="${id}"] > .block-main .block-content-wrapper`)!
    .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, shiftKey, clientX: 30 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
}
describe("subtree gestures in both editors (GH #941)", () => {
  for (const surface of ["main", "sidebar", "split"]) for (const folded of [false, true]) {
    it(`${surface}, folded=${folded}: first Shift+Down selects B; second takes D and its children`, () => {
      const keys = installKeybindings(), { host, dispose } = mount(surface, folded);
      try {
        press(host, "B"); const editor = editorIn(host); editor.select(editor.value().length);
        editor.keydown({ key: "ArrowDown", shiftKey: true });
        expect(selectedIds()).toEqual(["B", "B1", "B1a"]);
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", shiftKey: true, bubbles: true }));
        expect(selectedIds()).toEqual(["B", "B1", "B1a", "D", "D1"]);
        expect(selectionMarkdown()).toBe("- B\n\t- B1\n\t\t- B1a\n- D\n\t- D1");
      } finally { dispose(); keys(); }
    });
    it(`${surface}, folded=${folded}: Shift-click includes the endpoint's whole subtree`, () => {
      const { host, dispose } = mount(surface, folded);
      try { press(host, "A"); press(host, "B", true); expect(selectedIds()).toEqual(["A", "B", "B1", "B1a"]); }
      finally { dispose(); }
    });
    it(`${surface}, folded=${folded}: dragging across a parent includes its descendants`, () => {
      const { host, dispose } = mount(surface, folded);
      const content = (id: string) => host.querySelector(`[data-block-id="${id}"] > .block-main .block-content-wrapper`)!;
      try {
        content("A").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, clientX: 30, clientY: 30 }));
        Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => content("B") });
        document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 30, clientY: 90 }));
        expect(selectedIds()).toEqual(["A", "B", "B1", "B1a"]);
        document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      } finally { Reflect.deleteProperty(document, "elementFromPoint"); dispose(); }
    });
  }
  it("first Shift+Up selects the current block; another takes the preceding sibling subtree", () => {
    const keys = installKeybindings(), { host, dispose } = mount("main");
    try {
      press(host, "E"); const editor = editorIn(host); editor.select(0);
      editor.keydown({ key: "ArrowUp", shiftKey: true });
      expect(selectedIds()).toEqual(["E"]);
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", shiftKey: true, bubbles: true }));
      expect(selectedIds()).toEqual(["D", "D1", "E"]);
    } finally { dispose(); keys(); }
  });
  it("a zoomed folded root is selected completely and cannot extend outside the zoom", () => {
    const keys = installKeybindings(), { host, dispose } = mount("main", true, true);
    try {
      press(host, "B"); const editor = editorIn(host); editor.select(editor.value().length);
      editor.keydown({ key: "ArrowDown", shiftKey: true });
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", shiftKey: true, bubbles: true }));
      expect(selectedIds()).toEqual(["B", "B1", "B1a"]);
    } finally { dispose(); keys(); }
  });
});
