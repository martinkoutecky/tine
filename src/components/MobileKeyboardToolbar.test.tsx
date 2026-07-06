import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";

function mount(node: () => JSX.Element) {
  const div = document.createElement("div");
  document.body.appendChild(div);
  const dispose = render(node, div);
  return { div, dispose };
}

function mockVisualViewport() {
  Object.defineProperty(window, "innerHeight", { value: 800, configurable: true });
  Object.defineProperty(window, "visualViewport", {
    value: {
      height: 520,
      offsetTop: 0,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    },
    configurable: true,
  });
}

describe("MobileKeyboardToolbar", () => {
  beforeEach(() => {
    vi.resetModules();
    document.body.innerHTML = "";
    mockVisualViewport();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    document.body.innerHTML = "";
  });

  it("renders the confirmed scroll strip and pinned hide button on Android", async () => {
    vi.doMock("../nativeChrome", () => ({ isMobilePlatform: true }));
    const bridge = await import("../editorCommandBridge");
    const { MobileKeyboardToolbar } = await import("./MobileKeyboardToolbar");
    const calls: string[] = [];
    const unregister = bridge.registerFocusedEditorCommandBridge({
      blockId: "b1",
      dispatch(command) {
        calls.push(command);
        return true;
      },
      blur() {
        calls.push("blur");
      },
    });
    const { div, dispose } = mount(() => <MobileKeyboardToolbar />);

    const toolbar = div.querySelector("[data-mobile-keyboard-toolbar]") as HTMLElement | null;
    expect(toolbar).not.toBeNull();
    expect(toolbar?.style.top).toContain("520px");

    const stripButtons = Array.from(
      toolbar!.querySelectorAll<HTMLButtonElement>(".mobile-keyboard-toolbar-strip .mobile-keyboard-toolbar-btn")
    );
    expect(stripButtons.map((button) => button.getAttribute("aria-label"))).toEqual([
      "Outdent",
      "Indent",
      "Move block up",
      "Move block down",
      "Soft newline",
      "TODO",
      "Camera",
      "Voice memo",
      "Undo",
      "Redo",
      "Date picker",
      "Page reference",
      "Block reference",
      "Slash menu",
    ]);

    const hide = toolbar!.querySelector<HTMLButtonElement>(".mobile-keyboard-toolbar-hide");
    expect(hide?.closest(".mobile-keyboard-toolbar-strip")).toBeNull();
    expect(hide?.getAttribute("aria-label")).toBe("Hide keyboard");

    const down = new Event("pointerdown", { bubbles: true, cancelable: true });
    stripButtons[0].dispatchEvent(down);
    expect(down.defaultPrevented).toBe(true);
    stripButtons[0].click();
    expect(calls).toEqual(["editor/outdent"]);

    const hideDown = new Event("pointerdown", { bubbles: true, cancelable: true });
    hide!.dispatchEvent(hideDown);
    expect(hideDown.defaultPrevented).toBe(true);
    expect(calls).toEqual(["editor/outdent", "blur"]);

    unregister();
    dispose();
  });

  it("does not render on desktop even with a focused editor bridge", async () => {
    vi.doMock("../nativeChrome", () => ({ isMobilePlatform: false }));
    const bridge = await import("../editorCommandBridge");
    const { MobileKeyboardToolbar } = await import("./MobileKeyboardToolbar");
    const unregister = bridge.registerFocusedEditorCommandBridge({
      blockId: "b1",
      dispatch: () => true,
      blur() {},
    });
    const { div, dispose } = mount(() => <MobileKeyboardToolbar />);

    expect(div.querySelector("[data-mobile-keyboard-toolbar]")).toBeNull();

    unregister();
    dispose();
  });
});
