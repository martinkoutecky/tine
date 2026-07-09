import { afterEach, describe, expect, it } from "vitest";
import { closeInPageFind, inPageFindOpen } from "./inpageFind";
import { commandDefaults, eventToBindingString, installKeybindings } from "./keybindings";
import { closeSwitcher, focusMode, openSwitcher, setFocusMode, setPdfTarget, switcherOpen } from "./ui";
import { focusedPaneId, focusPane, layoutPaneIds, layoutRoot, paneRouter, resetPaneLayoutToSingle, splitRootAtEdge } from "./panes";
import { exitPaneSelect, paneSel } from "./paneSelect";
import { clearSelection, selectBlock } from "./store";
import type { PaneSnapshot } from "./router";

function keyEvent(init: Partial<KeyboardEvent>): KeyboardEvent {
  return {
    key: "",
    code: "",
    shiftKey: false,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    ...init,
  } as KeyboardEvent;
}

type FakeListener = {
  type: string;
  listener: EventListener;
  capture: boolean;
};

let restoreFakeGlobals: (() => void) | null = null;

function installFakeWindow() {
  const listeners: FakeListener[] = [];
  const windowDescriptor = Object.getOwnPropertyDescriptor(globalThis, "window");
  const documentDescriptor = Object.getOwnPropertyDescriptor(globalThis, "document");
  const fakeWindow = {
    addEventListener(type: string, listener: EventListener, options?: boolean | AddEventListenerOptions) {
      listeners.push({ type, listener, capture: options === true || !!(options as AddEventListenerOptions | undefined)?.capture });
    },
    removeEventListener(type: string, listener: EventListener, options?: boolean | EventListenerOptions) {
      const capture = options === true || !!(options as EventListenerOptions | undefined)?.capture;
      const i = listeners.findIndex((l) => l.type === type && l.listener === listener && l.capture === capture);
      if (i >= 0) listeners.splice(i, 1);
    },
  };

  Object.defineProperty(globalThis, "window", { value: fakeWindow, configurable: true });
  Object.defineProperty(globalThis, "document", {
    value: {
      activeElement: null,
      querySelector: () => null,
      querySelectorAll: () => [],
    },
    configurable: true,
  });

  restoreFakeGlobals = () => {
    if (windowDescriptor) Object.defineProperty(globalThis, "window", windowDescriptor);
    else delete (globalThis as { window?: Window }).window;
    if (documentDescriptor) Object.defineProperty(globalThis, "document", documentDescriptor);
    else delete (globalThis as { document?: Document }).document;
    restoreFakeGlobals = null;
  };

  return {
    dispatchCaptureKeydown(event: KeyboardEvent) {
      listeners
        .filter((l) => l.type === "keydown" && l.capture)
        .forEach((l) => l.listener(event));
    },
  };
}

function modFEvent() {
  let prevented = false;
  let stopped = false;
  const event = {
    key: "f",
    code: "KeyF",
    shiftKey: false,
    ctrlKey: true,
    metaKey: false,
    altKey: false,
    target: null,
    preventDefault: () => {
      prevented = true;
    },
    stopPropagation: () => {
      stopped = true;
    },
  } as unknown as KeyboardEvent;
  return {
    event,
    prevented: () => prevented,
    stopped: () => stopped,
  };
}

function trackedKeyEvent(init: Partial<KeyboardEvent>) {
  let prevented = false;
  const event = {
    key: "",
    code: "",
    shiftKey: false,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    target: null,
    preventDefault: () => {
      prevented = true;
    },
    stopPropagation: () => {},
    ...init,
  } as unknown as KeyboardEvent;
  return {
    event,
    prevented: () => prevented,
  };
}

const pageSnapshot = (name: string): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

const journalsSnapshot = (): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

afterEach(() => {
  setPdfTarget(null);
  setFocusMode(false);
  exitPaneSelect();
  clearSelection();
  if (switcherOpen()) closeSwitcher();
  resetPaneLayoutToSingle(journalsSnapshot());
  if (inPageFindOpen()) closeInPageFind({ restoreFocus: false });
  restoreFakeGlobals?.();
});

