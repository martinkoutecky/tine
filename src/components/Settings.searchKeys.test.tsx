// GH #996, #1007: Settings can be searched without the mouse. Opening Settings
// puts the caret in its search box (desktop), and Up, Down and Enter move
// through the matching settings the way they do in a Search tab (GH #697): Down
// from the box enters the list, the ends do not wrap, Up on the first result
// returns to the box, and Enter opens a result exactly as a click does.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";
import { initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { hasSelection, resetStore, selectBlock, selectedIds } from "../document";
import { installKeybindings } from "../keybindings";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
let cleanup: (() => void)[] = [];

beforeAll(() => initParser());
afterEach(() => {
  for (const undo of cleanup.splice(0).reverse()) undo();
  closeSettings(); resetStore();
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
async function type(root: HTMLElement, query: string) {
  box(root).focus();
  box(root).value = query;
  box(root).dispatchEvent(new InputEvent("input", { bubbles: true }));
  await tick();
}
function press(key: string): KeyboardEvent {
  const target = document.activeElement as HTMLElement;
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
  target.dispatchEvent(event);
  return event;
}
/** The result the keyboard is on, as assistive technology is told it; the caret never leaves the box. */
function current(root: HTMLElement): HTMLButtonElement | null {
  expect(document.activeElement).toBe(box(root));
  const id = box(root).getAttribute("aria-activedescendant");
  const marked = root.querySelectorAll(".settings-search-result.active");
  if (!id) { expect(marked.length).toBe(0); return null; }
  const row = root.querySelector<HTMLButtonElement>(`[id="${id}"]`)!;
  expect([...marked]).toEqual([row]);
  return row;
}
const activeTab = (root: HTMLElement) => root.querySelector<HTMLElement>(".settings-nav-item.active")?.dataset.settingsTab;

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

  it("Down enters the results, Down and Up step one result without wrapping, and Up on the first returns to the box", async () => {
    const root = mount();
    openSettings(); await tick();
    await type(root, "theme");
    const rows = results(root);
    expect(rows.length).toBeGreaterThan(2);
    expect(box(root).getAttribute("aria-controls")).toBe(root.querySelector(".settings-search-results")!.id);
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

  it("Enter on the current result opens the same tab and setting as clicking it, and the caret ends in the box", async () => {
    const root = mount();
    openSettings(); await tick();
    await type(root, "theme");
    const clicked = results(root)[1];
    const id = clicked.dataset.settingTarget!;
    press("ArrowDown"); press("ArrowDown");
    expect(current(root)).toBe(clicked);
    expect(press("Enter").defaultPrevented).toBe(true);
    await tick();
    const viaEnter = { tab: activeTab(root), shown: !!root.querySelector(`[data-setting-id="${id}"]`), query: box(root).value };
    expect(viaEnter).toEqual({ tab: viaEnter.tab, shown: true, query: "" });
    expect(document.activeElement).toBe(box(root));

    await type(root, "theme");
    results(root).find((row) => row.dataset.settingTarget === id)!.click();
    await tick();
    expect({ tab: activeTab(root), shown: !!root.querySelector(`[data-setting-id="${id}"]`), query: box(root).value }).toEqual(viaEnter);
  });

  it("typing again resets the current result: Down then starts at the new first result", async () => {
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
    press("ArrowDown"); press("Enter"); await tick();
    expect(selectedIds()).toEqual(["a"]);
  });
});
