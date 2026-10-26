// OG-MULTIWINDOW P3: each Tine window owns its pane tree, focused pane and
// maximize state, and pane commands resolve within the window the user is in.
// An iframe gives jsdom a genuinely separate window + document, the shape a
// workspace popup has in the real app.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  allPaneIds,
  closePane,
  createWindowLayout,
  currentLayoutWindowId,
  dropWindowLayout,
  focusPane,
  focusedPaneId,
  focusedRouter,
  installWorkspaceWindowCloser,
  layoutPaneIds,
  layoutRoot,
  paneRouter,
  resetPaneLayoutToSingle,
  restorePaneLayout,
  splitPane,
  togglePaneMaximize,
  visibleLayoutNode,
  windowOfPane,
} from "./panes";
import { installPaneTracker } from "./ui";
import { MAIN_WINDOW_ID, activeWindowId, mainWindow, registerWindow, setActiveWindowId } from "./windowRealm";
import type { PaneSnapshot } from "./router";

const page = (name: string): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
});
const journals = (): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

const cleanups: (() => void)[] = [];
beforeEach(() => {
  setActiveWindowId(MAIN_WINDOW_ID);
  resetPaneLayoutToSingle(journals());
});
afterEach(() => {
  while (cleanups.length) cleanups.pop()!();
  document.body.innerHTML = "";
  setActiveWindowId(MAIN_WINDOW_ID);
});

type Realm = Window & typeof globalThis;
function openWorkspace(id: string, snapshot = page("Alpha")) {
  const frame = document.createElement("iframe");
  document.body.append(frame);
  const win = frame.contentWindow as Realm;
  win.focus = () => {}; // jsdom does not implement window focus()
  const unregister = registerWindow(id, win);
  const paneId = createWindowLayout(id, snapshot);
  const close = () => { dropWindowLayout(id); unregister(); frame.remove(); };
  cleanups.push(close);
  return { win, doc: win.document, paneId, close };
}

