// GH #996, #1007: Settings can be searched without the mouse. Opening Settings
// puts the caret in its search box (desktop, when no block is being edited), and
// Up, Down and Enter move through the matching settings the way they do in a
// Search tab (GH #697): the results take real focus, Down from the box focuses
// the first, the ends do not wrap, Up on the first result returns to the box,
// and Enter is the focused result's own activation, exactly as a click.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { Editor } from "./Block";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { closeSettings, openSettings, settingsOpen } from "../ui";
import { initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { hasSelection, resetStore, selectBlock, selectedIds } from "../document";
import { installKeybindings } from "../keybindings";
import { editingId, endEdit, startEditing } from "../editorController";
import { editorIn, type EditorKind } from "../tests/editorDriver";
import { backend } from "../backend";
import * as panes from "../panes";

vi.mock("../panes", async (original) => ({
  ...(await original<typeof import("../panes")>()),
  layoutHasMultiplePanes: vi.fn(() => false),
  moveActiveTabInDirection: vi.fn(),
}));

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
  vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.mocked(panes.layoutHasMultiplePanes).mockClear();
  vi.mocked(panes.moveActiveTabInDirection).mockClear(); document.body.replaceChildren();
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
async function type(root: HTMLElement, query: string) {
  box(root).focus();
  box(root).value = query;
  box(root).dispatchEvent(new InputEvent("input", { bubbles: true }));
  await tick();
}
function press(key: string, init: KeyboardEventInit = {}): KeyboardEvent {
  const target = document.activeElement as HTMLElement;
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(event);
  return event;
}
/** What the browser does with Enter on a focused button the page left alone: it clicks it. */
function pressEnter() {
  const event = press("Enter");
  if (!event.defaultPrevented && document.activeElement instanceof HTMLButtonElement) document.activeElement.click();
}
/**
 * The result that looks current, which must be the one Enter opens: the focused
 * result button, or none while the caret is in the box. No other result carries
 * a highlight or an accessibility pointer of its own.
 */
function current(root: HTMLElement): HTMLButtonElement | null {
  expect(root.querySelectorAll(".settings-search-result.active").length).toBe(0);
  expect(box(root).hasAttribute("aria-activedescendant")).toBe(false);
  const focused = document.activeElement;
  if (focused === box(root)) return null;
  expect(results(root)).toContain(focused);
  return focused as HTMLButtonElement;
}
const activeTab = (root: HTMLElement) => root.querySelector<HTMLElement>(".settings-nav-item.active")?.dataset.settingsTab;
const navTo = (root: HTMLElement, tab: string) => root.querySelector<HTMLElement>(`.settings-nav-item[data-settings-tab="${tab}"]`)!.click();

describe("Settings search from the keyboard (GH #996, #1007)", () => {
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

  it("Down focuses the first result, Down and Up move focus one result without wrapping, and Up on the first returns to the box", async () => {
    const root = mount();
    openSettings(); await tick();
    await type(root, "theme");
    const rows = results(root);
    expect(rows.length).toBeGreaterThan(2);
    expect(current(root)).toBeNull();
    const scrolled = vi.spyOn(HTMLElement.prototype, "scrollIntoView");
    expect(press("ArrowDown").defaultPrevented).toBe(true);
    expect(current(root)).toBe(rows[0]);
    expect(scrolled.mock.contexts.at(-1)).toBe(rows[0]);
    press("ArrowDown");
    expect(current(root)).toBe(rows[1]);
    expect(scrolled.mock.contexts.at(-1)).toBe(rows[1]);
    press("ArrowUp");
    expect(current(root)).toBe(rows[0]);
    press("ArrowUp");
    expect(current(root)).toBeNull();
    // No wrap at the top: Up in the box stays in the box (the caret is the box's).
    expect(press("ArrowUp").defaultPrevented).toBe(false);
    expect(current(root)).toBeNull();
    // No wrap at the bottom: Down on the last result stays there.
    for (const _ of rows) press("ArrowDown");
    expect(current(root)).toBe(rows.at(-1));
    expect(press("ArrowDown").defaultPrevented).toBe(true);
    expect(current(root)).toBe(rows.at(-1));
  });

  it("Enter on the focused result opens the same tab and setting as clicking it, and the caret ends in the box", async () => {
    const root = mount();
    openSettings(); await tick();
    await type(root, "theme");
    const clicked = results(root)[1];
    const id = clicked.dataset.settingTarget!;
    press("ArrowDown"); press("ArrowDown");
    expect(current(root)).toBe(clicked);
    pressEnter();
    await tick();
    const viaEnter = { tab: activeTab(root), shown: !!root.querySelector(`[data-setting-id="${id}"]`), query: box(root).value };
    expect(viaEnter).toEqual({ tab: viaEnter.tab, shown: true, query: "" });
    expect(document.activeElement).toBe(box(root));

    await type(root, "theme");
    results(root).find((row) => row.dataset.settingTarget === id)!.click();
    await tick();
    expect({ tab: activeTab(root), shown: !!root.querySelector(`[data-setting-id="${id}"]`), query: box(root).value }).toEqual(viaEnter);
  });

  it("after Down twice, Tab back to the first result: only that result looks current, and Enter opens it", async () => {
    const root = mount();
    openSettings(); await tick();
    await type(root, "theme");
    const rows = results(root);
    press("ArrowDown"); press("ArrowDown");
    expect(current(root)).toBe(rows[1]);
    // Shift+Tab (or Tab from the box) moves the browser's focus; the page does not see a key it handles.
    expect(press("Tab", { shiftKey: true }).defaultPrevented).toBe(false);
    rows[0].focus();
    expect(current(root)).toBe(rows[0]);
    const id = rows[0].dataset.settingTarget!;
    pressEnter();
    await tick();
    expect(root.querySelector(`[data-setting-id="${id}"]`)).not.toBeNull();
    expect(box(root).value).toBe("");
  });

  it("typing again leaves no result current: Down then starts at the new first result", async () => {
    const root = mount();
    openSettings(); await tick();
    await type(root, "theme");
    press("ArrowDown"); press("ArrowDown");
    expect(current(root)).toBe(results(root)[1]);
    await type(root, "font");
    expect(current(root)).toBeNull();
    const rows = results(root);
    expect(rows.length).toBeGreaterThan(0);
    press("ArrowDown");
    expect(current(root)).toBe(rows[0]);
  });

  it("switching to Keyboard shortcuts and back with the query kept shows no result highlighted", async () => {
    const root = mount();
    openSettings(); await tick();
    await type(root, "theme");
    press("ArrowDown"); press("ArrowDown");
    navTo(root, "shortcuts"); await tick();
    expect(root.querySelector(".settings-search-results")).toBeNull();
    navTo(root, "appearance"); await tick();
    expect(box(root).value).toBe("theme");
    expect(results(root).length).toBeGreaterThan(2);
    expect(root.querySelectorAll(".settings-search-result.active").length).toBe(0);
    expect(box(root).hasAttribute("aria-activedescendant")).toBe(false);
    expect(results(root)).not.toContain(document.activeElement);
  });

  it("modified arrows stay with the global shortcuts: Mod+Alt+Down and Mod+Alt+Shift+Down run their pane command once and Settings moves nothing", async () => {
    cleanup.push(installKeybindings());
    const root = mount();
    openSettings(); await tick();
    await type(root, "theme");
    const panesAsked = vi.mocked(panes.layoutHasMultiplePanes).mock.calls.length;
    press("ArrowDown", { ctrlKey: true, altKey: true });
    expect(vi.mocked(panes.layoutHasMultiplePanes).mock.calls.length - panesAsked).toBe(1);
    expect(current(root)).toBeNull();
    press("ArrowDown", { ctrlKey: true, altKey: true, shiftKey: true });
    expect(vi.mocked(panes.moveActiveTabInDirection).mock.calls.map((call) => call[1])).toEqual(["down"]);
    expect(current(root)).toBeNull();
    // From a focused result too.
    press("ArrowDown");
    const first = current(root);
    press("ArrowUp", { ctrlKey: true, altKey: true, shiftKey: true });
    expect(vi.mocked(panes.moveActiveTabInDirection).mock.calls.map((call) => call[1])).toEqual(["down", "up"]);
    expect(current(root)).toBe(first);
  });

  it("no outline shortcut runs from these keys: a block selection behind Settings stays as it was", async () => {
    loadSingle({ name: "P", kind: "page", title: "P", id: "pages/P.md", rev: "r1", format: "md", pre_block: null,
      blocks: [{ id: "a", raw: "one", children: [], collapsed: false }, { id: "b", raw: "two", children: [], collapsed: false }] });
    selectBlock("a");
    cleanup.push(installKeybindings());
    const root = mount();
    openSettings(); await tick();
    await type(root, "theme");
    expect(hasSelection()).toBe(true);
    press("ArrowDown"); press("ArrowDown"); press("ArrowUp"); press("ArrowDown");
    expect(selectedIds()).toEqual(["a"]);
    press("ArrowUp"); press("ArrowUp");
    expect(current(root)).toBeNull();
    press("Enter");
    expect(selectedIds()).toEqual(["a"]);
    press("ArrowDown"); pressEnter(); await tick();
    expect(selectedIds()).toEqual(["a"]);
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
