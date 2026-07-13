import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { PaneTree } from "../App";
import { layoutPaneIds, layoutRoot, paneRouter, resetPaneLayoutToSingle, restorePaneLayout } from "../panes";
import { resetStore } from "../store";
import { tabRoute, type PaneSnapshot } from "../router";
import { TabBar } from "./TabBar";

const pageSnapshot = (names: string[], activeIndex = 0): PaneSnapshot => ({
  tabs: names.map((name) => ({ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false })),
  activeIndex,
});

const journalsSnapshot = (): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

function rect(left: number, top: number, width: number, height: number): DOMRect {
  return {
    x: left,
    y: top,
    left,
    top,
    width,
    height,
    right: left + width,
    bottom: top + height,
    toJSON: () => ({}),
  } as DOMRect;
}

function setRect(el: Element | null, left: number, top: number, width: number, height: number) {
  Object.defineProperty(el, "getBoundingClientRect", {
    configurable: true,
    value: () => rect(left, top, width, height),
  });
}

function pointer(type: string, x: number, y: number, init: PointerEventInit = {}): PointerEvent {
  const Ctor = window.PointerEvent ?? MouseEvent;
  return new Ctor(type, {
    bubbles: true,
    cancelable: true,
    clientX: x,
    clientY: y,
    button: init.button ?? 0,
    buttons: init.buttons ?? 1,
  }) as PointerEvent;
}

function namesForPane(paneId: string): string[] {
  return paneRouter(paneId).tabs().map((t) => {
    const r = tabRoute(t);
    return r.kind === "page" ? r.name : "Journals";
  });
}

function renderSplit(mainNames = ["A", "B"], otherNames = ["X", "Y"]) {
  restorePaneLayout(
    {
      kind: "split",
      dir: "row",
      ratio: 0.5,
      children: [
        { kind: "pane", paneId: "main" },
        { kind: "pane", paneId: "pane-2" },
      ],
    },
    new Map([
      ["main", pageSnapshot(mainNames)],
      ["pane-2", pageSnapshot(otherNames)],
    ]),
    "main"
  );
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <PaneTree node={layoutRoot()} path={[]} />, root);
  return { root, dispose };
}

function tab(root: ParentNode, paneId: string, index: number): HTMLElement {
  return root.querySelectorAll<HTMLElement>(`[data-pane-id="${paneId}"] .tab`)[index];
}

afterEach(() => {
  document.body.innerHTML = "";
  resetStore();
  resetPaneLayoutToSingle(journalsSnapshot());
});

