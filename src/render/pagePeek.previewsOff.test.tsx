import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { LONG_PRESS_DELAY } from "./longPress";
import { ContextMenu } from "../components/ContextMenu";
import { closeContextMenu, contextMenu } from "../ui";
import { setGraphMeta } from "../graphSession";
import { openPage, route } from "../router";
import { clearTransientLayersForTest } from "../transientLayers";
import { initHoverPreviewSettings, previewsEnabled } from "../hoverPreviewSettings";
import type { GraphMeta } from "../types";

// GH #824 / D-20261110-TT: Logseq's `:ui/enable-tooltip?` (served as
// GraphMeta.enable_tooltip) switches every page, tag and block-reference
// preview off through the one disabled predicate of createPeekBridge. On touch
// the hold then opens the page's context menu, as before b807df88b.

const touch = vi.hoisted(() => ({ primary: false }));
vi.mock("../inputCapability", async (original) => ({
  ...await original<typeof import("../inputCapability")>(), touchPrimary: () => touch.primary,
}));

const PEEK_OPEN_MS = 1000; // the default hover delay (OG tippy :delay [1000 100])
const BLOCK_REF = "((58900000-0000-4000-8000-0000000000b1))";

beforeAll(async () => { await initParser(); });
beforeEach(async () => {
  vi.useFakeTimers();
  touch.primary = false;
  vi.spyOn(backend(), "getAppString").mockImplementation(async (_key, fallback) => fallback);
  initHoverPreviewSettings();
  await advance(0);
});
afterEach(() => {
  closeContextMenu();
  clearTransientLayersForTest();
  setGraphMeta(null);
  vi.useRealTimers();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

const meta = (enable_tooltip?: boolean) => setGraphMeta({ root: "/test", ...(enable_tooltip === undefined ? {} : { enable_tooltip }) } as GraphMeta);
const popup = () => document.body.querySelector(".peek-popup");

async function advance(ms: number) {
  await vi.advanceTimersByTimeAsync(ms);
  await Promise.resolve();
  await Promise.resolve();
}

function mount(raw: string): { root: HTMLElement; dispose: () => void } {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <div class="block-content"><AstBody raw={raw} /><ContextMenu /></div>, host);
  return { root: host, dispose: () => { dispose(); host.remove(); } };
}

function hover(el: Element, pointerType = "mouse") {
  el.dispatchEvent(new PointerEvent("pointerenter", { bubbles: false, pointerType, isPrimary: true }));
  el.dispatchEvent(new MouseEvent("mouseenter", { bubbles: false }));
}

function pointer(element: Element, type: string, x = 40) {
  element.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true,
    pointerType: "touch", pointerId: 7, isPrimary: true, clientX: x, clientY: 40 }));
}

const LINKS = ["[[Tine]]", "#Tine", BLOCK_REF];
const anchorOf = (root: HTMLElement) => root.querySelector("a.page-ref, a.tag, .block-ref")!;

describe("GH #824: the setting is read from the graph's :ui/enable-tooltip?", () => {
  it.each([
    ["no graph loaded", null, true],
    ["key absent", undefined, true],
    ["true", true, true],
    ["false", false, false],
  ] as const)("%s → previews %s", (_name, value, expected) => {
    if (_name === "no graph loaded") setGraphMeta(null); else meta(value);
    expect(previewsEnabled()).toBe(expected);
  });
});

describe("GH #824: mouse hover with Reference previews off", () => {
  it.each(LINKS)("%s arms no preview and fetches no target", async (raw) => {
    meta(false);
    const getPage = vi.spyOn(backend(), "getPage");
    const previewBlock = vi.spyOn(backend(), "previewBlock");
    const { root, dispose } = mount(raw);
    try {
      hover(anchorOf(root));
      await advance(PEEK_OPEN_MS * 3);
      expect(popup()).toBeNull();
      expect(getPage).not.toHaveBeenCalledWith("Tine", expect.anything());
      expect(previewBlock).not.toHaveBeenCalled();
    } finally { dispose(); }
  });

  it.each(LINKS)("%s previews after the delay when the key is true or absent", async (raw) => {
    for (const enabled of [true, undefined]) {
      meta(enabled);
      const { root, dispose } = mount(raw);
      try {
        hover(anchorOf(root));
        await advance(PEEK_OPEN_MS - 1);
        expect(popup()).toBeNull();
        await advance(1);
        for (let i = 0; i < 40 && !popup(); i++) await advance(25);
        expect(popup()).not.toBeNull();
      } finally { dispose(); }
    }
  });

  it("switching previews off closes a preview that is already open", async () => {
    meta(true);
    const { root, dispose } = mount("[[Tine]]");
    try {
      hover(anchorOf(root));
      await advance(PEEK_OPEN_MS);
      for (let i = 0; i < 40 && !popup(); i++) await advance(25);
      expect(popup()).not.toBeNull();
      meta(false);
      await advance(0);
      expect(popup()).toBeNull();
    } finally { dispose(); }
  });
});

describe("GH #824: touch hold on a page link or tag", () => {
  it.each(["[[Tine]]", "#Tine"])("%s: off → the hold opens the page context menu, previews nothing and does not navigate", async (raw) => {
    touch.primary = true;
    meta(false);
    openPage("Before", "page");
    const { root, dispose } = mount(raw);
    try {
      const link = root.querySelector("a")!;
      pointer(link, "pointerdown", 41);
      await advance(LONG_PRESS_DELAY + 100);
      expect(contextMenu()).toMatchObject({ kind: "page", name: "Tine", x: 41 });
      expect(popup()).toBeNull();
      pointer(link, "pointerup", 41);
      link.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }));
      expect(route()).toMatchObject({ name: "Before" });
    } finally { dispose(); }
  });

  it.each(["[[Tine]]", "#Tine"])("%s: on → the hold previews and leaves the context menu closed", async (raw) => {
    touch.primary = true;
    for (const enabled of [true, undefined]) {
      closeContextMenu();
      meta(enabled);
      const { root, dispose } = mount(raw);
      try {
        const link = root.querySelector("a")!;
        pointer(link, "pointerdown");
        await advance(LONG_PRESS_DELAY + 100);
        expect(popup()).not.toBeNull();
        expect(contextMenu()).toBeNull();
      } finally { dispose(); closeContextMenu(); }
    }
  });

  it("switching previews off and on again does not reopen a held preview without a new hold", async () => {
    touch.primary = true;
    meta(true);
    const { root, dispose } = mount("[[Tine]]");
    try {
      pointer(root.querySelector("a")!, "pointerdown");
      await advance(LONG_PRESS_DELAY + 100);
      expect(popup()).not.toBeNull();
      meta(false);
      await advance(0);
      expect(popup()).toBeNull();
      meta(true);
      await advance(PEEK_OPEN_MS * 2);
      expect(popup()).toBeNull();
    } finally { dispose(); }
  });
});