describe("per-window pane layouts", () => {
  it("a workspace window gets its own one-pane tree, and splitting there leaves main alone", () => {
    const ws = openWorkspace("ws-1-1");
    expect(layoutPaneIds(layoutRoot("ws-1-1"))).toEqual([ws.paneId]);
    expect(layoutPaneIds(layoutRoot(MAIN_WINDOW_ID))).toEqual(["main"]);
    expect(windowOfPane(ws.paneId)).toBe("ws-1-1");
    expect(windowOfPane("main")).toBe(MAIN_WINDOW_ID);
    expect(paneRouter(ws.paneId).route()).toMatchObject({ kind: "page", name: "Alpha" });

    const second = splitPane(ws.paneId, "row")!;
    expect(second).toBeTruthy();
    expect(layoutPaneIds(layoutRoot("ws-1-1"))).toEqual([ws.paneId, second]);
    expect(layoutPaneIds(layoutRoot(MAIN_WINDOW_ID))).toEqual(["main"]);
    expect(allPaneIds()).toEqual(["main", ws.paneId, second]);
    // Maximize is per window too.
    expect(togglePaneMaximize(second)).toBe(true);
    expect(visibleLayoutNode("ws-1-1")).toEqual({ kind: "pane", paneId: second });
    expect(visibleLayoutNode(MAIN_WINDOW_ID)).toEqual(layoutRoot(MAIN_WINDOW_ID));
  });

  it("the focused pane and focused router follow the window the user is in", () => {
    const ws = openWorkspace("ws-2-1");
    expect(currentLayoutWindowId()).toBe(MAIN_WINDOW_ID);
    expect(focusedPaneId()).toBe("main");
    ws.win.dispatchEvent(new ws.win.FocusEvent("focus"));
    expect(activeWindowId()).toBe("ws-2-1");
    expect(currentLayoutWindowId()).toBe("ws-2-1");
    expect(focusedPaneId()).toBe(ws.paneId);
    expect(focusedRouter().paneId).toBe(ws.paneId);
    expect(focusedPaneId(MAIN_WINDOW_ID)).toBe("main"); // main keeps its own focus
    // Focusing a pane of another window hands the user over to that window.
    const focusMain = vi.spyOn(mainWindow, "focus").mockImplementation(() => {});
    focusPane("main");
    expect(activeWindowId()).toBe(MAIN_WINDOW_ID);
    expect(focusMain).toHaveBeenCalledOnce();
    expect(focusedRouter().paneId).toBe("main");
    focusMain.mockRestore();
  });

  it("a click outside every pane in a workspace window keeps that window's own pane", () => {
    const stop = installPaneTracker();
    cleanups.push(stop);
    const ws = openWorkspace("ws-3-1");
    const second = splitPane(ws.paneId, "row")!;
    focusPane(second);
    const chrome = ws.doc.createElement("div");
    ws.doc.body.append(chrome);
    chrome.dispatchEvent(new ws.win.MouseEvent("pointerdown", { bubbles: true }));
    expect(windowOfPane(focusedPaneId("ws-3-1"))).toBe("ws-3-1");
    expect(focusedPaneId("ws-3-1")).toBe(ws.paneId);
    // A click on a pane in the popup focuses that pane.
    const paneEl = ws.doc.createElement("div");
    paneEl.setAttribute("data-pane-id", second);
    ws.doc.body.append(paneEl);
    paneEl.dispatchEvent(new ws.win.MouseEvent("pointerdown", { bubbles: true }));
    expect(focusedPaneId("ws-3-1")).toBe(second);
    expect(focusedPaneId(MAIN_WINDOW_ID)).toBe("main");
  });

  it("closing a workspace window's last pane asks its closer; main's last pane stays", () => {
    const ws = openWorkspace("ws-4-1");
    const closer = vi.fn(() => true);
    cleanups.push(installWorkspaceWindowCloser(closer));
    expect(closePane("main")).toBe(false);
    expect(closePane(ws.paneId)).toBe(true);
    expect(closer).toHaveBeenCalledWith("ws-4-1");
  });

  it("restoring a tree refuses pane ids another window owns, and main in a workspace window", () => {
    const ws = openWorkspace("ws-5-1");
    const snaps = new Map<string, PaneSnapshot>();
    expect(restorePaneLayout({ kind: "pane", paneId: ws.paneId }, snaps, ws.paneId, "ws-5-2")).toBe(false);
    expect(restorePaneLayout({ kind: "pane", paneId: "main" }, snaps, "main", "ws-5-2")).toBe(false);
    expect(layoutPaneIds(layoutRoot("ws-5-1"))).toEqual([ws.paneId]);
    // Main's restore keeps the workspace window's routers alive.
    expect(restorePaneLayout({ kind: "pane", paneId: "main" }, new Map([["main", journals()]]))).toBe(true);
    expect(paneRouter(ws.paneId).route()).toMatchObject({ kind: "page", name: "Alpha" });
  });

  it("dropping a window forgets its tree and is idempotent; main can never be dropped", () => {
    const ws = openWorkspace("ws-6-1");
    dropWindowLayout("ws-6-1");
    dropWindowLayout("ws-6-1");
    dropWindowLayout(MAIN_WINDOW_ID);
    expect(allPaneIds()).toEqual(["main"]);
    expect(windowOfPane(ws.paneId)).toBe(MAIN_WINDOW_ID);
    expect(layoutRoot(MAIN_WINDOW_ID)).toEqual({ kind: "pane", paneId: "main" });
  });

  it("resetting main to a single pane leaves workspace windows intact", () => {
    const ws = openWorkspace("ws-7-1");
    splitPane("main", "row");
    resetPaneLayoutToSingle(journals());
    expect(layoutPaneIds(layoutRoot(MAIN_WINDOW_ID))).toEqual(["main"]);
    expect(layoutPaneIds(layoutRoot("ws-7-1"))).toEqual([ws.paneId]);
  });
});
