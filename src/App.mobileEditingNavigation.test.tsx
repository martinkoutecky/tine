import { maybeEditorIn } from "./tests/editorDriver";
import { afterEach, beforeAll, describe, expect, it, onTestFinished, vi } from "vitest";
import { render } from "solid-js/web";
import { createEffect, createRoot, createSignal } from "solid-js";

// GH #598 (D-20261019-N2): with "Navigation buttons at the bottom" on, the keyboard
// hides the bottom row, so the editing toolbar carries Sidebar / Search / Back /
// Forward at its leading end. Real App, real Block editor, real router; the keyboard
// is simulated by a visual-viewport resize as in MobileKeyboardToolbar.test.tsx.
vi.hoisted(() => { globalThis.__TINE_PLATFORM__ = "android"; });
vi.mock("./inputCapability", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./inputCapability")>()),
  touchPrimary: () => true,
  touchCapable: () => true,
}));
import { App } from "./App";
import { backend } from "./backend";
import { initParser } from "./render/parse";
import { pageByName, resetStore } from "./document";
import { endEdit, editingId, startEditing } from "./editorController";
import { bindTestHost, submittedPages } from "./document/host/wiring.test.support";
import { resetPaneLayoutToSingle } from "./panes";
import { mainPaneRouter, route } from "./router";
import { resetMobileNavigationAtBottomForTests } from "./navSettings";
import { firstLoadDone, setFirstLoadDone } from "./graphSession";

beforeAll(async () => { await initParser(); await bindTestHost(); });

let disposeApp: (() => void) | undefined;
afterEach(() => {
  disposeApp?.();
  disposeApp = undefined;
  endEdit("blur");
  resetStore();
  resetMobileNavigationAtBottomForTests(false);
  vi.restoreAllMocks();
  document.body.innerHTML = "";
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
});

function keyboardUp() {
  Object.defineProperty(window, "innerHeight", { value: 800, configurable: true });
  Object.defineProperty(window, "visualViewport", {
    value: { height: 520, offsetTop: 0, addEventListener: vi.fn(), removeEventListener: vi.fn() },
    configurable: true,
  });
}

function pointer(type: string): Event {
  const Ctor = (window as { PointerEvent?: typeof PointerEvent }).PointerEvent ?? MouseEvent;
  const event = new Ctor(type, { bubbles: true, cancelable: true, clientX: 5, clientY: 5, button: 0 });
  Object.defineProperty(event, "pointerId", { value: 7, configurable: true });
  Object.defineProperty(event, "isPrimary", { value: true, configurable: true });
  return event;
}

/** Observe domain and DOM conditions; the test's timeout still detects a hang. */
function until<T>(host: HTMLElement, read: () => T | null): Promise<T> {
  return new Promise(resolve => {
    let stop = () => {};
    const cleanup = () => { observer.disconnect(); stop(); };
    const check = () => {
      const value = read();
      if (value !== null) { cleanup(); resolve(value); }
    };
    const observer = new MutationObserver(check);
    onTestFinished(cleanup);
    observer.observe(host, { childList: true, subtree: true, attributes: true });
    createRoot(dispose => { stop = dispose; createEffect(check); });
  });
}

/** Observe the actual save submissions without changing the backend's replies. */
function observeSubmissions() {
  const [revision, setRevision] = createSignal(0);
  const save = backend().pageSubmit.bind(backend());
  const submit = vi.spyOn(backend(), "pageSubmit").mockImplementation((...args) => {
    const reply = save(...args);
    setRevision(value => value + 1);
    return reply;
  });
  return () => { revision(); return submittedPages(submit, "Tine").at(-1); };
}

/** A page "Tine" with one block, reached from the journals (so Back has somewhere to go). */
async function openTine(bottom: boolean) {
  keyboardUp();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  const host = document.createElement("div");
  document.body.appendChild(host);
  setFirstLoadDone(false);
  const unmount = render(() => <App />, host);
  const dispose = () => { unmount(); disposeApp = undefined; };
  disposeApp = dispose;
  await vi.dynamicImportSettled();
  // Finish graph startup before choosing the page and seeding the preference;
  // rendering the unrelated journal feed is not a toolbar prerequisite.
  await until(host, () => firstLoadDone() ? true : null);
  resetMobileNavigationAtBottomForTests(bottom);
  mainPaneRouter.openPage("Tine", "page");
  expect(route()).toMatchObject({ kind: "page", name: "Tine" });
  const id = await until(host, () => {
    const first = pageByName("Tine")?.roots[0];
    return first && host.querySelector(`[data-block-id="${first}"]`) ? first : null;
  });
  startEditing(id, 0);
  const textarea = await until(host, () => maybeEditorIn(host));
  textarea.element.focus();
  const toolbar = await until(host, () => host.querySelector<HTMLElement>("[data-mobile-keyboard-toolbar]:not([hidden])"));
  // jsdom reports no document focus; the real WebView has it, and a blur then ends the edit.
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  return { host, dispose, textarea, toolbar, id };
}

