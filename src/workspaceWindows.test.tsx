// OG-MULTIWINDOW P4: a workspace window is disposed exactly once whichever door
// closes it, ends an edit typed in it, asks the save engine to flush, forgets
// its pane tree and destroys its native window. An iframe stands in for the
// native popup's engine-linked Document.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const native = vi.hoisted(() => ({
  calls: [] as { cmd: string; args: unknown }[],
  closeRequested: undefined as ((event: { payload: string }) => void) | undefined,
  label: 0,
}));
vi.mock("./backend", async (importOriginal) => ({ ...(await importOriginal<typeof import("./backend")>()), isTauri: () => true }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args: unknown) => {
    native.calls.push({ cmd, args });
    return cmd === "workspace_window_prepare" ? `ws-main-${++native.label}` : undefined;
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_name: string, handler: (event: { payload: string }) => void) => {
    native.closeRequested = handler;
    return () => { native.closeRequested = undefined; };
  }),
}));
const saves = vi.hoisted(() => ({ flushAll: vi.fn(async () => true) }));
vi.mock("./document", async (importOriginal) => ({ ...(await importOriginal<typeof import("./document")>()), flushAll: saves.flushAll }));

import { render } from "solid-js/web";
import {
  closeAllWorkspaceWindows,
  disposeWorkspaceWindow,
  installWorkspaceWindowShell,
  installWorkspaceWindows,
  openRouteInNewWindow,
  workspaceWindowCount,
  resetWorkspaceWindowsForTest,
} from "./workspaceWindows";
import { closePane, layoutPaneIds, layoutRoot, layoutWindowIds, resetPaneLayoutToSingle } from "./panes";
import { installWorkspaceWindowSession } from "./session";
import { MAIN_WINDOW_ID, MAX_WORKSPACE_WINDOWS, mainWindow, setActiveWindowId, windowById, windowIds } from "./windowRealm";
import { editingId, startEditing } from "./editorController";
import { setToasts, toasts } from "./toasts";

type Realm = Window & typeof globalThis;
const frames: HTMLIFrameElement[] = [];
const cleanups: (() => void)[] = [];

beforeEach(() => {
  native.calls = [];
  native.label = 0;
  saves.flushAll.mockClear();
  setToasts([]);
  setActiveWindowId(MAIN_WINDOW_ID);
  resetPaneLayoutToSingle();
  vi.spyOn(mainWindow, "open").mockImplementation(() => {
    const frame = document.createElement("iframe");
    document.body.append(frame);
    frames.push(frame);
    const win = frame.contentWindow as Realm;
    win.focus = () => {};
    // The native window's own close: the iframe goes away.
    win.close = () => { frame.remove(); };
    return win;
  });
  cleanups.push(installWorkspaceWindowShell((id, mount) => render(() => (
    <div class="shell" data-shell={id}><textarea class="block-editor" /></div>
  ), mount)));
  cleanups.push(installWorkspaceWindows(installWorkspaceWindowSession));
});
afterEach(() => {
  resetWorkspaceWindowsForTest();
  while (cleanups.length) cleanups.pop()!();
  for (const frame of frames.splice(0)) frame.remove();
  vi.restoreAllMocks();
});

const page = (name: string) => ({ kind: "page" as const, name, pageKind: "page" as const });
const destroys = () => native.calls.filter((c) => c.cmd === "workspace_window_destroy").map((c) => (c.args as { label: string }).label);

async function openReady(name = "Alpha"): Promise<{ id: string; win: Realm }> {
  const id = openRouteInNewWindow(page(name))!;
  expect(id).toBeTruthy();
  await vi.waitFor(() => expect(windowById(id)).toBeTruthy());
  return { id, win: windowById(id) as Realm };
}

