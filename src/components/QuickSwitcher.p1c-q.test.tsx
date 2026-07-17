import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { QuickSwitcher } from "./QuickSwitcher";
import { installKeybindings } from "../keybindings";
import { clearTransientLayersForTest, registerTransientLayer, topTransientLayer } from "../transientLayers";
import { installMobileDrawerMode, mobileDrawerMode, takeDrawerOpener } from "../mobileDrawers";
import {
  activeDrawer,
  closeSwitcher,
  openSwitcher,
  setLeftSidebarOpen,
  setRightSidebarOpen,
  switcherEmbryo,
  switcherOpen,
} from "../ui";
import { layoutPaneIds, resetPaneLayoutToSingle, splitPane } from "../panes";

type Mounted = { root: HTMLDivElement; dispose: () => void };
let disposeMobileMode: (() => void) | undefined;
let matchMediaDescriptor: PropertyDescriptor | undefined;

function mountSwitcher(): Mounted {
  const root = document.createElement("div");
  document.body.append(root);
  return { root, dispose: render(() => <QuickSwitcher />, root) };
}

function escape(init: { composing?: boolean; keyCode?: number } = {}) {
  const event = new KeyboardEvent("keydown", {
    key: "Escape",
    code: "Escape",
    bubbles: true,
    cancelable: true,
  });
  if (init.composing) Object.defineProperty(event, "isComposing", { value: true });
  if (init.keyCode != null) Object.defineProperty(event, "keyCode", { value: init.keyCode });
  return event;
}

async function settle() {
  await Promise.resolve();
  await Promise.resolve();
}

function openSyntax(root: HTMLElement) {
  const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
  const syntax = root.querySelector<HTMLButtonElement>(".switcher-syntax-toggle")!;
  syntax.click();
  expect(root.querySelector("#switcher-search-syntax")).not.toBeNull();
  return input;
}

function installMobileDrawerHarness() {
  matchMediaDescriptor = Object.getOwnPropertyDescriptor(globalThis, "matchMedia");
  const query = {
    matches: true,
    media: "(max-width: 639px)",
    onchange: null,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    dispatchEvent() { return true; },
  } as unknown as MediaQueryList;
  Object.defineProperty(globalThis, "matchMedia", { configurable: true, value: () => query });
  disposeMobileMode = installMobileDrawerMode();
}

