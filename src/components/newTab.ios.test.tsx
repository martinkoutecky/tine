// GH #705 / D-20261018-W1 (review r1): an iOS identity with a FINE pointer (an
// iPad with a trackpad, or a harness) still offers Open in new tab. Identity and
// capability are independent variables; the old identity gate fails this file.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
vi.hoisted(() => { globalThis.__TINE_PLATFORM__ = "ios"; });
import { ContextMenu } from "./ContextMenu";
import { closeContextMenu, openContextMenu, openPageContextMenu } from "../ui";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { initParser } from "../render/parse";
import { resetPaneLayoutToSingle } from "../panes";
import { internalLinkAuxClick, internalLinkDest } from "../linkGesture";
import { isMobilePlatform } from "../nativeChrome";
import type { BlockDto, PageDto } from "../types";

beforeAll(initParser);
const snapshot = () => ({
  tabs: [{ history: [{ kind: "page" as const, name: "P", pageKind: "page" as const }], pos: 0, pinned: false }],
  activeIndex: 0,
});
afterEach(() => {
  closeContextMenu(); resetStore(); resetPaneLayoutToSingle(snapshot()); document.body.replaceChildren();
});
const page = (): PageDto => ({
  name: "P", kind: "page", title: "P", pre_block: null,
  blocks: [{ id: "A", raw: "target block", collapsed: false, children: [] } as BlockDto],
} as PageDto);
const labels = () => [...document.querySelectorAll(".ctx-item")].map((e) => e.textContent?.trim() ?? "");

describe("iOS identity, fine pointer", () => {
  it("really is an iOS identity", () => { expect(isMobilePlatform).toBe(true); });

  it("page and block menus offer Open in new tab", () => {
    loadSingle(page());
    const dispose = render(() => <ContextMenu />, document.body.appendChild(document.createElement("div")));
    try {
      openPageContextMenu(10, 10, "P");
      expect(labels()).toContain("Open in new tab");
      closeContextMenu();
      openContextMenu(10, 10, "A");
      expect(labels()).toContain("Open in new tab");
    } finally { dispose(); }
  });

  it("Ctrl/Cmd-click and middle-click resolve to a background tab", () => {
    expect(internalLinkDest({ shiftKey: false, ctrlKey: true, metaKey: false, altKey: false })).toBe("background");
    const open = vi.fn();
    expect(internalLinkAuxClick({ button: 1, preventDefault: vi.fn() }, open)).toBe(true);
    expect(open).toHaveBeenCalledOnce();
  });
});
