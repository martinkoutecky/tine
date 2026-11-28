// GH #1007: Settings can be searched without the mouse. Opening Settings puts the
// caret in its search box (desktop, when no block is being edited); while a block
// is being edited, focus and the edit stay where they were.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { Editor } from "./Block";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { closeSettings, openSettings, settingsOpen } from "../ui";
import { initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { hasSelection, pageRoots, resetStore, selectBlock, selectedIds } from "../document";
import { installKeybindings } from "../keybindings";
import { editingId, endEdit, startEditing } from "../editorController";
import { editorIn, type EditorKind } from "../tests/editorDriver";
import { backend } from "../backend";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
let cleanup: (() => void)[] = [];

beforeAll(() => initParser());
beforeEach(() => {
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
});
afterEach(() => {
  for (const undo of cleanup.splice(0).reverse()) undo();
  closeSettings(); endEdit("page-navigation"); resetStore();
  vi.restoreAllMocks(); vi.unstubAllGlobals(); document.body.replaceChildren();
});

/** Desktop unless `phone`: the phone layout is a narrow, touch-primary window. */
function mount(phone = false) {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: phone && (query.includes("max-width") || query.includes("pointer: coarse")),
    addEventListener() {}, removeEventListener() {},
  }));
  const root = document.createElement("div");
  document.body.append(root);
  cleanup.push(render(() => <Settings />, root));
  return root;
}
const box = (root: HTMLElement) => root.querySelector<HTMLInputElement>(".settings-search-input")!;
const results = (root: HTMLElement) => [...root.querySelectorAll<HTMLButtonElement>(".settings-search-result")];
/** Types into whatever has focus, as a keyboard does: no focus call first. */
async function typeIntoFocused(query: string) {
  const input = document.activeElement as HTMLInputElement;
  input.value = query;
  input.dispatchEvent(new InputEvent("input", { bubbles: true }));
  await tick();
}
function press(key: string, init: KeyboardEventInit = {}): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
  (document.activeElement as HTMLElement).dispatchEvent(event);
  return event;
}
const activeTab = (root: HTMLElement) => root.querySelector<HTMLElement>(".settings-nav-item.active")?.dataset.settingsTab;

describe("Settings search from the keyboard (GH #1007)", () => {
  it("opening Settings puts the caret in the search box, also when it opens on a tab or on a setting", async () => {
    const root = mount();
    openSettings(); await tick();
    expect(document.activeElement).toBe(box(root));
    closeSettings(); await tick();
    openSettings("editor"); await tick();
    expect(activeTab(root)).toBe("editor");
    expect(document.activeElement).toBe(box(root));
    closeSettings(); await tick();
    openSettings("toolbar"); await tick();
    expect(activeTab(root)).toBe("appearance");
    expect(root.querySelector('[data-setting-id="toolbar-title"]')).not.toBeNull();
    expect(document.activeElement).toBe(box(root));
  });

  it("typing right after opening, with no click, shows the matching settings", async () => {
    const root = mount();
    openSettings(); await tick();
    await typeIntoFocused("Wide mode");
    expect(box(root).value).toBe("Wide mode");
    expect(results(root).map((row) => row.dataset.settingTarget)).toContain("settings-ui-wide-mode");
  });

  it("Escape from the box clears a query, then closes Settings", async () => {
    cleanup.push(installKeybindings());
    const root = mount();
    openSettings(); await tick();
    box(root).focus();
    await typeIntoFocused("theme");
    press("Escape"); await tick();
    expect(settingsOpen()).toBe(true);
    expect(box(root).value).toBe("");
    press("Escape"); await tick();
    expect(settingsOpen()).toBe(false);
  });

  it("the caret lands in the box also when the dialog first waits for the platform probe, as the app mounts it", async () => {
    vi.resetModules();
    const { render: mountFresh } = await import("solid-js/web");
    const { Show, Suspense } = await import("solid-js");
    const { backend } = await import("../backend");
    const fresh = await import("./Settings");
    const ui = await import("../ui");
    let answer!: (kind: "desktop") => void;
    vi.spyOn(backend(), "appPlatform").mockReturnValue(new Promise((resolve) => { answer = resolve; }));
    vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
    const root = document.createElement("div");
    document.body.append(root);
    // The app's own wrapping: Settings mounts per open, inside a Suspense boundary.
    cleanup.push(mountFresh(() => <Show when={ui.settingsOpen()}><Suspense><fresh.Settings /></Suspense></Show>, root));
    cleanup.push(() => ui.closeSettings());
    ui.openSettings(); await tick();
    answer("desktop");
    await vi.waitFor(() => expect(box(root)).not.toBeNull());
    await tick();
    expect(document.activeElement).toBe(box(root));
  });

  it("on a phone, opening Settings leaves the focus alone so the keyboard does not cover the category list", async () => {
    const root = mount(true);
    openSettings(); await tick();
    expect(root.querySelector(".settings-nav-item")).not.toBeNull();
    expect(document.activeElement).not.toBe(box(root));
  });

  it("keys typed in the box leave a block selection behind Settings and its page unchanged", async () => {
    loadSingle({ name: "P", kind: "page", title: "P", id: "pages/P.md", rev: "r1", format: "md", pre_block: null,
      blocks: [{ id: "a", raw: "one", children: [], collapsed: false }, { id: "b", raw: "two", children: [], collapsed: false }] });
    const page = () => pageRoots("P").map((id) => doc.byId[id]?.raw);
    selectBlock("a");
    cleanup.push(installKeybindings());
    const root = mount();
    openSettings(); await tick();
    // A guard that holds with or without the caret placed on open: put it in the box as a click does.
    box(root).focus();
    await typeIntoFocused("theme");
    expect(hasSelection()).toBe(true);
    for (const [key, init] of [["Backspace", {}], ["Delete", {}], ["ArrowDown", {}], ["ArrowDown", { ctrlKey: true, altKey: true }]] as const) {
      press(key, init); await tick();
      expect(selectedIds(), key).toEqual(["a"]);
      expect(page(), key).toEqual(["one", "two"]);
      expect(document.activeElement, key).toBe(box(root));
    }
  });

  describe.each<EditorKind>(["textarea", "cm6"])("while a block is being edited (%s editor)", (kind) => {
    it("opening Settings keeps the edit and its focus, and Escape returns to the still-open editor", async () => {
      vi.spyOn(backend(), "applySpellcheck").mockResolvedValue(undefined);
      cleanup.push(installKeybindings());
      loadSingle({ name: "E", title: "E", kind: "page", pre_block: null,
        blocks: [{ id: "edited", raw: "typing here", collapsed: false, children: [] }] });
      startEditing("edited", 6);
      const host = document.createElement("div"); document.body.append(host);
      cleanup.push(render(() => kind === "cm6" ? <Cm6BlockEditor id="edited" surface="main" scope={null} /> : <Editor id="edited" />, host));
      await tick();
      const editor = editorIn(host);
      editor.focus();
      expect(editor.element.contains(document.activeElement)).toBe(true);
      const focusedBefore = document.activeElement;
      const root = mount();
      openSettings(); await tick();
      expect(settingsOpen()).toBe(true);
      expect(document.activeElement).toBe(focusedBefore);
      expect(document.activeElement).not.toBe(box(root));
      expect(editingId()).toBe("edited");
      press("Escape"); await tick();
      expect(settingsOpen()).toBe(false);
      expect(editingId()).toBe("edited");
      expect(editorIn(host).element.isConnected).toBe(true);
      expect(document.activeElement).toBe(focusedBefore);
      expect(editorIn(host).selection()).toEqual([6, 6]);
    });
  });
});