describe("keyboard binding strings", () => {
  it("serializes Shift+/ as shift+? because KeyboardEvent.key is already shifted", () => {
    expect(eventToBindingString(keyEvent({ key: "?", code: "Slash", shiftKey: true }))).toBe("shift+?");
  });

  it("normalizes synthetic Shift+/ events that report key slash", () => {
    expect(eventToBindingString(keyEvent({ key: "/", code: "Slash", shiftKey: true }))).toBe("shift+?");
  });

  it("binds help and keyboard-shortcuts commands to non-editing shortcuts", () => {
    const byId = Object.fromEntries(commandDefaults().map((c) => [c.id, c]));

    expect(byId["ui/toggle-help"]).toMatchObject({ binding: "shift+?", scope: "select" });
    expect(byId["go/keyboard-shortcuts"]).toMatchObject({ binding: "g s", scope: "select" });
  });

  it("exposes pane-select mode as a palette command (discoverability)", () => {
    const byId = Object.fromEntries(commandDefaults().map((c) => [c.id, c]));
    expect(byId["pane/select-mode"]).toMatchObject({ binding: "", scope: "global" });
  });

  it("exposes Open Guide as a palette command without a default chord", () => {
    const byId = Object.fromEntries(commandDefaults().map((c) => [c.id, c]));
    expect(byId["guide/open"]).toMatchObject({
      binding: "",
      label: "Open Guide",
      scope: "global",
    });
  });

  it("binds developer tools to mod+shift+j as a global command (#31)", () => {
    // Ctrl+Shift+I / F12 / Ctrl+Shift+C are grabbed by WebKitGTK's own inspector;
    // mod+shift+j (Chrome's console shortcut) is free and reaches the dispatcher.
    const byId = Object.fromEntries(commandDefaults().map((c) => [c.id, c]));
    expect(byId["ui/toggle-devtools"]).toMatchObject({ binding: "mod+shift+j", scope: "global" });
  });
});

describe("find-in-page routing", () => {
  it("opens notes find on mod+f when no PDF is open", () => {
    const fake = installFakeWindow();
    const dispose = installKeybindings();
    const e = modFEvent();

    fake.dispatchCaptureKeydown(e.event);

    expect(inPageFindOpen()).toBe(true);
    expect(e.prevented()).toBe(true);
    expect(e.stopped()).toBe(false);

    dispose();
  });

  it("leaves mod+f for the PDF viewer when a PDF is open", () => {
    const fake = installFakeWindow();
    const dispose = installKeybindings();
    const e = modFEvent();
    setPdfTarget({ filename: "paper.pdf", label: "Paper" });

    fake.dispatchCaptureKeydown(e.event);

    expect(inPageFindOpen()).toBe(false);
    expect(e.prevented()).toBe(false);
    expect(e.stopped()).toBe(false);

    dispose();
  });
});

