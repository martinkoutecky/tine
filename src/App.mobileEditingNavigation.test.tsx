import { maybeEditorIn } from "./tests/editorDriver";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

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

beforeAll(async () => { await initParser(); await bindTestHost(); });

afterEach(() => {
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

/** A page "Tine" with one block, reached from the journals (so Back has somewhere to go). */
async function openTine(bottom: boolean) {
  keyboardUp();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <App />, host);
  // After mount: App's startup preference load would overwrite an earlier seed.
  await vi.waitFor(() => expect(host.querySelector(".journal-title")).not.toBeNull());
  resetMobileNavigationAtBottomForTests(bottom);
  mainPaneRouter.openPage("Tine", "page");
  await vi.waitFor(() => expect(route()).toMatchObject({ kind: "page", name: "Tine" }));
  const id = await vi.waitFor(() => { const roots = pageByName("Tine")?.roots; expect(roots?.length).toBeGreaterThan(0); return roots![0]; });
  await vi.waitFor(() => expect(host.querySelector(`[data-block-id="${id}"]`), "the block is rendered").not.toBeNull(), { timeout: 5000 });
  startEditing(id, 0);
  const textarea = await vi.waitFor(() => {
    const ta = maybeEditorIn(host);
    expect(ta).not.toBeNull();
    return ta!;
  });
  textarea.element.focus();
  const toolbar = await vi.waitFor(() => {
    const found = host.querySelector<HTMLElement>("[data-mobile-keyboard-toolbar]:not([hidden])");
    expect(found, "the editing toolbar is up").not.toBeNull();
    return found!;
  });
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
    const submit = vi.spyOn(backend(), "pageSubmit");
    const { host, dispose, textarea, toolbar, id } = await openTine(true);
    try {
      textarea.replaceText("typed on the phone");
      textarea.select(textarea.value().length, textarea.value().length);
      textarea.reportInput({ bubbles: true }, "plain");
      const back = toolbar.querySelector<HTMLButtonElement>('.mobile-keyboard-toolbar-nav button[title="Go back"]')!;
      expect(back.disabled).toBe(false);
      expect(route()).toMatchObject({ kind: "page", name: "Tine" });

      const down = pointer("pointerdown");
      back.dispatchEvent(down);
      expect(down.defaultPrevented).toBe(true);
      back.dispatchEvent(pointer("pointerup"));

      expect(editingId(), "the edit ended").toBeNull();
      await vi.waitFor(() => expect(route(), "Back navigated").toMatchObject({ kind: "journals" }));
      expect(pageByName("Tine")!.roots[0]).toBe(id);
      await vi.waitFor(() => expect(submittedPages(submit, "Tine").at(-1)?.blocks[0].raw).toBe("typed on the phone"));
    } finally { dispose(); host.remove(); }
  });

  it("tapping Search ends the edit through the same save path and opens the switcher", async () => {
    const submit = vi.spyOn(backend(), "pageSubmit");
    const { host, dispose, textarea, toolbar } = await openTine(true);
    try {
      textarea.replaceText("search me");
      textarea.reportInput({ bubbles: true }, "plain");
      const search = toolbar.querySelector<HTMLButtonElement>(".mobile-keyboard-toolbar-nav [data-search-trigger]")!;
      search.dispatchEvent(pointer("pointerdown"));
      search.dispatchEvent(pointer("pointerup"));
      await vi.waitFor(() => expect(host.querySelector(".switcher-overlay")).not.toBeNull());
      expect(editingId()).toBeNull();
      expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Tine" });
      await vi.waitFor(() => expect(submittedPages(submit, "Tine").at(-1)?.blocks[0].raw).toBe("search me"));
    } finally { dispose(); host.remove(); }
  });
});
