import { Show } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  DrawerBackground,
  MobileDrawerController,
  MobileDrawerPanel,
  dismissDrawerAndRestore,
} from "./components/MobileDrawerShell";
import {
  activeDrawer,
  rightSidebarOpen,
  setLeftSidebarOpen,
  setRightSidebarOpen,
  sidebarOpen,
} from "./ui";
import { mobileDrawerMode, takeDrawerOpener } from "./mobileDrawers";
import { clearTransientLayersForTest, registerTransientLayer } from "./transientLayers";

type MediaListener = EventListenerOrEventListenerObject;

let cleanup: (() => void) | undefined;

function restoreStorage(key: string, value: string | null) {
  if (value == null) localStorage.removeItem(key);
  else localStorage.setItem(key, value);
}

async function settle() {
  await Promise.resolve();
  await Promise.resolve();
}

function isInert(element: HTMLElement) {
  return element.inert === true || element.hasAttribute("inert");
}

function installHarness(initialMatches = true) {
  const leftStorage = localStorage.getItem("logseq-claude.sidebarOpen");
  const rightStorage = localStorage.getItem("logseq-claude.rightSidebarOpen");
  const matchMediaDescriptor = Object.getOwnPropertyDescriptor(globalThis, "matchMedia");
  let matches = initialMatches;
  let listener: MediaListener | undefined;
  const query = {
    get matches() { return matches; },
    media: "(max-width: 639px)",
    onchange: null,
    addEventListener(type: string, value: MediaListener) {
      if (type === "change") listener = value;
    },
    removeEventListener(type: string, value: MediaListener) {
      if (type === "change" && listener === value) listener = undefined;
    },
    addListener() {},
    removeListener() {},
    dispatchEvent() { return true; },
  } as unknown as MediaQueryList;
  Object.defineProperty(globalThis, "matchMedia", { configurable: true, value: () => query });

  setLeftSidebarOpen(false);
  setRightSidebarOpen(false);

  const root = document.createElement("div");
  document.body.append(root);
  const underClick = vi.fn();
  const dispose = render(() => (
    <div class="app-container" data-mobile-drawer-mode={mobileDrawerMode() ? "true" : "false"} data-active-drawer={activeDrawer() ?? ""}>
      <Show when={sidebarOpen()}>
        <MobileDrawerPanel side="left" label="Navigation sidebar" class="left-sidebar">
          <button class="left-first" onClick={() => dismissDrawerAndRestore("explicit")}>Close left</button>
          <button class="left-last">Last left action</button>
        </MobileDrawerPanel>
      </Show>
      <DrawerBackground class="main-container" blockedBy="left">
        <DrawerBackground class="top-region" blockedBy="right">
          <button class="under-target" onClick={underClick}>Underlying action</button>
        </DrawerBackground>
        <DrawerBackground class="drawer-workspace" blockedBy="right">
          <main class="main-content pane-focused" tabIndex={-1}>Main</main>
        </DrawerBackground>
        <Show when={rightSidebarOpen()}>
          <MobileDrawerPanel side="right" label="Reference sidebar" class="right-sidebar">
            <button class="right-first" onClick={() => dismissDrawerAndRestore("explicit")}>Close right</button>
            <button class="right-last">Last right action</button>
          </MobileDrawerPanel>
        </Show>
      </DrawerBackground>
      <MobileDrawerController />
      <DrawerBackground class="test-mobile-toolbar-region" blockedBy="any">
        <button class="test-mobile-toolbar-action">Mobile editor action</button>
      </DrawerBackground>
      <DrawerBackground class="test-toast-region" blockedBy="any">
        <button class="test-toast-dismiss">Dismiss notification</button>
      </DrawerBackground>
    </div>
  ), root);

  const changeWidth = async (next: boolean) => {
    matches = next;
    const event = new Event("change");
    if (typeof listener === "function") listener.call(query, event);
    else listener?.handleEvent(event);
    await settle();
  };

  const teardown = () => {
    dispose();
    setLeftSidebarOpen(false);
    setRightSidebarOpen(false);
    takeDrawerOpener();
    clearTransientLayersForTest();
    root.remove();
    restoreStorage("logseq-claude.sidebarOpen", leftStorage);
    restoreStorage("logseq-claude.rightSidebarOpen", rightStorage);
    if (matchMediaDescriptor) Object.defineProperty(globalThis, "matchMedia", matchMediaDescriptor);
    else delete (globalThis as { matchMedia?: typeof matchMedia }).matchMedia;
  };
  cleanup = teardown;
  return { root, underClick, changeWidth, teardown };
}

