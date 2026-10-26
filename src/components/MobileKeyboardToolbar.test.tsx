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

    // GH #599: visual golden for the selected conventional return-arrow glyphs.
    // The Chromium fixture also paints these at the actual phone toolbar size.
    expect(stripButtons.slice(8, 10).map((button) => button.querySelector("path")?.getAttribute("d")))
      .toEqual(["M9 14l-4-4 4-4M5 10h10a4 4 0 0 1 0 8h-2", "M15 14l4-4-4-4M19 10H9a4 4 0 0 0 0 8h2"]);

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

  it.each(["main-content", "right-sidebar", "left-sidebar-scroll"])("reveals the focused editor in its own %s scroller (GH #384, GH #751)", async (surface) => {
    const { revealFocusedEditorAboveToolbar } = await import("./MobileKeyboardToolbar");
    const scroller = document.createElement("main");
    scroller.className = surface;
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

  it("routes drawer toolbar actions to the current editor after a pane/left/right handoff (GH #751)", async () => {
    const { render } = await import("solid-js/web");
    const bridge = await import("../editorCommandBridge");
    const { MobileKeyboardToolbar } = await import("./MobileKeyboardToolbar");
    const { div, dispose } = mount(render, MobileKeyboardToolbar);
    const calls: string[] = [];
    for (const surface of ["main-content", "left-sidebar-scroll", "right-sidebar"]) {
      const host = document.createElement("div");
      host.className = surface;
      const editor = document.createElement("textarea");
      editor.className = "block-editor";
      host.append(editor);
      document.body.append(host);
      editor.focus();
      const unregister = bridge.registerFocusedEditorCommandBridge({
        blockId: surface,
        dispatch(command) { calls.push(`${surface}:${command}`); return true; },
        blur() { editor.blur(); },
      });
      await Promise.resolve();
      div.querySelector<HTMLButtonElement>('[aria-label="TODO"]')!.click();
      expect(document.activeElement).toBe(editor);
      unregister();
      host.remove();
    }
    expect(calls).toEqual(["main-content:editor/cycle-todo", "left-sidebar-scroll:editor/cycle-todo", "right-sidebar:editor/cycle-todo"]);
    dispose();
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

  it("marks the editing dock on the root while it is up, so the bottom navigation row yields (GH #598)", async () => {
    platform.mobile = true;
    const { render } = await import("solid-js/web");
    const bridge = await import("../editorCommandBridge");
    const { MobileKeyboardToolbar } = await import("./MobileKeyboardToolbar");
    const root = document.documentElement;
    const { dispose } = mount(render, MobileKeyboardToolbar);
    expect(root.hasAttribute("data-mobile-editing-dock")).toBe(false);

    const unregister = bridge.registerFocusedEditorCommandBridge({ blockId: "b1", dispatch: () => true, blur() {} });
    await Promise.resolve();
    expect(root.hasAttribute("data-mobile-editing-dock")).toBe(true);

    unregister();
    await Promise.resolve();
    expect(root.hasAttribute("data-mobile-editing-dock")).toBe(false);
    dispose();
    expect(root.hasAttribute("data-mobile-editing-dock")).toBe(false);
  });

  // GH #716. `layout` is the window's innerHeight, `visual` the visual
  // viewport's bottom. Android pads the WebView above the IME, so there the IME
  // and a window resize both move `layout`; only MainActivity's `tine-ime`
  // event tells them apart. iOS keeps `layout` and shrinks `visual`.
  async function keyboardProbe(initial: number) {
    let layout = initial;
    let visual = initial;
    Object.defineProperty(window, "innerHeight", { get: () => layout, configurable: true });
    Object.defineProperty(window, "visualViewport", {
      value: { get height() { return visual; }, offsetTop: 0, addEventListener: vi.fn(), removeEventListener: vi.fn() },
      configurable: true,
    });
    platform.mobile = true;
    const { render } = await import("solid-js/web");
    const bridge = await import("../editorCommandBridge");
    const { MobileKeyboardToolbar, NATIVE_IME_EVENT } = await import("./MobileKeyboardToolbar");
    const editor = document.createElement("textarea");
    editor.className = "block-editor";
    document.body.appendChild(editor);
    const { div, dispose } = mount(render, MobileKeyboardToolbar);
    let unregister = () => {};
    const settle = () => Promise.resolve();
    return {
      async focus() {
        editor.focus();
        unregister = bridge.registerFocusedEditorCommandBridge({ blockId: "b1", dispatch: () => true, blur() {} });
        await settle();
      },
      /** A window resize, or the Android IME moving the padded viewport. */
      async resize(layoutHeight: number, visualHeight = layoutHeight) {
        layout = layoutHeight;
        visual = visualHeight;
        window.dispatchEvent(new Event("resize"));
        await settle();
      },
      async nativeIme(visible: boolean) {
        window.dispatchEvent(new CustomEvent(NATIVE_IME_EVENT, { detail: visible }));
        await settle();
      },
      dock() {
        const shown = document.documentElement.hasAttribute("data-mobile-editing-dock");
        const toolbar = div.querySelector<HTMLElement>("[data-mobile-keyboard-toolbar]")!;
        expect(toolbar.hidden).toBe(!shown);
        return shown;
      },
      editorFocused: () => document.activeElement === editor,
      cleanup() { unregister(); dispose(); editor.remove(); },
    };
  }

  it("gives the bottom row back once the Android keyboard is dismissed while the editor keeps focus (GH #716)", async () => {
    const probe = await keyboardProbe(800);
    try {
      await probe.focus();
      // Focused, keyboard not (yet) seen: the dock stays available (hardware keyboard).
      expect(probe.dock()).toBe(true);
      await probe.nativeIme(true);
      await probe.resize(500);
      expect(probe.dock()).toBe(true);
      // Back / the IME's own hide key: the keyboard goes, the editor stays focused.
      await probe.nativeIme(false);
      await probe.resize(800);
      expect(probe.editorFocused()).toBe(true);
      expect(probe.dock()).toBe(false);
      // Tapping the editor brings the keyboard, and the dock, back.
      await probe.nativeIme(true);
      await probe.resize(500);
      expect(probe.dock()).toBe(true);
    } finally { probe.cleanup(); }
  });

  it("gives the bottom row back once the iOS keyboard is dismissed while the editor keeps focus (GH #716)", async () => {
    const probe = await keyboardProbe(800);
    try {
      await probe.focus();
      await probe.resize(800, 500);
      expect(probe.dock()).toBe(true);
      await probe.resize(800, 800);
      expect(probe.editorFocused()).toBe(true);
      expect(probe.dock()).toBe(false);
    } finally { probe.cleanup(); }
  });

  it("keeps the hardware-keyboard dock through a height-only window resize (GH #716 review)", async () => {
    // Android before and after MainActivity's first report, and iOS: no IME is used.
    for (const reported of [false, true]) {
      const probe = await keyboardProbe(900);
      try {
        if (reported) await probe.nativeIme(false);
        await probe.focus();
        expect(probe.dock()).toBe(true);
        await probe.resize(700);
        expect(probe.dock()).toBe(true);
        await probe.resize(900);
        expect(probe.dock(), `no IME was used (native report: ${reported})`).toBe(true);
      } finally { probe.cleanup(); }
    }
  });

  it("keeps the dock while the IME stays open and the window grows (GH #716 review)", async () => {
    for (const reported of [false, true]) {
      const probe = await keyboardProbe(900);
      try {
        await probe.focus();
        if (reported) await probe.nativeIme(true);
        await probe.resize(600); // 900px window minus a 300px IME
        expect(probe.dock()).toBe(true);
        await probe.resize(900); // window enlarged to 1200px, the IME still open
        expect(probe.dock(), `the IME is still open (native report: ${reported})`).toBe(true);
      } finally { probe.cleanup(); }
    }
  });

  it("gives the bottom row back after the IME closes in a window made shorter (GH #716 review)", async () => {
    const probe = await keyboardProbe(900);
    try {
      await probe.resize(700); // the window, not a keyboard
      await probe.focus();
      await probe.nativeIme(true);
      await probe.resize(400);
      expect(probe.dock()).toBe(true);
      await probe.nativeIme(false);
      await probe.resize(700);
      expect(probe.dock(), "the keyboard is closed in the current 700px window").toBe(false);
    } finally { probe.cleanup(); }
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
