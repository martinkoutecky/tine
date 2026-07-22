import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { installKeybindings } from "../keybindings";
import { paneRouter, resetPaneLayoutToSingle, restorePaneLayout } from "../panes";
import { setPaneSel, paneSel } from "../paneSelect";
import { type PaneSnapshot } from "../router";
import { resetStore } from "../store";
import { clearTransientLayersForTest, registerTransientLayer } from "../transientLayers";
import { TabBar } from "./TabBar";

const initialElementFromPoint = document.elementFromPoint;

const snapshot = (names: string[]): PaneSnapshot => ({
  tabs: names.map((name) => ({ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false })),
  activeIndex: 0,
});

function pointer(type: string, x: number, y: number, init: PointerEventInit = {}): PointerEvent {
  const Ctor = window.PointerEvent ?? MouseEvent;
  const event = new Ctor(type, {
    bubbles: true,
    cancelable: true,
    clientX: x,
    clientY: y,
    button: init.button ?? 0,
    buttons: init.buttons ?? 1,
  }) as PointerEvent;
  if (init.pointerId != null && event.pointerId !== init.pointerId) {
    Object.defineProperty(event, "pointerId", { value: init.pointerId });
  }
  return event;
}

function escape(init: { composing?: boolean; keyCode?: number } = {}) {
  const event = new KeyboardEvent("keydown", { key: "Escape", code: "Escape", bubbles: true, cancelable: true });
  if (init.composing) Object.defineProperty(event, "isComposing", { value: true });
  if (init.keyCode != null) Object.defineProperty(event, "keyCode", { value: init.keyCode });
  return event;
}

function mountOverflow() {
  const callbacks: ResizeObserverCallback[] = [];
  const OriginalResizeObserver = globalThis.ResizeObserver;
  globalThis.ResizeObserver = class {
    constructor(callback: ResizeObserverCallback) { callbacks.push(callback); }
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  resetPaneLayoutToSingle(snapshot(["A", "B", "C"]));
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <TabBar router={paneRouter("main")} />, root);
  const strip = root.querySelector<HTMLElement>(".tab-strip-scroll")!;
  const setOverflow = (overflow: boolean) => {
    Object.defineProperties(strip, {
      clientWidth: { configurable: true, value: 120 },
      scrollWidth: { configurable: true, value: overflow ? 480 : 120 },
    });
    callbacks.forEach((callback) => callback([], {} as ResizeObserver));
  };
  setOverflow(true);
  const trigger = root.querySelector<HTMLButtonElement>("[data-tab-overview-trigger]")!;
  trigger.click();
  return {
    root,
    trigger,
    setOverflow,
    dispose: () => {
      dispose();
      globalThis.ResizeObserver = OriginalResizeObserver;
    },
  };
}