describe("workspace window lifecycle (P4)", () => {
  it("installs the pane tree synchronously, then opens, mirrors styles and renders the shell", async () => {
    const style = document.createElement("style");
    style.textContent = ".probe{color:red}";
    document.head.append(style);
    cleanups.push(() => style.remove());
    document.documentElement.setAttribute("data-theme", "dark");
    const id = openRouteInNewWindow(page("Alpha"))!;
    expect(layoutWindowIds()).toContain(id); // before any await
    await vi.waitFor(() => expect(windowById(id)).toBeTruthy());
    const doc = windowById(id)!.document;
    expect(doc.querySelector(`[data-shell="${id}"]`)).toBeTruthy();
    expect(doc.documentElement.getAttribute("data-theme")).toBe("dark");
    expect([...doc.head.querySelectorAll("style")].some((s) => s.textContent === ".probe{color:red}")).toBe(true);
    // A theme change in main reaches the open window.
    document.documentElement.setAttribute("data-theme", "light");
    await vi.waitFor(() => expect(doc.documentElement.getAttribute("data-theme")).toBe("light"));
    expect(native.calls[0]).toEqual({ cmd: "workspace_window_prepare", args: { geometry: null } });
    expect(mainWindow.open).toHaveBeenCalledWith("about:blank", "ws-main-1", "popup");
  });

  it("disposes exactly once whichever doors race, and ends an edit typed in the window", async () => {
    const { id, win } = await openReady();
    const editor = win.document.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
    editor.focus();
    startEditing("block-1", 0);
    expect(editingId()).toBe("block-1");
    const blurred = vi.fn();
    editor.addEventListener("blur", blurred);
    // Its last pane closing is the user's door.
    const paneId = layoutPaneIds(layoutRoot(id))[0];
    expect(closePane(paneId)).toBe(true);
    expect(editingId()).toBeNull();
    expect(saves.flushAll).toHaveBeenCalledOnce();
    expect(layoutWindowIds()).toEqual([MAIN_WINDOW_ID]);
    expect(windowIds()).not.toContain(id);
    // The native close request and main closing arrive late: nothing more happens.
    native.closeRequested?.({ payload: "ws-main-1" });
    expect(disposeWorkspaceWindow(id, "native")).toBe(false);
    closeAllWorkspaceWindows("quit");
    await vi.waitFor(() => expect(destroys()).toContain("ws-main-1"));
    // The stray request for an already-disposed label destroys natively again
    // (idempotent on the Rust side), but never re-disposes the window.
    expect(saves.flushAll).toHaveBeenCalledOnce();
    expect(blurred.mock.calls.length).toBeLessThanOrEqual(1);
  });

  it("a native title-bar close disposes the window named by its label", async () => {
    const { id } = await openReady();
    await vi.waitFor(() => expect(native.closeRequested).toBeTruthy());
    native.closeRequested!({ payload: "ws-main-1" });
    await vi.waitFor(() => expect(workspaceWindowCount()).toBe(0));
    expect(layoutWindowIds()).not.toContain(id);
    expect(saves.flushAll).toHaveBeenCalledOnce();
    await vi.waitFor(() => expect(destroys()).toEqual(["ws-main-1"]));
  });

  it("a title-bar close lets input already on its way land before the edit ends", async () => {
    const { win } = await openReady();
    await vi.waitFor(() => expect(native.closeRequested).toBeTruthy());
    const editor = win.document.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
    editor.focus();
    startEditing("block-1", 0);
    native.closeRequested!({ payload: "ws-main-1" });
    native.closeRequested!({ payload: "ws-main-1" }); // a repeated request is ignored
    // A keystroke the engine delivers after the close request still finds the editor live.
    await new Promise((resolve) => setTimeout(resolve, 60));
    expect(workspaceWindowCount()).toBe(1);
    expect(editingId()).toBe("block-1");
    editor.dispatchEvent(new Event("input", { bubbles: true }));
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(workspaceWindowCount()).toBe(1);
    await vi.waitFor(() => expect(workspaceWindowCount()).toBe(0));
    expect(editingId()).toBeNull();
    expect(saves.flushAll).toHaveBeenCalledOnce();
    await vi.waitFor(() => expect(destroys()).toEqual(["ws-main-1"]));
  });

  it("another door disposes at once while a title-bar close is settling", async () => {
    await openReady();
    await vi.waitFor(() => expect(native.closeRequested).toBeTruthy());
    native.closeRequested!({ payload: "ws-main-1" });
    closeAllWorkspaceWindows("quit");
    expect(workspaceWindowCount()).toBe(0);
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(destroys()).toEqual(["ws-main-1"]);
  });

  it("main reloading closes every window without a session save", async () => {
    await openReady("Alpha");
    await openReady("Beta");
    saves.flushAll.mockClear();
    mainWindow.dispatchEvent(new Event("pagehide"));
    expect(workspaceWindowCount()).toBe(0);
    expect(layoutWindowIds()).toEqual([MAIN_WINDOW_ID]);
    await vi.waitFor(() => expect(destroys().sort()).toEqual(["ws-main-1", "ws-main-2"]));
    expect(saves.flushAll).not.toHaveBeenCalled();
  });

  it("refuses past MAX_WORKSPACE_WINDOWS with a visible reason", () => {
    for (let i = 0; i < MAX_WORKSPACE_WINDOWS; i++) expect(openRouteInNewWindow(page(`P${i}`))).toBeTruthy();
    expect(openRouteInNewWindow(page("One too many"))).toBeNull();
    expect(workspaceWindowCount()).toBe(MAX_WORKSPACE_WINDOWS);
    expect(toasts().some((t) => t.kind === "error" && t.message.includes(String(MAX_WORKSPACE_WINDOWS)))).toBe(true);
  });

  it("a window closed before its native window appears never opens it", async () => {
    const id = openRouteInNewWindow(page("Alpha"))!;
    expect(disposeWorkspaceWindow(id, "user")).toBe(true);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(mainWindow.open).not.toHaveBeenCalled();
    expect(windowIds()).not.toContain(id);
  });
});
