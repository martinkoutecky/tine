// GH #705: a phone or tablet shell has no tab strip (App.tsx hides it when
// isMobilePlatform), so a background tab opened there is unreachable. Every
// surface that offered one must offer something reachable instead.
import { readFileSync } from "node:fs";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
vi.mock("../nativeChrome", async (original) => ({ ...(await original<typeof import("../nativeChrome")>()), isMobilePlatform: true }));
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

describe("GH #705: mobile menus offer no unreachable background tab", () => {
  it("page menu keeps Open in sidebar and drops Open in new tab", () => {
    loadSingle(page());
    const dispose = render(() => <ContextMenu />, document.body.appendChild(document.createElement("div")));
    try {
      openPageContextMenu(10, 10, "P");
      expect(labels()).toContain("Open in sidebar");
      expect(labels()).not.toContain("Open in new tab");
    } finally { dispose(); }
  });

  it.each([false, true])("block menu (read-only=%s) keeps Open in sidebar and drops Open in new tab", (readOnly) => {
    loadSingle(page(readOnly));
    const dispose = render(() => <ContextMenu />, document.body.appendChild(document.createElement("div")));
    try {
      openContextMenu(10, 10, "A");
      expect(labels()).toContain("Open in sidebar");
      expect(labels()).not.toContain("Open in new tab");
    } finally { dispose(); }
  });
});

describe("GH #705: mobile modified-click and chord paths never open a background tab", () => {
  it("the shared link gesture resolves Ctrl/Cmd+click to the ordinary navigation and ignores middle-click", () => {
    expect(internalLinkDest({ shiftKey: false, ctrlKey: true, metaKey: false, altKey: false })).toBe("default");
    expect(internalLinkDest({ shiftKey: false, ctrlKey: false, metaKey: true, altKey: false })).toBe("default");
    // The other destinations are untouched.
    expect(internalLinkDest({ shiftKey: true, ctrlKey: false, metaKey: false, altKey: false })).toBe("sidebar");
    expect(internalLinkDest({ shiftKey: false, ctrlKey: false, metaKey: false, altKey: true })).toBe("pane");
    const open = vi.fn();
    const preventDefault = vi.fn();
    expect(internalLinkAuxClick({ button: 1, preventDefault }, open)).toBe(false);
    expect(open).not.toHaveBeenCalled();
  });

  it("Ctrl/Cmd+click on an outline bullet zooms in place and opens no tab", () => {
    loadSingle(page());
    const host = document.body.appendChild(document.createElement("div"));
    const dispose = render(() => <For each={pageByName("P")?.roots ?? []}>{(id) => <Block id={id} />}</For>, host);
    try {
      const bullet = host.querySelector<HTMLElement>('[data-block-id="A"] > .block-main .bullet-container')!;
      const before = tabCount();
      bullet.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, ctrlKey: true }));
      expect(tabCount()).toBe(before);
      expect(layoutPaneIds()).toEqual(["main"]);
    } finally { dispose(); }
  });

  it("Search Ctrl-click and Ctrl+Enter choose the result instead of opening a background tab", async () => {
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
      await vi.waitFor(() => expect(root.querySelector(".switcher-overlay")).toBeNull());
      expect(tabCount()).toBe(before);
      expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "Clickable" });

      openSwitcher();
      await vi.waitFor(() => expect(root.querySelector(".switcher-row")).not.toBeNull());
      root.querySelector<HTMLInputElement>(".switcher-input")!.dispatchEvent(new KeyboardEvent("keydown", {
        key: "Enter", ctrlKey: true, bubbles: true, cancelable: true,
      }));
      await vi.waitFor(() => expect(root.querySelector(".switcher-overlay")).toBeNull());
      expect(tabCount()).toBe(before);
    } finally { dispose(); }
  });
});

it("the Guide says phones and tablets omit Open in new tab", () => {
  const guide = readFileSync("crates/tine-core/src/templates/pages-links-references-search.md", "utf8");
  expect(guide).toContain("Phones and tablets have no tab strip, so their menus omit **Open in new tab**");
});