describe("TabBar pointer tab drag", () => {
  it("keeps plain clicks under the drag threshold as tab activation and cancels window drag on tabs", () => {
    resetPaneLayoutToSingle(pageSnapshot(["A", "B"]));
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <TabBar router={paneRouter("main")} />, root);
    const strip = root.querySelector(".tab-bar") as HTMLElement;
    const second = root.querySelectorAll<HTMLElement>(".tab")[1];

    expect(strip.hasAttribute("data-tauri-drag-region")).toBe(true);
    expect(second.hasAttribute("data-tauri-drag-region")).toBe(false);

    const down = pointer("pointerdown", 50, 10);
    second.dispatchEvent(down);
    window.dispatchEvent(pointer("pointermove", 52, 10));
    window.dispatchEvent(pointer("pointerup", 52, 10, { buttons: 0 }));
    second.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));

    expect(down.defaultPrevented).toBe(true);
    expect(paneRouter("main").activeId()).toBe(paneRouter("main").tabs()[1].id);
    dispose();
  });

  it("reorders a tab within the same strip after the pointer threshold", () => {
    const { root, dispose } = renderSplit(["A", "B", "C"], ["X"]);
    const first = tab(root, "main", 0);
    const third = tab(root, "main", 2);
    setRect(third, 100, 0, 50, 24);
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => third;

    first.dispatchEvent(pointer("pointerdown", 10, 10));
    window.dispatchEvent(pointer("pointermove", 140, 10));
    window.dispatchEvent(pointer("pointerup", 140, 10, { buttons: 0 }));

    expect(namesForPane("main")).toEqual(["B", "C", "A"]);
    document.elementFromPoint = prevElementFromPoint;
    dispose();
  });

  it("adopts a dragged tab into another strip at the hit index", () => {
    const { root, dispose } = renderSplit(["A", "B"], ["X", "Y"]);
    const dragged = tab(root, "main", 1);
    const target = tab(root, "pane-2", 0);
    setRect(target, 200, 0, 60, 24);
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => target;

    dragged.dispatchEvent(pointer("pointerdown", 70, 10));
    window.dispatchEvent(pointer("pointermove", 205, 10));
    window.dispatchEvent(pointer("pointerup", 205, 10, { buttons: 0 }));

    expect(namesForPane("main")).toEqual(["A"]);
    expect(namesForPane("pane-2")).toEqual(["B", "X", "Y"]);
    document.elementFromPoint = prevElementFromPoint;
    dispose();
  });

  it("drops on a pane body by appending and activating the tab in that pane", () => {
    const { root, dispose } = renderSplit(["A", "B"], ["X"]);
    const dragged = tab(root, "main", 1);
    const targetPane = root.querySelector('[data-pane-id="pane-2"]') as HTMLElement;
    const body = root.querySelector('[data-pane-id="pane-2"] .pane-main-content') as HTMLElement;
    setRect(targetPane, 100, 0, 300, 300);
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => body;

    dragged.dispatchEvent(pointer("pointerdown", 70, 10));
    window.dispatchEvent(pointer("pointermove", 240, 140));
    window.dispatchEvent(pointer("pointerup", 240, 140, { buttons: 0 }));

    expect(namesForPane("main")).toEqual(["A"]);
    expect(namesForPane("pane-2")).toEqual(["X", "B"]);
    expect(tabRoute(paneRouter("pane-2").activeTab())).toMatchObject({ kind: "page", name: "B" });
    document.elementFromPoint = prevElementFromPoint;
    dispose();
  });

  it("drops on a seam by creating a split and moving the tab into the new pane", () => {
    const { root, dispose } = renderSplit(["A", "B"], ["X"]);
    const dragged = tab(root, "main", 1);
    const seam = root.querySelector(".pane-resizer") as HTMLElement;
    setRect(seam, 100, 0, 6, 300);
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => seam;

    dragged.dispatchEvent(pointer("pointerdown", 70, 10));
    window.dispatchEvent(pointer("pointermove", 102, 140));
    window.dispatchEvent(pointer("pointerup", 102, 140, { buttons: 0 }));

    const created = layoutPaneIds().find((id) => id !== "main" && id !== "pane-2")!;
    expect(created).toBeTruthy();
    expect(namesForPane("main")).toEqual(["A"]);
    expect(namesForPane(created)).toEqual(["B"]);
    document.elementFromPoint = prevElementFromPoint;
    dispose();
  });

  it("Escape cancels an armed tab drag before drop", () => {
    const { root, dispose } = renderSplit(["A", "B", "C"], ["X"]);
    const first = tab(root, "main", 0);
    const third = tab(root, "main", 2);
    setRect(third, 100, 0, 50, 24);
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => third;

    first.dispatchEvent(pointer("pointerdown", 10, 10));
    window.dispatchEvent(pointer("pointermove", 140, 10));
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    window.dispatchEvent(pointer("pointerup", 140, 10, { buttons: 0 }));

    expect(namesForPane("main")).toEqual(["A", "B", "C"]);
    document.elementFromPoint = prevElementFromPoint;
    dispose();
  });
});

