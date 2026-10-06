// OG-MULTIWINDOW: pane select acts in the window the user is in, so its hint
// renders there and nowhere else. Before the gate, entering pane select in a
// workspace window showed the hint in main too (seen in the hosted journey).
import { afterEach, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { PaneSelectHint } from "../App";
import { createWindowLayout, dropWindowLayout, firstPaneId, layoutRoot, resetPaneLayoutToSingle } from "../panes";
import { enterPaneSelect, exitPaneSelect } from "../paneSelect";
import { MAIN_WINDOW_ID, WindowContext, registerWindow, setActiveWindowId } from "../windowRealm";

const page = { tabs: [{ history: [{ kind: "page" as const, name: "Alpha", pageKind: "page" as const }], pos: 0, pinned: false }], activeIndex: 0 };
const cleanups: (() => void)[] = [];

afterEach(() => {
  exitPaneSelect();
  setActiveWindowId(MAIN_WINDOW_ID);
  dropWindowLayout("ws-hint");
  resetPaneLayoutToSingle();
  while (cleanups.length) cleanups.pop()!();
});

function mount(windowId: string): HTMLElement {
  const host = document.createElement("div");
  document.body.append(host);
  cleanups.push(render(() => <WindowContext.Provider value={windowId}><PaneSelectHint /></WindowContext.Provider>, host), () => host.remove());
  return host;
}

describe("pane select hint across windows", () => {
  it("shows only in the window where pane select was entered", () => {
    const paneId = createWindowLayout("ws-hint", page);
    const frame = document.createElement("iframe");
    document.body.append(frame);
    cleanups.push(registerWindow("ws-hint", frame.contentWindow!), () => frame.remove());
    const main = mount(MAIN_WINDOW_ID);
    const ws = mount("ws-hint");
    setActiveWindowId("ws-hint");
    enterPaneSelect(paneId);
    expect(ws.querySelector(".pane-select-hint")).toBeTruthy();
    expect(main.querySelector(".pane-select-hint")).toBeNull();

    exitPaneSelect();
    setActiveWindowId(MAIN_WINDOW_ID);
    enterPaneSelect(firstPaneId(layoutRoot(MAIN_WINDOW_ID))!);
    expect(main.querySelector(".pane-select-hint")).toBeTruthy();
    expect(ws.querySelector(".pane-select-hint")).toBeNull();
  });
});
