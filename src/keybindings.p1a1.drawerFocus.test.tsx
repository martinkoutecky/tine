import { afterEach, describe, expect, it } from "vitest";
import {
  installMobileDrawerMode,
  mobileDrawerMode,
  takeDrawerOpener,
} from "./mobileDrawers";
import { installKeybindings } from "./keybindings";
import {
  activeDrawer,
  setLeftSidebarOpen,
  setRightSidebarOpen,
} from "./ui";

type MediaListener = EventListenerOrEventListenerObject;

let activeTeardown: (() => void) | undefined;

function escape() {
  return new KeyboardEvent("keydown", {
    key: "Escape",
    code: "Escape",
    bubbles: true,
    cancelable: true,
  });
}

function restoreStorage(key: string, value: string | null) {
  if (value === null) localStorage.removeItem(key);
  else localStorage.setItem(key, value);
}

function installDrawerHarness() {
  const leftStorage = localStorage.getItem("logseq-claude.sidebarOpen");
  const rightStorage = localStorage.getItem("logseq-claude.rightSidebarOpen");
  const matchMediaDescriptor = Object.getOwnPropertyDescriptor(globalThis, "matchMedia");
  let matches = true;
  let change: MediaListener | undefined;
  const query = {
    get matches() { return matches; },
    media: "(max-width: 639px)",
    onchange: null,
    addEventListener(type: string, listener: MediaListener) {
      if (type === "change") change = listener;
    },
    removeEventListener(type: string, listener: MediaListener) {
      if (type === "change" && change === listener) change = undefined;
    },
    addListener() {},
    removeListener() {},
    dispatchEvent() { return true; },
  } as unknown as MediaQueryList;
  Object.defineProperty(globalThis, "matchMedia", {
    configurable: true,
    value: () => query,
  });
  const disposeMobileMode = installMobileDrawerMode();
  const uninstall = installKeybindings();

  setLeftSidebarOpen(false);
  setRightSidebarOpen(false);
  expect(mobileDrawerMode()).toBe(true);
  expect(activeDrawer()).toBeNull();

  let tornDown = false;
  const teardown = () => {
    if (tornDown) return;
    tornDown = true;
      // This happens while the production classifier callback remains installed:
      // it proves the reactive width transition and cleanup path.
      matches = false;
      if (typeof change === "function") change.call(query, new Event("change"));
      else change?.handleEvent(new Event("change"));
      expect(mobileDrawerMode()).toBe(false);

      uninstall();
      setLeftSidebarOpen(false);
      setRightSidebarOpen(false);
      takeDrawerOpener();
      restoreStorage("logseq-claude.sidebarOpen", leftStorage);
      restoreStorage("logseq-claude.rightSidebarOpen", rightStorage);
      disposeMobileMode();
      if (matchMediaDescriptor) Object.defineProperty(globalThis, "matchMedia", matchMediaDescriptor);
      else delete (globalThis as { matchMedia?: typeof matchMedia }).matchMedia;
      document.body.innerHTML = "";
  };
  activeTeardown = teardown;
  return { teardown };
}

afterEach(() => {
  // A failed assertion must not leave module-level drawer state, storage, or
  // owned DOM behind for the neighboring render tests.
  activeTeardown?.();
  activeTeardown = undefined;
});

describe("GH #161 P1A1 drawer Escape focus preservation", () => {
  it("closes a real left drawer and restores focus to its connected opener", () => {
    const harness = installDrawerHarness();
    const opener = document.createElement("button");
    const drawerTarget = document.createElement("div");
    drawerTarget.tabIndex = -1;
    document.body.append(opener, drawerTarget);
    opener.focus();

    setLeftSidebarOpen(true, opener);
    expect(activeDrawer()).toBe("left");
    drawerTarget.focus();
    expect(document.activeElement).toBe(drawerTarget);
    expect(document.activeElement).not.toBe(opener);

    const event = escape();
    drawerTarget.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(activeDrawer()).toBeNull();
    expect(document.activeElement).toBe(opener);
    harness.teardown();
  });

  it("uses the real main-content fallback when the captured opener is disconnected", () => {
    const harness = installDrawerHarness();
    const opener = document.createElement("button");
    const drawerTarget = document.createElement("div");
    drawerTarget.tabIndex = -1;
    const main = document.createElement("main");
    main.className = "main-content";
    main.tabIndex = -1;
    document.body.append(opener, drawerTarget, main);
    opener.focus();

    setLeftSidebarOpen(true, opener);
    expect(activeDrawer()).toBe("left");
    opener.remove();
    drawerTarget.focus();
    expect(document.activeElement).toBe(drawerTarget);

    const event = escape();
    drawerTarget.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(activeDrawer()).toBeNull();
    expect(document.activeElement).not.toBe(opener);
    expect(document.activeElement).toBe(main);
    harness.teardown();
  });
});