describe("TabBar overflow overview", () => {
  it("keeps readable tabs scrollable and exposes every tab through an accessible overview", async () => {
    const callbacks: ResizeObserverCallback[] = [];
    const OriginalResizeObserver = globalThis.ResizeObserver;
    globalThis.ResizeObserver = class {
      constructor(callback: ResizeObserverCallback) { callbacks.push(callback); }
      observe() {}
      unobserve() {}
      disconnect() {}
    };
    resetPaneLayoutToSingle(pageSnapshot([
      "First readable page title",
      "Second readable page title",
      "Third readable page title",
      "Fourth readable page title",
      "Fifth readable page title",
      "Sixth readable page title",
    ]));
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <TabBar router={paneRouter("main")} />, root);
    try {
      const strip = root.querySelector<HTMLElement>(".tab-strip-scroll");
      expect(strip).not.toBeNull();
      Object.defineProperties(strip!, {
        clientWidth: { configurable: true, value: 240 },
        scrollWidth: { configurable: true, value: 900 },
      });
      root.querySelectorAll<HTMLElement>(".tab").forEach((element, index) => {
        Object.defineProperties(element, {
          offsetLeft: { configurable: true, value: index * 150 },
          offsetWidth: { configurable: true, value: 145 },
        });
      });
      callbacks.forEach((callback) => callback([], {} as ResizeObserver));
      await Promise.resolve();

      const trigger = root.querySelector<HTMLButtonElement>("[data-tab-overview-trigger]");
      expect(trigger).not.toBeNull();
      expect(trigger!.getAttribute("aria-expanded")).toBe("false");
      trigger!.click();
      const rows = document.querySelectorAll<HTMLElement>("[data-tab-overview-row]");
      expect(rows).toHaveLength(6);
      expect(rows[5].textContent).toContain("Sixth readable page title");
      expect(rows[0].getAttribute("aria-selected")).toBe("true");

      rows[5].click();
      expect(tabRoute(paneRouter("main").activeTab())).toMatchObject({ name: "Sixth readable page title" });
      expect(document.querySelector("[role=listbox]")).toBeNull();
      await Promise.resolve();
      expect(strip!.scrollLeft).toBeGreaterThan(0);

      trigger!.click();
      const list = document.querySelector<HTMLElement>("[role=listbox]")!;
      list.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true, cancelable: true }));
      expect(document.activeElement).toBe(document.querySelectorAll("[data-tab-overview-row]")[5]);
      list.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(paneRouter("main").tabs()).toHaveLength(5));
    } finally {
      dispose();
      globalThis.ResizeObserver = OriginalResizeObserver;
    }
  });

  it("shows overflow controls only in the pane whose strip actually overflows", async () => {
    const callbacks: ResizeObserverCallback[] = [];
    const OriginalResizeObserver = globalThis.ResizeObserver;
    globalThis.ResizeObserver = class {
      constructor(callback: ResizeObserverCallback) { callbacks.push(callback); }
      observe() {}
      unobserve() {}
      disconnect() {}
    };
    const { root, dispose } = renderSplit(["A", "B", "C", "D", "E"], ["X", "Y"]);
    try {
      const strips = root.querySelectorAll<HTMLElement>(".tab-strip-scroll");
      expect(strips).toHaveLength(2);
      Object.defineProperties(strips[0], {
        clientWidth: { configurable: true, value: 220 },
        scrollWidth: { configurable: true, value: 700 },
      });
      Object.defineProperties(strips[1], {
        clientWidth: { configurable: true, value: 320 },
        scrollWidth: { configurable: true, value: 250 },
      });
      callbacks.forEach((callback) => callback([], {} as ResizeObserver));
      await Promise.resolve();

      expect(root.querySelectorAll('[data-pane-id="main"] [data-tab-overview-trigger]')).toHaveLength(1);
      expect(root.querySelectorAll('[data-pane-id="pane-2"] [data-tab-overview-trigger]')).toHaveLength(0);
    } finally {
      dispose();
      globalThis.ResizeObserver = OriginalResizeObserver;
    }
  });
});