afterEach(() => {
  disposeMobileMode?.();
  disposeMobileMode = undefined;
  if (matchMediaDescriptor) Object.defineProperty(globalThis, "matchMedia", matchMediaDescriptor);
  else delete (globalThis as { matchMedia?: typeof matchMedia }).matchMedia;
  matchMediaDescriptor = undefined;
  closeSwitcher();
  setLeftSidebarOpen(false);
  setRightSidebarOpen(false);
  takeDrawerOpener();
  clearTransientLayersForTest();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

describe("GH #161 P1C-Q QuickSwitcher Escape ladder", () => {
  it("peels mounted syntax then the switcher-owned embryo before the lower transient", async () => {
    const lower = vi.fn(() => true);
    registerTransientLayer({ id: "quick-switcher-lower", dismiss: lower });
    const embryo = splitPane("main", "row")!;
    const mounted = mountSwitcher();
    openSwitcher({ mode: "embryo", paneId: embryo });
    await settle();
    const input = openSyntax(mounted.root);
    const uninstall = installKeybindings();

    const syntaxEscape = escape();
    input.dispatchEvent(syntaxEscape);
    await settle();
    expect(syntaxEscape.defaultPrevented).toBe(true);
    expect(mounted.root.querySelector("#switcher-search-syntax")).toBeNull();
    expect(switcherOpen()).toBe(true);
    expect(switcherEmbryo()?.paneId).toBe(embryo);
    expect(layoutPaneIds()).toContain(embryo);
    expect(lower).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(input);

    const baseEscape = escape();
    input.dispatchEvent(baseEscape);
    await settle();
    expect(baseEscape.defaultPrevented).toBe(true);
    expect(switcherOpen()).toBe(false);
    expect(switcherEmbryo()).toBeNull();
    expect(layoutPaneIds()).not.toContain(embryo);
    expect(lower).not.toHaveBeenCalled();

    const lowerEscape = escape();
    window.dispatchEvent(lowerEscape);
    expect(lowerEscape.defaultPrevented).toBe(true);
    expect(lower).toHaveBeenCalledTimes(1);
    uninstall();
    mounted.dispose();
  });

  it("keeps a real open mobile drawer below syntax and base, restoring to its live target only after unmount", async () => {
    installMobileDrawerHarness();
    expect(mobileDrawerMode()).toBe(true);
    const drawerShell = document.createElement("div");
    drawerShell.dataset.activeDrawer = "left";
    const drawer = document.createElement("aside");
    drawer.className = "left-sidebar";
    const drawerTarget = document.createElement("button");
    drawerTarget.textContent = "Drawer target";
    drawer.append(drawerTarget);
    drawerShell.append(drawer);
    document.body.append(drawerShell);
    setLeftSidebarOpen(true);
    expect(activeDrawer()).toBe("left");

    const embryo = splitPane("main", "row")!;
    const mounted = mountSwitcher();
    openSwitcher({ mode: "embryo", paneId: embryo });
    await settle();
    const input = openSyntax(mounted.root);
    const uninstall = installKeybindings();

    input.dispatchEvent(escape());
    await settle();
    expect(activeDrawer()).toBe("left");
    expect(document.activeElement).toBe(input);

    input.dispatchEvent(escape());
    await settle();
    expect(switcherOpen()).toBe(false);
    expect(layoutPaneIds()).not.toContain(embryo);
    expect(activeDrawer()).toBe("left");
    expect(drawerTarget.isConnected).toBe(true);
    expect(document.activeElement).toBe(drawerTarget);
    uninstall();
    mounted.dispose();
  });

  it("declines composing and keyCode-229 Escape at the mounted input without changing any rung", async () => {
    const lower = vi.fn(() => true);
    registerTransientLayer({ id: "quick-switcher-composition-lower", dismiss: lower });
    const embryo = splitPane("main", "row")!;
    const mounted = mountSwitcher();
    openSwitcher({ mode: "embryo", paneId: embryo });
    await settle();
    const input = openSyntax(mounted.root);
    input.value = "preserve this query";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    input.focus();
    const uninstall = installKeybindings();

    for (const ignored of [escape({ composing: true }), escape({ keyCode: 229 })]) {
      input.dispatchEvent(ignored);
      await settle();
      expect(ignored.defaultPrevented).toBe(false);
      expect(mounted.root.querySelector("#switcher-search-syntax")).not.toBeNull();
      expect(switcherOpen()).toBe(true);
      expect(switcherEmbryo()?.paneId).toBe(embryo);
      expect(layoutPaneIds()).toContain(embryo);
      expect(input.value).toBe("preserve this query");
      expect(document.activeElement).toBe(input);
      expect(lower).not.toHaveBeenCalled();
    }
    uninstall();
    mounted.dispose();
  });

  it("without the global listener, gives a newer transient the first input Escape before the switcher", async () => {
    const embryo = splitPane("main", "row")!;
    const mounted = mountSwitcher();
    openSwitcher({ mode: "embryo", paneId: embryo });
    await settle();
    const input = openSyntax(mounted.root);
    const newer = vi.fn();
    const unregisterNewer = registerTransientLayer({
      id: "quick-switcher-newer-owner",
      dismiss: () => { newer(); unregisterNewer(); return true; },
    });

    const first = escape();
    input.dispatchEvent(first);
    await settle();
    expect(first.defaultPrevented).toBe(true);
    expect(newer).toHaveBeenCalledTimes(1);
    expect(mounted.root.querySelector("#switcher-search-syntax")).not.toBeNull();
    expect(switcherOpen()).toBe(true);
    expect(switcherEmbryo()?.paneId).toBe(embryo);
    expect(layoutPaneIds()).toContain(embryo);

    const second = escape();
    input.dispatchEvent(second);
    await settle();
    expect(second.defaultPrevented).toBe(true);
    expect(mounted.root.querySelector("#switcher-search-syntax")).toBeNull();
    expect(switcherOpen()).toBe(true);
    expect(switcherEmbryo()?.paneId).toBe(embryo);
    mounted.dispose();
  });

  it("cleans the visible registration and embryo ownership across ordinary close and reopen", async () => {
    const lower = vi.fn(() => true);
    registerTransientLayer({ id: "quick-switcher-cleanup-lower", dismiss: lower });
    const embryo = splitPane("main", "row")!;
    const mounted = mountSwitcher();
    const uninstall = installKeybindings();
    openSwitcher({ mode: "embryo", paneId: embryo });
    await settle();
    const input = mounted.root.querySelector<HTMLInputElement>(".switcher-input")!;
    input.dispatchEvent(escape());
    await settle();
    expect(switcherOpen()).toBe(false);
    expect(switcherEmbryo()).toBeNull();
    expect(layoutPaneIds()).not.toContain(embryo);
    expect(topTransientLayer()?.id).toBe("quick-switcher-cleanup-lower");

    openSwitcher();
    await settle();
    expect(switcherOpen()).toBe(true);
    expect(switcherEmbryo()).toBeNull();
    closeSwitcher();
    await settle();
    expect(topTransientLayer()?.id).toBe("quick-switcher-cleanup-lower");
    const later = escape();
    window.dispatchEvent(later);
    expect(later.defaultPrevented).toBe(true);
    expect(lower).toHaveBeenCalledTimes(1);
    uninstall();
    mounted.dispose();
  });
});
