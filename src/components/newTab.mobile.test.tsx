import { readFileSync } from "node:fs";
// GH #705 / D-20261018-W1: a phone has no room for the top bar's tab strip, but
// it is no longer a place without tabs: the sidebar's Tabs section lists them at
// every width, so every surface that opens a background tab offers it on a
// touch-first screen too, exactly as on a desktop.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
// Identity is varied independently of capability (review r1): this file runs
// as an Android app WITH a finger; newTab.ios.test.tsx runs an iOS identity
// with a fine pointer. The old identity gate (!isMobilePlatform) fails both.
vi.hoisted(() => { globalThis.__TINE_PLATFORM__ = "android"; });
vi.mock("../inputCapability", async (original) => ({
  ...(await original<typeof import("../inputCapability")>()), touchPrimary: () => true, touchCapable: () => true,
}));
import { ContextMenu } from "./ContextMenu";
import { QuickSwitcher } from "./QuickSwitcher";
import { Block } from "./Block";
import { closeContextMenu, closeSwitcher, openContextMenu, openPageContextMenu, openSwitcher, setRecentPages, setRightSidebar } from "../ui";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { initParser } from "../render/parse";
import { layoutPaneIds, paneRouter, resetPaneLayoutToSingle } from "../panes";
import { internalLinkAuxClick, internalLinkDest } from "../linkGesture";
import type { BlockDto, PageDto } from "../types";

beforeAll(initParser);

const snapshot = () => ({
  tabs: [{ history: [{ kind: "page" as const, name: "P", pageKind: "page" as const }], pos: 0, pinned: false }],
  activeIndex: 0,
});

afterEach(() => {
  closeContextMenu();
  closeSwitcher();
  setRecentPages([]);
  setRightSidebar([]);
  resetStore();
  resetPaneLayoutToSingle(snapshot());
  document.body.replaceChildren();
});

const page = (readOnly = false): PageDto => ({
  name: "P", kind: "page", title: "P", pre_block: null,
  blocks: [{ id: "A", raw: "target block", collapsed: false, children: [] } as BlockDto],
  ...(readOnly ? { read_only: true } : {}),
} as PageDto);

const labels = () => [...document.querySelectorAll(".ctx-item")].map((e) => e.textContent?.trim() ?? "");
const tabCount = () => paneRouter("main").tabs().length;

import { isMobilePlatform } from "../nativeChrome";
it("really runs as an Android identity", () => { expect(isMobilePlatform).toBe(true); });

describe("GH #705 + D-20261018-W1: a touch-first screen offers Open in new tab", () => {
  it("page menu offers Open in sidebar and Open in new tab", () => {
    loadSingle(page());
    const dispose = render(() => <ContextMenu />, document.body.appendChild(document.createElement("div")));
    try {
      openPageContextMenu(10, 10, "P");
      expect(labels()).toContain("Open in sidebar");
      expect(labels()).toContain("Open in new tab");
    } finally { dispose(); }
  });

  it.each([false, true])("block menu (read-only=%s) offers Open in sidebar and Open in new tab", (readOnly) => {
    loadSingle(page(readOnly));
    const dispose = render(() => <ContextMenu />, document.body.appendChild(document.createElement("div")));
    try {
      openContextMenu(10, 10, "A");
      expect(labels()).toContain("Open in sidebar");
      expect(labels()).toContain("Open in new tab");
    } finally { dispose(); }
  });

  it("Open in new tab adds a background tab (reachable in the sidebar's Tabs section)", () => {
    loadSingle(page());
    const dispose = render(() => <ContextMenu />, document.body.appendChild(document.createElement("div")));
    try {
      const before = tabCount();
      openPageContextMenu(10, 10, "P");
      [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((e) => e.textContent?.trim() === "Open in new tab")!.click();
      expect(tabCount()).toBe(before + 1);
    } finally { dispose(); }
  });
});

describe("GH #705 + D-20261018-W1: modified-click and chord paths open a background tab on any device", () => {
  it("the shared link gesture resolves Ctrl/Cmd+click to a background tab and middle-click opens one", () => {
    expect(internalLinkDest({ shiftKey: false, ctrlKey: true, metaKey: false, altKey: false })).toBe("background");
    expect(internalLinkDest({ shiftKey: false, ctrlKey: false, metaKey: true, altKey: false })).toBe("background");
    // The other destinations are untouched.
    expect(internalLinkDest({ shiftKey: true, ctrlKey: false, metaKey: false, altKey: false })).toBe("sidebar");
    expect(internalLinkDest({ shiftKey: false, ctrlKey: false, metaKey: false, altKey: true })).toBe("pane");
    const open = vi.fn();
    const preventDefault = vi.fn();
    expect(internalLinkAuxClick({ button: 1, preventDefault }, open)).toBe(true);
    expect(open).toHaveBeenCalledOnce();
  });

  it("Ctrl/Cmd+click on an outline bullet opens a tab", () => {
    loadSingle(page());
    const host = document.body.appendChild(document.createElement("div"));
    const dispose = render(() => <For each={pageByName("P")?.roots ?? []}>{(id) => <Block id={id} />}</For>, host);
    try {
      const bullet = host.querySelector<HTMLElement>('[data-block-id="A"] > .block-main .bullet-container')!;
      const before = tabCount();
      bullet.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, ctrlKey: true }));
      expect(tabCount()).toBe(before + 1);
      expect(layoutPaneIds()).toEqual(["main"]);
    } finally { dispose(); }
  });

  it("Search Ctrl-click opens a background tab and Ctrl+Enter does too", async () => {
    setRecentPages([{ name: "Clickable", kind: "page", path: "pages/Clickable.md" }]);
    const root = document.body.appendChild(document.createElement("div"));
    const dispose = render(() => <QuickSwitcher />, root);
    try {
      openSwitcher();
      await vi.waitFor(() => expect(root.querySelector(".switcher-row")).not.toBeNull());
      const before = tabCount();
      root.querySelector<HTMLElement>(".switcher-row")!.dispatchEvent(new MouseEvent("mousedown", {
        button: 0, ctrlKey: true, bubbles: true, cancelable: true,
      }));
      await vi.waitFor(() => expect(tabCount()).toBe(before + 1));
      // The switcher stays open so several results can fan out.
      expect(root.querySelector(".switcher-overlay")).not.toBeNull();
      root.querySelector<HTMLInputElement>(".switcher-input")!.dispatchEvent(new KeyboardEvent("keydown", {
        key: "Enter", ctrlKey: true, bubbles: true, cancelable: true,
      }));
      await vi.waitFor(() => expect(tabCount()).toBe(before + 2));
    } finally { dispose(); }
  });
});

it("the Guide says a narrow window lists the new tab under Open tabs in the left sidebar", () => {
  const guide = readFileSync("crates/tine-core/src/templates/pages-links-references-search.md", "utf8");
  expect(guide).toContain("it is listed under **Open tabs** in the left sidebar");
});