describe("pane-select Esc cascade", () => {
  it("enters pane-select only after earlier Escape handlers decline", () => {
    const fake = installFakeWindow();
    const dispose = installKeybindings();
    const e = trackedKeyEvent({ key: "Escape", code: "Escape" });

    fake.dispatchCaptureKeydown(e.event);

    expect(paneSel()).toEqual({ kind: "pane", paneId: "main" });
    expect(e.prevented()).toBe(true);
    dispose();
  });

  it("lets focus mode consume Escape before pane-select entry", () => {
    const fake = installFakeWindow();
    const dispose = installKeybindings();
    const e = trackedKeyEvent({ key: "Escape", code: "Escape" });
    setFocusMode(true);

    fake.dispatchCaptureKeydown(e.event);

    expect(focusMode()).toBe(false);
    expect(paneSel()).toBeNull();
    expect(e.prevented()).toBe(true);
    dispose();
  });

  it("exits pane-select on the second Escape", () => {
    const fake = installFakeWindow();
    const dispose = installKeybindings();

    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "Escape", code: "Escape" }).event);
    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "Escape", code: "Escape" }).event);

    expect(paneSel()).toBeNull();
    dispose();
  });

  it("never intercepts keys while a text field has focus (stale mode)", () => {
    const fake = installFakeWindow();
    const dispose = installKeybindings();
    // Enter pane-select, then simulate the user having clicked into an editor
    // (a stale mode): typing must reach the textarea untouched.
    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "Escape", code: "Escape" }).event);
    expect(paneSel()).not.toBeNull();

    const typed = trackedKeyEvent({
      key: "a",
      code: "KeyA",
      target: { tagName: "TEXTAREA", isContentEditable: false } as unknown as EventTarget,
    });
    fake.dispatchCaptureKeydown(typed.event);

    expect(typed.prevented()).toBe(false);
    dispose();
  });

  it("cancels an embryo switcher by closing the materialized pane", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const embryo = splitRootAtEdge("right", "main")!;
    openSwitcher({ mode: "embryo", paneId: embryo, prefill: "x" });
    const fake = installFakeWindow();
    const dispose = installKeybindings();

    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "Escape", code: "Escape" }).event);

    expect(switcherOpen()).toBe(false);
    expect(layoutPaneIds()).toEqual(["main"]);
    dispose();
  });

  it("typing on a selected edge materializes an embryo pane that Escape unsplits", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const fake = installFakeWindow();
    const dispose = installKeybindings();

    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "Escape", code: "Escape" }).event);
    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "ArrowRight", code: "ArrowRight" }).event);
    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "z", code: "KeyZ" }).event);

    expect(switcherOpen()).toBe(true);
    expect(layoutPaneIds()).toHaveLength(2);

    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "Escape", code: "Escape" }).event);

    expect(switcherOpen()).toBe(false);
    expect(layoutPaneIds()).toEqual(["main"]);
    dispose();
  });

  it("Esc from block-select climbs STRAIGHT to pane-select (Martin's 2-rung ladder)", () => {
    const fake = installFakeWindow();
    const dispose = installKeybindings();
    selectBlock("some-block");

    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "Escape", code: "Escape" }).event);

    expect(paneSel()).toEqual({ kind: "pane", paneId: "main" });
    dispose();
  });

  it("Delete on a selected pane closes it and stays in the mode on the survivor", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const extra = splitRootAtEdge("right", "main")!;
    const fake = installFakeWindow();
    const dispose = installKeybindings();

    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "Escape", code: "Escape" }).event);
    expect(paneSel()).toEqual({ kind: "pane", paneId: extra }); // focus followed the split
    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "Delete", code: "Delete" }).event);

    expect(layoutPaneIds()).toEqual(["main"]);
    expect(paneSel()).toEqual({ kind: "pane", paneId: "main" });
    dispose();
  });

  it("Ctrl+K on a selected pane exits the mode and opens the switcher for THAT pane", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    splitRootAtEdge("right", "main");
    const fake = installFakeWindow();
    const dispose = installKeybindings();

    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "Escape", code: "Escape" }).event);
    const selected = paneSel();
    expect(selected?.kind).toBe("pane");
    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "k", code: "KeyK", ctrlKey: true }).event);

    expect(paneSel()).toBeNull();
    expect(switcherOpen()).toBe(true);
    // focus-follows-selection means the switcher acts on the selected pane
    expect(focusedPaneId()).toBe((selected as { paneId: string }).paneId);
    dispose();
  });

  it("typing on a pane-edge SEGMENT splits only that pane, not the root", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    splitRootAtEdge("right", "main"); // row [main, extra]
    focusPane("main");
    const fake = installFakeWindow();
    const dispose = installKeybindings();

    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "Escape", code: "Escape" }).event);
    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "ArrowUp", code: "ArrowUp" }).event);
    expect(paneSel()).toEqual({ kind: "pane-edge", paneId: "main", side: "top" });
    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "z", code: "KeyZ" }).event);

    // root is still the row split; only its LEFT branch became a col split
    const root = layoutRoot();
    expect(root.kind).toBe("split");
    if (root.kind === "split") {
      expect(root.dir).toBe("row");
      expect(root.children[0].kind).toBe("split");
      if (root.children[0].kind === "split") expect(root.children[0].dir).toBe("col");
      expect(root.children[1].kind).toBe("pane");
    }
    expect(layoutPaneIds()).toHaveLength(3);
    expect(switcherOpen()).toBe(true);
    dispose();
  });

  it("Enter on a selected edge makes a MIRROR split — no switcher (Martin's Jul 8 ruling)", () => {
    resetPaneLayoutToSingle(pageSnapshot("Source"));
    const fake = installFakeWindow();
    const dispose = installKeybindings();

    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "Escape", code: "Escape" }).event);
    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "ArrowRight", code: "ArrowRight" }).event);
    fake.dispatchCaptureKeydown(trackedKeyEvent({ key: "Enter", code: "Enter" }).event);

    expect(switcherOpen()).toBe(false); // mirror, not embryo
    expect(paneSel()).toBeNull();
    const ids = layoutPaneIds();
    expect(ids).toHaveLength(2);
    const newPane = ids.find((id) => id !== "main")!;
    // The new pane mirrors the source content and takes focus.
    const tabs = paneRouter(newPane).tabs();
    expect(tabs[0].history[tabs[0].pos]).toMatchObject({ kind: "page", name: "Source" });
    expect(focusedPaneId()).toBe(newPane);
    dispose();
  });
});