const NAV = ["Toggle sidebar (t l)", "Go back", "Go forward"];

describe("navigation buttons in the editing toolbar (GH #598)", { timeout: 30_000 }, () => {
  it("with the setting on, the toolbar shows Sidebar, Search, Back and Forward before the formatting buttons", async () => {
    const { host, dispose, toolbar } = await openTine(true);
    try {
      const nav = toolbar.querySelector<HTMLElement>(".mobile-keyboard-toolbar-nav")!;
      expect(nav, "the leading navigation group").not.toBeNull();
      expect(toolbar.firstElementChild).toBe(nav);
      for (const title of NAV) expect(nav.querySelector(`button[title="${title}"]`), title).not.toBeNull();
      expect(nav.querySelector("[data-search-trigger]")).not.toBeNull();
      expect(nav.closest(".mobile-keyboard-toolbar-strip")).toBeNull();
      expect(toolbar.querySelector('.mobile-keyboard-toolbar-strip button[aria-label="Outdent"]')).not.toBeNull();
    } finally { dispose(); host.remove(); }
  });

  it("with the setting off, the toolbar is unchanged and the buttons stay in the top bar", async () => {
    const { host, dispose, toolbar } = await openTine(false);
    try {
      expect(toolbar.querySelector(".mobile-keyboard-toolbar-nav")).toBeNull();
      expect(host.querySelector("header.topbar [data-search-trigger]")).not.toBeNull();
      expect(host.querySelectorAll("[data-search-trigger]")).toHaveLength(1);
    } finally { dispose(); host.remove(); }
  });

  it("types, taps Back in the toolbar: the text is saved, the edit ends, and the previous page opens", async () => {
    const lastSubmit = observeSubmissions();
    const { host, dispose, textarea, toolbar, id } = await openTine(true);
    try {
      textarea.input("typed on the phone", undefined, { eventKind: "plain" });
      const back = toolbar.querySelector<HTMLButtonElement>('.mobile-keyboard-toolbar-nav button[title="Go back"]')!;
      expect(back.disabled).toBe(false);
      expect(route()).toMatchObject({ kind: "page", name: "Tine" });

      const down = pointer("pointerdown");
      back.dispatchEvent(down);
      expect(down.defaultPrevented).toBe(true);
      back.dispatchEvent(pointer("pointerup"));

      expect(editingId(), "the edit ended").toBeNull();
      await until(host, () => route().kind === "journals" ? route() : null);
      expect(route(), "Back navigated").toMatchObject({ kind: "journals" });
      expect(pageByName("Tine")!.roots[0]).toBe(id);
      await until(host, () => lastSubmit()?.blocks[0].raw === "typed on the phone" ? true : null);
      expect(lastSubmit()?.blocks[0].raw).toBe("typed on the phone");
    } finally { dispose(); host.remove(); }
  });

  it("tapping Search ends the edit through the same save path and opens the switcher", async () => {
    const lastSubmit = observeSubmissions();
    const { host, dispose, textarea, toolbar } = await openTine(true);
    try {
      textarea.input("search me", undefined, { eventKind: "plain" });
      const search = toolbar.querySelector<HTMLButtonElement>(".mobile-keyboard-toolbar-nav [data-search-trigger]")!;
      search.dispatchEvent(pointer("pointerdown"));
      search.dispatchEvent(pointer("pointerup"));
      await until(host, () => host.querySelector(".switcher-overlay"));
      expect(editingId()).toBeNull();
      expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Tine" });
      await until(host, () => lastSubmit()?.blocks[0].raw === "search me" ? true : null);
      expect(lastSubmit()?.blocks[0].raw).toBe("search me");
    } finally { dispose(); host.remove(); }
  });
});
