import { afterEach, describe, expect, it } from "vitest";
import { closeInPageFind, inPageFindOpen } from "./inpageFind";
import { commandDefaults, eventToBindingString, installKeybindings } from "./keybindings";
import { setPdfTarget } from "./ui";

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

afterEach(() => {
  setPdfTarget(null);
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

  it("binds developer tools to mod+shift+i as a global command (#31)", () => {
    const byId = Object.fromEntries(commandDefaults().map((c) => [c.id, c]));
    expect(byId["ui/toggle-devtools"]).toMatchObject({ binding: "mod+shift+i", scope: "global" });
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