afterEach(() => {
  cleanup?.();
  cleanup = undefined;
  vi.restoreAllMocks();
});

describe("GH #161 mounted responsive drawer shell", () => {
  it("normalizes compact exclusivity but preserves simultaneous persistent sidebars", async () => {
    const harness = installHarness(true);
    await settle();
    expect(mobileDrawerMode()).toBe(true);

    const compactOpener = document.createElement("button");
    document.body.append(compactOpener);
    setLeftSidebarOpen(true, compactOpener);
    expect(activeDrawer()).toBe("left");
    setRightSidebarOpen(true);
    expect(sidebarOpen()).toBe(false);
    expect(rightSidebarOpen()).toBe(true);
    expect(activeDrawer()).toBe("right");
    setLeftSidebarOpen(true);
    expect(sidebarOpen()).toBe(true);
    expect(rightSidebarOpen()).toBe(false);
    expect(activeDrawer()).toBe("left");

    await harness.changeWidth(false);
    expect(mobileDrawerMode()).toBe(false);
    expect(takeDrawerOpener()).toBeNull();
    setLeftSidebarOpen(true);
    setRightSidebarOpen(true);
    expect(sidebarOpen()).toBe(true);
    expect(rightSidebarOpen()).toBe(true);
    expect(activeDrawer()).toBeNull();
    expect(harness.root.querySelectorAll("[data-mobile-drawer-scrim]")).toHaveLength(0);

    await harness.changeWidth(true);
    expect(mobileDrawerMode()).toBe(true);
    expect(sidebarOpen()).toBe(false);
    expect(rightSidebarOpen()).toBe(true);
    expect(activeDrawer()).toBe("right");
    compactOpener.remove();
    harness.teardown();
    cleanup = undefined;
  });

  it("mounts one modal panel, isolates every ordinary region, traps focus, and consumes the scrim", async () => {
    const harness = installHarness(true);
    const opener = document.createElement("button");
    opener.textContent = "Left opener";
    document.body.prepend(opener);
    setLeftSidebarOpen(true, opener);
    await settle();

    const left = harness.root.querySelector<HTMLElement>(".left-sidebar")!;
    const main = harness.root.querySelector<HTMLElement>(".main-container")!;
    const first = harness.root.querySelector<HTMLButtonElement>(".left-first")!;
    const last = harness.root.querySelector<HTMLButtonElement>(".left-last")!;
    expect(left.getAttribute("role")).toBe("dialog");
    expect(left.getAttribute("aria-modal")).toBe("true");
    expect(left.getAttribute("aria-label")).toBe("Navigation sidebar");
    expect(isInert(left)).toBe(false);
    expect(isInert(main)).toBe(true);
    expect(isInert(harness.root.querySelector<HTMLElement>(".test-mobile-toolbar-region")!)).toBe(true);
    expect(isInert(harness.root.querySelector<HTMLElement>(".test-toast-region")!)).toBe(true);
    expect(harness.root.querySelectorAll("[data-mobile-drawer-scrim]")).toHaveLength(1);
    expect(left.contains(document.activeElement)).toBe(true);

    left.focus();
    const rootBackwards = new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true });
    left.dispatchEvent(rootBackwards);
    expect(rootBackwards.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(last);
    left.focus();
    const rootForwards = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
    left.dispatchEvent(rootForwards);
    expect(rootForwards.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(first);

    first.focus();
    const backwards = new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true });
    first.dispatchEvent(backwards);
    expect(backwards.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(last);
    const forwards = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
    last.dispatchEvent(forwards);
    expect(forwards.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(first);

    const inertLocal = harness.root.querySelector<HTMLButtonElement>(".under-target")!;
    const unregisterLocal = registerTransientLayer({ id: "test-inert-local", root: () => inertLocal, dismiss: () => true });
    inertLocal.focus();
    expect(left.contains(document.activeElement)).toBe(true);
    unregisterLocal();

    const transient = document.createElement("button");
    document.body.append(transient);
    const unregister = registerTransientLayer({ id: "test-higher-modal", root: () => transient, dismiss: () => true });
    transient.focus();
    expect(document.activeElement).toBe(transient);
    unregister();
    first.focus();
    transient.focus();
    expect(left.contains(document.activeElement)).toBe(true);

    const scrim = harness.root.querySelector<HTMLElement>("[data-mobile-drawer-scrim]")!;
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    scrim.dispatchEvent(click);
    await settle();
    expect(click.defaultPrevented).toBe(true);
    expect(harness.underClick).not.toHaveBeenCalled();
    expect(activeDrawer()).toBeNull();
    expect(isInert(main)).toBe(false);
    expect(harness.root.querySelectorAll("[data-mobile-drawer-scrim]")).toHaveLength(0);
    expect(document.activeElement).toBe(harness.root.querySelector(".main-content"));

    opener.remove();
    transient.remove();
    harness.teardown();
    cleanup = undefined;
  });

  it("closes the drawer on a horizontal swipe toward its closed edge, but leaves vertical scrolls, short drags, and opening-ward drags alone (GH: Martin)", async () => {
    const harness = installHarness(true);
    const opener = document.createElement("button");
    document.body.append(opener);

    const makeTouch = (type: string, x: number, y: number) => {
      const event = new Event(type, { bubbles: true, cancelable: true });
      const point = { clientX: x, clientY: y };
      Object.defineProperty(event, "touches", { value: type === "touchend" ? [] : [point] });
      Object.defineProperty(event, "changedTouches", { value: [point] });
      return event;
    };
    const swipe = async (from: [number, number], to: [number, number]) => {
      const panel = harness.root.querySelector<HTMLElement>(".left-sidebar")!;
      panel.dispatchEvent(makeTouch("touchstart", from[0], from[1]));
      panel.dispatchEvent(makeTouch("touchmove", to[0], to[1]));
      panel.dispatchEvent(makeTouch("touchend", to[0], to[1]));
      await settle();
    };

    // Horizontal drag left (toward the left drawer's closed edge) closes it.
    setLeftSidebarOpen(true, opener);
    await settle();
    expect(activeDrawer()).toBe("left");
    await swipe([220, 300], [120, 300]);
    expect(activeDrawer()).toBeNull();

    // A vertical drag (scroll) must NOT close the drawer.
    setLeftSidebarOpen(true, opener);
    await settle();
    await swipe([220, 300], [220, 200]);
    expect(activeDrawer()).toBe("left");

    // A short horizontal drag (below the close threshold) must NOT close it.
    await swipe([220, 300], [196, 300]);
    expect(activeDrawer()).toBe("left");

    // An opening-ward drag (rightward, away from the left edge) must NOT close it.
    await swipe([220, 300], [320, 300]);
    expect(activeDrawer()).toBe("left");

    dismissDrawerAndRestore("explicit");
    await settle();
    opener.remove();
    harness.teardown();
    cleanup = undefined;
  });

  it("uses the same semantics for the right drawer and restores or safely falls back by reason", async () => {
    const harness = installHarness(true);
    const connected = document.createElement("button");
    document.body.prepend(connected);
    setRightSidebarOpen(true, connected);
    await settle();

    const right = harness.root.querySelector<HTMLElement>(".right-sidebar")!;
    expect(right.getAttribute("role")).toBe("dialog");
    expect(right.getAttribute("aria-modal")).toBe("true");
    expect(isInert(harness.root.querySelector<HTMLElement>(".top-region")!)).toBe(true);
    expect(isInert(harness.root.querySelector<HTMLElement>(".drawer-workspace")!)).toBe(true);
    expect(isInert(harness.root.querySelector<HTMLElement>(".main-container")!)).toBe(false);
    expect(isInert(harness.root.querySelector<HTMLElement>(".test-mobile-toolbar-region")!)).toBe(true);
    expect(isInert(harness.root.querySelector<HTMLElement>(".test-toast-region")!)).toBe(true);
    harness.root.querySelector<HTMLButtonElement>(".right-first")!.click();
    await settle();
    expect(document.activeElement).toBe(connected);

    setLeftSidebarOpen(true, connected);
    await settle();
    connected.remove();
    expect(dismissDrawerAndRestore("explicit")).toBe(true);
    await settle();
    expect(document.activeElement).toBe(harness.root.querySelector(".main-content"));

    await harness.changeWidth(false);
    setLeftSidebarOpen(true);
    setRightSidebarOpen(true);
    await settle();
    for (const panel of harness.root.querySelectorAll<HTMLElement>("[data-mobile-drawer-panel]")) {
      expect(panel.hasAttribute("role")).toBe(false);
      expect(panel.hasAttribute("aria-modal")).toBe(false);
    }
    for (const background of harness.root.querySelectorAll<HTMLElement>("[data-drawer-background]")) {
      expect(isInert(background)).toBe(false);
    }
    harness.teardown();
    cleanup = undefined;
  });
});
