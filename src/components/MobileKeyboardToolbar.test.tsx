import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { JSX } from "solid-js";

const platform = vi.hoisted(() => ({ mobile: true }));
vi.mock("../nativeChrome", () => ({
  get isMobilePlatform() {
    return platform.mobile;
  },
}));

type Render = typeof import("solid-js/web")["render"];

function mount(render: Render, node: () => JSX.Element) {
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

// jsdom has no PointerEvent in every environment; MouseEvent carries the same
// coordinates and the handlers only read pointerId/isPrimary beyond that.
function pointer(type: string, init: { pointerId?: number; isPrimary?: boolean } = {}): Event {
  const Ctor = (window as { PointerEvent?: typeof PointerEvent }).PointerEvent ?? MouseEvent;
  const event = new Ctor(type, { bubbles: true, cancelable: true, clientX: 5, clientY: 5, button: 0 });
  Object.defineProperty(event, "pointerId", { value: init.pointerId ?? 1, configurable: true });
  Object.defineProperty(event, "isPrimary", { value: init.isPrimary ?? true, configurable: true });
  return event;
}

describe("MobileKeyboardToolbar", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    mockVisualViewport();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    document.body.innerHTML = "";
  });

  it("renders the confirmed scroll strip and pinned hide button on Android", async () => {
    platform.mobile = true;
    const { render } = await import("solid-js/web");
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
    const { div, dispose } = mount(render, MobileKeyboardToolbar);

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

  it("moves the focused editor fully above the toolbar without disturbing an already-visible block (GH #384)", async () => {
    const { revealFocusedEditorAboveToolbar } = await import("./MobileKeyboardToolbar");
    const scroller = document.createElement("main");
    scroller.className = "main-content";
    const editor = document.createElement("textarea");
    editor.className = "block-editor";
    scroller.appendChild(editor);
    document.body.appendChild(scroller);
    editor.focus();

    editor.getBoundingClientRect = () => ({
      top: 470, bottom: 540, left: 0, right: 300, width: 300, height: 70, x: 0, y: 470,
      toJSON() { return {}; },
    });
    expect(revealFocusedEditorAboveToolbar(520)).toBe(true);
    expect(scroller.scrollTop).toBe(28); // 20px overlap + 8px breathing room

    editor.getBoundingClientRect = () => ({
      top: 420, bottom: 500, left: 0, right: 300, width: 300, height: 80, x: 0, y: 420,
      toJSON() { return {}; },
    });
    expect(revealFocusedEditorAboveToolbar(520)).toBe(false);
    expect(scroller.scrollTop).toBe(28);
  });

  it("consumes the full hide-keyboard gesture instead of touching through (GH #336)", async () => {
    platform.mobile = true;
    const { render } = await import("solid-js/web");
    const bridge = await import("../editorCommandBridge");
    const { MobileKeyboardToolbar } = await import("./MobileKeyboardToolbar");
    const calls: string[] = [];
    const unregister = bridge.registerFocusedEditorCommandBridge({
      blockId: "b1",
      dispatch: () => true,
      blur() {
        calls.push("blur");
      },
    });
    // The "block underneath": anything the trailing synthesized click could hit.
    const probe = document.createElement("button");
    let probeClicks = 0;
    probe.onclick = () => probeClicks++;
    document.body.appendChild(probe);
    const { div, dispose } = mount(render, MobileKeyboardToolbar);
    const toolbarOf = () => div.querySelector("[data-mobile-keyboard-toolbar]:not([hidden])");
    expect(toolbarOf()).not.toBeNull();

    // pointerdown blurs (keyboard starts hiding) — unchanged behavior.
    const hide = toolbarOf()!.querySelector<HTMLButtonElement>(".mobile-keyboard-toolbar-hide")!;
    const down = new Event("pointerdown", { bubbles: true, cancelable: true });
    hide.dispatchEvent(down);
    expect(down.defaultPrevented).toBe(true);
    expect(calls).toEqual(["blur"]);

    // The IME actually closes: viewport grows back to full height. On Android
    // this is exactly when the toolbar used to vanish mid-gesture.
    Object.defineProperty(window, "visualViewport", {
      value: { height: 800, offsetTop: 0, addEventListener: vi.fn(), removeEventListener: vi.fn() },
      configurable: true,
    });
    window.dispatchEvent(new Event("resize"));
    expect(toolbarOf()).not.toBeNull();
    expect((toolbarOf() as HTMLElement).style.top).toContain("520px");

    // The gesture's trailing click lands on the still-mounted button and is
    // swallowed there; the page underneath never sees it.
    const hideStill = toolbarOf()!.querySelector<HTMLButtonElement>(".mobile-keyboard-toolbar-hide")!;
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    hideStill.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
    expect(calls).toEqual(["blur"]);
    expect(probeClicks).toBe(0);

    // Once the click is consumed, the toolbar retires with the hidden keyboard.
    expect(toolbarOf()).toBeNull();

    probe.remove();
    unregister();
    dispose();
  });

  it("releases a cancelled hide-keyboard gesture after its safety window", async () => {
    platform.mobile = true;
    const { render } = await import("solid-js/web");
    const bridge = await import("../editorCommandBridge");
    const { MobileKeyboardToolbar } = await import("./MobileKeyboardToolbar");
    const unregister = bridge.registerFocusedEditorCommandBridge({
      blockId: "b1",
      dispatch: () => true,
      blur() {},
    });
    const { div, dispose } = mount(render, MobileKeyboardToolbar);
    const toolbarOf = () => div.querySelector("[data-mobile-keyboard-toolbar]:not([hidden])");

    const hide = toolbarOf()!.querySelector<HTMLButtonElement>(".mobile-keyboard-toolbar-hide")!;
    vi.useFakeTimers();
    hide.dispatchEvent(new Event("pointerdown", { bubbles: true, cancelable: true }));
    Object.defineProperty(window, "visualViewport", {
      value: { height: 800, offsetTop: 0, addEventListener: vi.fn(), removeEventListener: vi.fn() },
      configurable: true,
    });
    window.dispatchEvent(new Event("resize"));
    expect(toolbarOf()).not.toBeNull();

    // pointercancel / lost events: no click ever arrives, so the safety window
    // retires the gesture instead of leaking a permanently-visible toolbar.
    vi.advanceTimersByTime(800);
    expect(toolbarOf()).toBeNull();
    vi.useRealTimers();

    unregister();
    dispose();
  });

  // GH #434: on iOS 27 the toolbar painted but nothing responded. The buttons
  // acted only on the compatibility `click` that trails a pointer sequence
  // opened by a preventDefault()ed pointerdown — a WebKit that declines to
  // synthesize that click left them inert. The pointer sequence is the contract
  // now; `click` remains the mouse/keyboard/AT path, and the two never
  // double-fire.
  it("runs a toolbar action from the pointer sequence alone, with no trailing click", async () => {
    platform.mobile = true;
    const { render } = await import("solid-js/web");
    const bridge = await import("../editorCommandBridge");
    const { MobileKeyboardToolbar } = await import("./MobileKeyboardToolbar");
    const calls: string[] = [];
    const unregister = bridge.registerFocusedEditorCommandBridge({
      blockId: "b1",
      dispatch(command) {
        calls.push(command);
        return true;
      },
      blur() {},
    });
    const { div, dispose } = mount(render, MobileKeyboardToolbar);
    const indent = div.querySelector<HTMLButtonElement>('[aria-label="Indent"]')!;

    indent.dispatchEvent(pointer("pointerdown"));
    indent.dispatchEvent(pointer("pointerup"));
    expect(calls).toEqual(["editor/indent"]);

    // The click a cooperative WebView still emits must not run it a second time.
    indent.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }));
    expect(calls).toEqual(["editor/indent"]);

    // Keyboard and assistive technology deliver a bare click; it must still work.
    indent.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    expect(calls).toEqual(["editor/indent", "editor/indent"]);

    unregister();
    dispose();
  });

  it("keeps toolbar ownership across an editor handoff and consumes a retargeted click", async () => {
    const { render } = await import("solid-js/web");
    const bridge = await import("../editorCommandBridge");
    const { MobileKeyboardToolbar } = await import("./MobileKeyboardToolbar");
    const calls: string[] = [];
    let unregister = () => {};
    const register = () => bridge.registerFocusedEditorCommandBridge({
      blockId: "current",
      dispatch(command) {
        calls.push(command);
        unregister();
        unregister = register();
        return true;
      },
      blur() {},
    });
    unregister = register();
    const { div, dispose } = mount(render, MobileKeyboardToolbar);
    const toolbar = div.querySelector("[data-mobile-keyboard-toolbar]")!;
    const strip = toolbar.querySelector<HTMLElement>(".mobile-keyboard-toolbar-strip")!;
    strip.scrollLeft = 80;
    const indent = toolbar.querySelector<HTMLButtonElement>('[aria-label="Indent"]')!;
    indent.dispatchEvent(pointer("pointerdown"));
    indent.dispatchEvent(pointer("pointerup"));
    expect(div.querySelector("[data-mobile-keyboard-toolbar]")).toBe(toolbar);
    expect(strip.scrollLeft).toBe(80);
    // Android can target a different button after the editor moves. The
    // completed pointer owns this click regardless of its target.
    const outdent = div.querySelector<HTMLButtonElement>('[aria-label="Outdent"]')!;
    outdent.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }));
    expect(calls).toEqual(["editor/indent"]);
    // A real second gesture is never suppressed by the first one's age.
    outdent.dispatchEvent(pointer("pointerdown", { pointerId: 2 }));
    outdent.dispatchEvent(pointer("pointerup", { pointerId: 2 }));
    expect(calls).toEqual(["editor/indent", "editor/outdent"]);
    // No compatibility click is required; keyboard/AT still acts immediately.
    outdent.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    expect(calls).toEqual(["editor/indent", "editor/outdent", "editor/outdent"]);
    const outside = document.createElement("button");
    const outsideClick = vi.fn();
    outside.onclick = outsideClick; document.body.append(outside);
    outside.dispatchEvent(pointer("pointerdown", { pointerId: 3 }));
    outside.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }));
    expect(outsideClick).toHaveBeenCalledOnce();
    outside.remove(); unregister(); dispose();
  });

  it("treats a press that slides off the button as a cancel", async () => {
    platform.mobile = true;
    const { render } = await import("solid-js/web");
    const bridge = await import("../editorCommandBridge");
    const { MobileKeyboardToolbar } = await import("./MobileKeyboardToolbar");
    const calls: string[] = [];
    const unregister = bridge.registerFocusedEditorCommandBridge({
      blockId: "b1",
      dispatch(command) {
        calls.push(command);
        return true;
      },
      blur() {},
    });
    const { div, dispose } = mount(render, MobileKeyboardToolbar);
    const indent = div.querySelector<HTMLButtonElement>('[aria-label="Indent"]')!;
    vi.spyOn(indent, "getBoundingClientRect").mockReturnValue({
      x: 0, y: 0, top: 0, left: 0, right: 40, bottom: 40, width: 40, height: 40,
      toJSON: () => ({}),
    });

    indent.dispatchEvent(pointer("pointerdown"));
    const away = pointer("pointerup");
    Object.defineProperty(away, "clientX", { value: 400, configurable: true });
    Object.defineProperty(away, "clientY", { value: 400, configurable: true });
    indent.dispatchEvent(away);
    expect(calls).toEqual([]);

    unregister();
    dispose();
  });

  it("does not render on desktop even with a focused editor bridge", async () => {
    platform.mobile = false;
    const { render } = await import("solid-js/web");
    const bridge = await import("../editorCommandBridge");
    const { MobileKeyboardToolbar } = await import("./MobileKeyboardToolbar");
    const unregister = bridge.registerFocusedEditorCommandBridge({
      blockId: "b1",
      dispatch: () => true,
      blur() {},
    });
    const { div, dispose } = mount(render, MobileKeyboardToolbar);

    expect(div.querySelector("[data-mobile-keyboard-toolbar]")).toBeNull();

    unregister();
    dispose();
  });
});
