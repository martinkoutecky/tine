import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { PageView } from "./Page";
import { backend } from "../backend";
import { closeSettings, openSettings, settingsOpen } from "../ui";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { resetStore } from "../document";
import { installEdgeSwipe } from "../edgeSwipe";
import { dismissTopTransient } from "../transientLayers";
import { settingEntries } from "./settingsRegistry";
import { setActiveLocale } from "../i18n";

const tick = () => new Promise(resolve => setTimeout(resolve, 0));
let dispose = () => {};
let resize = (_width: number, _touch: boolean) => {};
afterEach(async () => {
  dispose(); closeSettings(); resetStore(); resetTabsToJournals();
  vi.restoreAllMocks(); vi.unstubAllGlobals(); document.body.replaceChildren();
  await setActiveLocale("en");
});
async function mount(width = 390, touch = true) {
  const listeners = new Set<() => void>();
  resize = (nextWidth, nextTouch) => { width = nextWidth; touch = nextTouch; for (const listener of listeners) listener(); };
  vi.stubGlobal("matchMedia", (query: string) => ({
    get matches() {
      const maximum = /max-width:\s*(\d+)px/.exec(query);
      const coarse = query.includes("pointer: coarse");
      return (!!maximum || coarse) && (!maximum || width <= Number(maximum[1])) && (!coarse || touch);
    },
    addEventListener(_type: string, listener: () => void) { listeners.add(listener); },
    removeEventListener(_type: string, listener: () => void) { listeners.delete(listener); },
  }));
  vi.spyOn(backend(), "getPage").mockResolvedValue({ id: "pages/Settings proof.md", name: "Settings proof", title: "Settings proof", kind: "page", pre_block: null,
    blocks: [{ id: "settings-proof", raw: "Routed Settings proof", children: [], collapsed: false }] });
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Settings proof", pageKind: "page" });
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <><PageView /><button onClick={() => openSettings()}>Open Settings</button><Settings /></>, root);
  await vi.waitFor(() => expect(root.textContent).toContain("Routed Settings proof"));
  [...root.querySelectorAll("button")].find(b => b.textContent === "Open Settings")!.click();
  await tick(); return root;
}
function category(root: HTMLElement, label: string) {
  return [...root.querySelectorAll<HTMLButtonElement>(".settings-nav-item")].find(b => b.textContent === label)!;
}
function search(root: HTMLElement, query: string) {
  const input = root.querySelector<HTMLInputElement>(".settings-search-input")!;
  input.value = query; input.dispatchEvent(new InputEvent("input", { bubbles: true }));
}
it("opens a category list over a routed page, drills in, returns and closes", async () => {
  const root = await mount();
  expect(category(root, "Themes")).toBeDefined();
  expect(root.querySelector('[data-setting-id="settings-ui-mode"]')).toBeNull();
  category(root, "Appearance").click(); await tick();
  expect(root.querySelector('[data-setting-id="settings-ui-mode"]')).not.toBeNull();
  expect(root.querySelector(".settings-nav-item")).toBeNull();
  root.querySelector<HTMLButtonElement>('[aria-label="Back to Settings"]')!.click(); await tick();
  expect(category(root, "Themes")).toBeDefined();
  root.querySelector<HTMLButtonElement>('[aria-label="Close Settings"]')!.click();
  expect(settingsOpen()).toBe(false);
  openSettings(); await tick();
  expect(root.querySelector('[data-setting-id="settings-ui-mode"]')).toBeNull();
});
it("reopens the list after closing an active search, and localizes Themes and navigation", async () => {
  const root = await mount();
  search(root, "color scheme"); await tick();
  root.querySelector<HTMLButtonElement>('[aria-label="Close Settings"]')!.click();
  openSettings(); await tick();
  expect(category(root, "Themes")).toBeDefined();
  expect(root.querySelector<HTMLInputElement>(".settings-search-input")!.value).toBe("");
  await setActiveLocale("zh-CN");
  category(root, "主题").click(); await tick();
  expect(root.querySelector('[aria-label="返回设置"]')).not.toBeNull();
  expect(root.querySelector('[aria-label="关闭设置"]')).not.toBeNull();
});
it("search drills straight to a theme control and all theme entries belong to Themes", async () => {
  const root = await mount();
  category(root, "Keyboard shortcuts").click(); await tick();
  root.querySelector<HTMLButtonElement>('[aria-label="Back to Settings"]')!.click(); await tick();
  expect(root.querySelector<HTMLInputElement>(".settings-search-input")!.getAttribute("aria-label")).toBe("Search settings");
  const entries = settingEntries().filter(e => e.id.startsWith("theme.") || e.id.startsWith("themes-"));
  expect(entries.length).toBeGreaterThan(5);
  expect(entries.every(e => e.tab === "themes")).toBe(true);
  search(root, "color scheme"); await tick();
  root.querySelector<HTMLButtonElement>('[data-setting-target="themes-color-scheme"]')!.click(); await tick();
  expect(root.querySelector('[data-setting-id="themes-color-scheme"]')).not.toBeNull();
  expect(root.querySelector(".settings-nav-item")).toBeNull();
  expect(root.querySelector(".settings-pane-head")?.textContent).toContain("Themes");
});
it("the existing iOS recognizer returns to categories, then dismisses Settings without opening a drawer", async () => {
  const root = await mount(); category(root, "Editor").click(); await tick();
  const drawer = vi.fn();
  const stop = installEdgeSwipe({ platform: "ios", backAvailable: settingsOpen, drawerOpenable: () => false,
    back: () => { dismissTopTransient("back"); }, openDrawer: drawer, surface: () => null, viewportWidth: () => 390 });
  const swipe = () => {
    for (const [type, x] of [["touchstart", 5], ["touchmove", 180], ["touchend", 180]] as const) {
      const event = new Event(type, { bubbles: true, cancelable: true });
      const point = { clientX: x, clientY: 100 };
      Object.defineProperties(event, { touches: { value: type === "touchend" ? [] : [point] }, changedTouches: { value: [point] } });
      root.dispatchEvent(event);
    }
  };
  try {
    swipe(); await tick(); expect(settingsOpen()).toBe(true); expect(category(root, "Themes")).toBeDefined();
    swipe(); expect(settingsOpen()).toBe(false); expect(drawer).not.toHaveBeenCalled();
  } finally { stop(); }
});
it.each([[1280, false], [820, true], [390, false]])("retains nav and pane together at width %s with touch=%s", async (width, touch) => {
  const root = await mount(width as number, touch as boolean);
  expect(category(root, "Appearance")).toBeDefined();
  expect(root.querySelector('[data-setting-id="settings-ui-mode"]')).not.toBeNull();
});
it("responds to width and input changes without losing the phone navigation level", async () => {
  const root = await mount();
  resize(820, true); await tick();
  expect(category(root, "Themes")).toBeDefined();
  expect(root.querySelector('[data-setting-id="settings-ui-mode"]')).not.toBeNull();
  resize(390, true); await tick();
  expect(root.querySelector('[data-setting-id="settings-ui-mode"]')).toBeNull();
  category(root, "Themes").click(); await tick();
  resize(1280, false); await tick();
  expect(category(root, "Themes")).toBeDefined();
  expect(root.querySelector('[data-setting-id="themes-style"]')).not.toBeNull();
  resize(390, true); await tick();
  expect(root.querySelector(".settings-nav-item")).toBeNull();
  expect(root.querySelector('[data-setting-id="themes-style"]')).not.toBeNull();
});