function mountCrossPaneOverflows() {
  const callbacks: ResizeObserverCallback[] = [];
  const OriginalResizeObserver = globalThis.ResizeObserver;
  globalThis.ResizeObserver = class {
    constructor(callback: ResizeObserverCallback) { callbacks.push(callback); }
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  restorePaneLayout(
    { kind: "split", dir: "row", ratio: 0.5, children: [{ kind: "pane", paneId: "main" }, { kind: "pane", paneId: "pane-2" }] },
    new Map([["main", snapshot(["A", "B", "C"])], ["pane-2", snapshot(["X", "Y", "Z"])]]),
    "main",
  );
  const mainRoot = document.createElement("div");
  const replacementRoot = document.createElement("div");
  document.body.append(mainRoot, replacementRoot);
  const disposeMain = render(() => <TabBar router={paneRouter("main")} />, mainRoot);
  const disposeReplacement = render(() => <TabBar router={paneRouter("pane-2")} />, replacementRoot);
  for (const strip of document.querySelectorAll<HTMLElement>(".tab-strip-scroll")) {
    Object.defineProperties(strip, {
      clientWidth: { configurable: true, value: 120 },
      scrollWidth: { configurable: true, value: 480 },
    });
  }
  callbacks.forEach((callback) => callback([], {} as ResizeObserver));
  const triggers = [...document.querySelectorAll<HTMLButtonElement>("[data-tab-overview-trigger]")];
  triggers[0].click();
  triggers[1].click();
  return {
    replacementTrigger: triggers[1],
    disposeMain,
    dispose: () => {
      disposeReplacement();
      globalThis.ResizeObserver = OriginalResizeObserver;
    },
  };
}

function tabNames() {
  return paneRouter("main").tabs().map((tab) => {
    const route = tab.history[tab.pos];
    return route.kind === "page" ? route.name : "Journals";
  });
}

function armOverviewRowReorder(source = 0, target = 1, pointerId = 1) {
  const rows = [...document.querySelectorAll<HTMLElement>("[data-tab-overview-row]")];
  const handle = rows[source].querySelector<HTMLElement>(".tab-overview-drag-handle")!;
  const originalElementFromPoint = document.elementFromPoint;
  Object.defineProperty(rows[target], "getBoundingClientRect", {
    configurable: true,
    value: () => ({ top: 100, height: 32 }),
  });
  document.elementFromPoint = () => rows[target];
  handle.dispatchEvent(pointer("pointerdown", 10, 10, { pointerId }));
  window.dispatchEvent(pointer("pointermove", 10, 130, { pointerId }));
  return {
    release: () => window.dispatchEvent(pointer("pointerup", 10, 130, { buttons: 0, pointerId })),
    cancel: () => window.dispatchEvent(pointer("pointercancel", 10, 130, { buttons: 0, pointerId })),
    restore: () => { document.elementFromPoint = originalElementFromPoint; },
  };
}

afterEach(() => {
  clearTransientLayersForTest();
  setPaneSel(null);
  resetStore();
  document.body.innerHTML = "";
  expect(document.elementFromPoint).toBe(initialElementFromPoint);
});

describe("GH #161 P1E-O TabBar overview transient ownership", () => {
  it("keeps one Escape above a lower transient and out of the pane ladder", () => {
    const lower = vi.fn(() => true);
    const unregisterLower = registerTransientLayer({ id: "p1e-o-lower-sentinel", dismiss: lower });
    const { trigger, dispose } = mountOverflow();
    const row = document.querySelector<HTMLElement>("[data-tab-overview-row]")!;
    const uninstall = installKeybindings();
    try {
      const first = escape();
      row.dispatchEvent(first);

      expect(document.querySelector("[role=listbox]")).toBeNull();
      expect(lower).not.toHaveBeenCalled();
      expect(paneSel()).toBeNull();
      expect(first.defaultPrevented).toBe(true);

      const second = escape();
      trigger.dispatchEvent(second);
      expect(lower).toHaveBeenCalledTimes(1);
    } finally {
      uninstall();
      dispose();
      unregisterLower();
    }
  });

  it("closes the overview, and not the pane ladder, for ordinary Escape without a lower owner", () => {
    const { dispose } = mountOverflow();
    const row = document.querySelector<HTMLElement>("[data-tab-overview-row]")!;
    const uninstall = installKeybindings();
    try {
      const ordinary = escape();
      row.dispatchEvent(ordinary);

      expect(document.querySelector("[role=listbox]")).toBeNull();
      expect(paneSel()).toBeNull();
      expect(ordinary.defaultPrevented).toBe(true);
    } finally {
      uninstall();
      dispose();
    }
  });

  it("removes a lower sentinel's document listeners through its captured disposer", () => {
    const add = vi.spyOn(document, "addEventListener");
    const remove = vi.spyOn(document, "removeEventListener");
    const unregister = registerTransientLayer({ id: "p1e-o-disposal-sentinel", dismiss: () => true });
    let disposed = false;
    try {
      const registrations = add.mock.calls.filter(([type]) => type === "focusin" || type === "pointerdown");
      expect(registrations).toHaveLength(2);

      unregister();
      disposed = true;
      for (const [type, listener, options] of registrations) {
        expect(remove).toHaveBeenCalledWith(type, listener, options);
      }
    } finally {
      if (!disposed) unregister();
      add.mockRestore();
      remove.mockRestore();
    }
  });

  it("keeps an armed overview-row session live for composing and keyCode-229 Escape", () => {
    for (const [label, init] of [["composing", { composing: true }], ["keyCode-229", { keyCode: 229 }]] as const) {
      const { dispose } = mountOverflow();
      const row = document.querySelector<HTMLElement>("[data-tab-overview-row]")!;
      const drag = armOverviewRowReorder();
      const uninstall = installKeybindings();
      try {
        row.focus();
        const imeEscape = escape(init);
        row.dispatchEvent(imeEscape);

        expect(document.querySelector("[role=listbox]")).not.toBeNull();
        expect(document.activeElement).toBe(row);
        expect(paneSel()).toBeNull();
        expect(imeEscape.defaultPrevented, label).toBe(false);

        drag.release();
        expect(tabNames(), label).toEqual(["B", "A", "C"]);
      } finally {
        uninstall();
        drag.restore();
        dispose();
      }
    }
  });

  it("does not let a late overview row pointer-up reorder after trigger dismissal", () => {
    const { trigger, dispose } = mountOverflow();
    const drag = armOverviewRowReorder();
    try {
      trigger.click();
      drag.release();

      expect(tabNames()).toEqual(["A", "B", "C"]);
    } finally {
      drag.restore();
      dispose();
    }
  });

  it("retires armed row reorders on activation, outside dismissal, overflow loss, cancellation, replacement, and disposal", () => {
    const activation = mountOverflow();
    const activationDrag = armOverviewRowReorder();
    try {
      document.querySelectorAll<HTMLElement>("[data-tab-overview-row]")[2].click();
      activationDrag.release();
      expect(tabNames()).toEqual(["A", "B", "C"]);
    } finally {
      activationDrag.restore();
      activation.dispose();
    }

    const outside = mountOverflow();
    const outsideDrag = armOverviewRowReorder();
    try {
      document.body.dispatchEvent(pointer("pointerdown", 500, 500));
      outsideDrag.release();
      expect(tabNames()).toEqual(["A", "B", "C"]);
    } finally {
      outsideDrag.restore();
      outside.dispose();
    }

    const overflowLoss = mountOverflow();
    const overflowDrag = armOverviewRowReorder();
    try {
      overflowLoss.setOverflow(false);
      overflowDrag.release();
      expect(tabNames()).toEqual(["A", "B", "C"]);
    } finally {
      overflowDrag.restore();
      overflowLoss.dispose();
    }

    const pointerCancel = mountOverflow();
    const cancelDrag = armOverviewRowReorder();
    try {
      cancelDrag.cancel();
      cancelDrag.release();
      expect(tabNames()).toEqual(["A", "B", "C"]);
    } finally {
      cancelDrag.restore();
      pointerCancel.dispose();
    }

    const replacement = mountOverflow();
    const first = armOverviewRowReorder();
    const second = armOverviewRowReorder(1, 2, 2);
    try {
      first.release();
      second.release();
      expect(tabNames()).toEqual(["A", "C", "B"]);
    } finally {
      second.restore();
      first.restore();
      replacement.dispose();
    }

    const disposal = mountOverflow();
    const disposalDrag = armOverviewRowReorder();
    try {
      disposal.dispose();
      disposalDrag.release();
      expect(tabNames()).toEqual(["A", "B", "C"]);
    } finally {
      disposalDrag.restore();
    }
  });

  it("uses registry Escape for row retirement and queued trigger restoration", async () => {
    const { trigger, dispose } = mountOverflow();
    const drag = armOverviewRowReorder();
    const uninstall = installKeybindings();
    try {
      document.querySelector<HTMLElement>("[data-tab-overview-row]")!.dispatchEvent(escape());
      drag.release();
      await Promise.resolve();

      expect(document.querySelector("[role=listbox]")).toBeNull();
      expect(tabNames()).toEqual(["A", "B", "C"]);
      expect(document.activeElement).toBe(trigger);
    } finally {
      uninstall();
      drag.restore();
      dispose();
    }
  });

  it("keeps a cross-pane replacement overview live when stale disposal removes the older owner", () => {
    const lower = vi.fn(() => true);
    const unregisterLower = registerTransientLayer({ id: "p1e-o-cross-pane-lower", dismiss: lower });
    const { replacementTrigger, disposeMain, dispose } = mountCrossPaneOverflows();
    const uninstall = installKeybindings();
    try {
      disposeMain();
      const replacement = document.querySelectorAll<HTMLElement>("[role=listbox]")[0];
      replacement.dispatchEvent(escape());

      expect(document.querySelector("[role=listbox]")).toBeNull();
      expect(lower).not.toHaveBeenCalled();
      replacementTrigger.dispatchEvent(escape());
      expect(lower).toHaveBeenCalledTimes(1);
    } finally {
      uninstall();
      dispose();
      unregisterLower();
    }
  });
});
