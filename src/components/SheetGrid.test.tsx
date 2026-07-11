import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { initParser } from "../render/parse";
import { blockProperty, resetStore, setDoc, type Node, type FeedPage } from "../store";
import { openJournals, route } from "../router";
import { resetCellSelectionForTests } from "../sheet/selection";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetCellSelectionForTests();
  resetStore();
  openJournals({ inPlace: true });
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function page(format: "md" | "org", roots: string[]): FeedPage {
  return {
    name: format === "org" ? "Org Sheet" : "Sheet",
    kind: "page",
    title: format === "org" ? "Org Sheet" : "Sheet",
    preBlock: null,
    roots,
    format,
    readOnly: false,
    guide: false,
  };
}

function node(
  id: string,
  raw: string,
  pageName: string,
  parent: string | null,
  children: string[] = []
): Node {
  return { id, raw, collapsed: false, parent, page: pageName, children };
}

function contextMenu(target: EventTarget): MouseEvent {
  const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 20, clientY: 30 });
  target.dispatchEvent(event);
  return event;
}

function pointerEnter(target: EventTarget): Event {
  const event = new Event("pointerenter", { bubbles: false, cancelable: true });
  target.dispatchEvent(event);
  return event;
}

function pointerLeave(target: EventTarget): Event {
  const event = new Event("pointerleave", { bubbles: false, cancelable: true });
  target.dispatchEvent(event);
  return event;
}

function animationFrame(): Promise<void> {
  return new Promise((resolve) => requestAnimationFrame(() => resolve()));
}

async function settledMeasure(): Promise<void> {
  for (let i = 0; i < 6; i++) await animationFrame();
}

function rectSnapshot(el: Element): Pick<DOMRect, "left" | "top" | "right" | "bottom" | "width" | "height"> {
  const rect = el.getBoundingClientRect();
  return {
    left: rect.left,
    top: rect.top,
    right: rect.right,
    bottom: rect.bottom,
    width: rect.width,
    height: rect.height,
  };
}

function mockSheetLayout(width: () => number) {
  const originalClientWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
  const originalScrollWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollWidth");
  const originalGetBoundingClientRect = HTMLElement.prototype.getBoundingClientRect;
  const originalResizeObserver = globalThis.ResizeObserver;
  const observers: ResizeObserverCallback[] = [];
  const rect = (left: number, top: number, width: number, height: number): DOMRect => ({
    x: left,
    y: top,
    left,
    top,
    right: left + width,
    bottom: top + height,
    width,
    height,
    toJSON: () => ({}),
  } as DOMRect);
  const hasDirectSheetContainer = (el: HTMLElement): boolean =>
    [...el.children].some((child) => child.classList.contains("block-sheet-container"));

  Object.defineProperty(HTMLElement.prototype, "clientWidth", {
    configurable: true,
    get() {
      if (this.classList.contains("ls-block") || this.classList.contains("block-sheet-container") || hasDirectSheetContainer(this)) return 200;
      return 0;
    },
  });
  Object.defineProperty(HTMLElement.prototype, "scrollWidth", {
    configurable: true,
    get() {
      if (this.classList.contains("block-sheet-container") || this.classList.contains("sheet-grid")) return width();
      return 0;
    },
  });
  HTMLElement.prototype.getBoundingClientRect = function () {
    if (this.classList.contains("main-content")) return rect(0, 0, 1000, 600);
    if (hasDirectSheetContainer(this)) return rect(400, 30, 200, 80);
    if (this.classList.contains("block-sheet-container")) return rect(400, 30, Math.max(200, width()), 80);
    if (this.classList.contains("sheet-grid")) return rect(400, 30, width(), 80);
    return originalGetBoundingClientRect.call(this);
  };
  globalThis.ResizeObserver = class ResizeObserver {
    constructor(callback: ResizeObserverCallback) {
      observers.push(callback);
    }
    observe = vi.fn();
    unobserve = vi.fn();
    disconnect = vi.fn();
  };

  return {
    trigger() {
      for (const observer of observers) observer([], {} as ResizeObserver);
    },
    restore() {
      if (originalClientWidth) Object.defineProperty(HTMLElement.prototype, "clientWidth", originalClientWidth);
      else delete (HTMLElement.prototype as { clientWidth?: number }).clientWidth;
      if (originalScrollWidth) Object.defineProperty(HTMLElement.prototype, "scrollWidth", originalScrollWidth);
      else delete (HTMLElement.prototype as { scrollWidth?: number }).scrollWidth;
      HTMLElement.prototype.getBoundingClientRect = originalGetBoundingClientRect;
      globalThis.ResizeObserver = originalResizeObserver;
    },
  };
}

function loadMdSheetDoc() {
  const pageName = "Sheet";
  setDoc({
    byId: {
      grid: node(
        "grid",
        "Grid parent\ntine.view:: grid\ntine.header:: true\ntine.col-widths:: 0=120;2=88",
        pageName,
        null,
        ["r1", "r2", "r3"]
      ),
      r1: node("r1", "", pageName, "grid", ["h1", "h2", "h3"]),
      h1: node("h1", "Name", pageName, "r1"),
      h2: node("h2", "Status", pageName, "r1"),
      h3: node("h3", "Notes", pageName, "r1"),
      r2: node("r2", "", pageName, "grid", ["c1", "c2", "c3"]),
      c1: node("c1", "TODO Ship [[Page Ref]]", pageName, "r2"),
      c2: node("c2", "Nested grid\ntine.view:: grid", pageName, "r2", ["nr1", "nr2"]),
      c3: node("c3", "Tail", pageName, "r2"),
      nr1: node("nr1", "", pageName, "c2", ["n11", "n12"]),
      n11: node("n11", "inner A", pageName, "nr1"),
      n12: node("n12", "inner B", pageName, "nr1"),
      nr2: node("nr2", "", pageName, "c2", ["n21"]),
      n21: node("n21", "inner C", pageName, "nr2"),
      r3: node("r3", "", pageName, "grid", ["c4"]),
      c4: node("c4", "ragged", pageName, "r3"),
      plain: node("plain", "Plain parent", pageName, null, ["plain-child"]),
      "plain-child": node("plain-child", "plain child", pageName, "plain"),
    },
    pages: [page("md", ["grid", "plain"])],
    feed: [pageName],
    loaded: true,
  });
}

function loadOrgSheetDoc() {
  const pageName = "Org Sheet";
  setDoc({
    byId: {
      "org-grid": node(
        "org-grid",
        "Org grid\n:PROPERTIES:\n:tine.view: grid\n:tine.header: true\n:END:",
        pageName,
        null,
        ["or1"]
      ),
      or1: node("or1", "", pageName, "org-grid", ["oh1", "oh2"]),
      oh1: node("oh1", "Org A", pageName, "or1"),
      oh2: node("oh2", "Org B", pageName, "or1"),
    },
    pages: [page("org", ["org-grid"])],
    feed: [pageName],
    loaded: true,
  });
}

describe("SheetGrid", () => {
  it("caps and progressively discloses a very wide grid", () => {
    const pageName = "Sheet";
    const byId: Record<string, Node> = {};
    const rowIds = ["wide-row-0", "wide-row-1", "wide-row-2"];
    for (let row = 0; row < 3; row++) {
      const cellIds = Array.from({ length: 20_000 }, (_, col) => `wide-cell-${row}-${col}`);
      byId[rowIds[row]] = node(rowIds[row], "", pageName, "wide-grid", cellIds);
      for (const cellId of cellIds) byId[cellId] = node(cellId, cellId, pageName, rowIds[row]);
    }
    byId["wide-grid"] = node("wide-grid", "Wide\ntine.view:: grid", pageName, null, rowIds);
    setDoc({ byId, pages: [page("md", ["wide-grid"])], feed: [pageName], loaded: true });

    const { root, dispose } = mount(() => <Block id="wide-grid" />);
    const grid = root.querySelector(".sheet-grid") as HTMLElement;
    expect(grid.querySelectorAll(":scope > .sheet-cell")).toHaveLength(600);
    expect(grid.querySelector(".sheet-load-more-columns")?.textContent).toContain("1-200 of 20000");
    (grid.querySelector(".sheet-load-more-columns") as HTMLButtonElement).click();
    expect(grid.querySelectorAll(":scope > .sheet-cell")).toHaveLength(1_200);
    expect(grid.querySelectorAll(":scope > .sheet-cell").length).toBeLessThanOrEqual(2_000);
    dispose();
  });

  it("progressively discovers and discloses a large ragged grid without mounting its full rectangle", async () => {
    const pageName = "Sheet";
    const byId: Record<string, Node> = {};
    const rowIds: string[] = [];
    for (let row = 0; row < 500; row++) {
      const rowId = `large-row-${row}`;
      const cellIds = Array.from({ length: row === 499 ? 3 : 1 }, (_, col) => `large-cell-${row}-${col}`);
      rowIds.push(rowId);
      byId[rowId] = node(rowId, "", pageName, "large-grid", cellIds);
      for (const cellId of cellIds) byId[cellId] = node(cellId, cellId, pageName, rowId);
    }
    byId["large-grid"] = node("large-grid", "Large\ntine.view:: grid", pageName, null, rowIds);
    setDoc({ byId, pages: [page("md", ["large-grid"])], feed: [pageName], loaded: true });

    const { root, dispose } = mount(() => <Block id="large-grid" />);
    const grid = root.querySelector(".sheet-grid") as HTMLElement;
    expect(grid.querySelectorAll(":scope > .sheet-cell")).toHaveLength(200);
    await vi.waitFor(() => expect(grid.querySelectorAll(":scope > .sheet-cell")).toHaveLength(600));
    expect(grid.querySelector(".sheet-load-more")?.textContent).toContain("200 of 500");
    (grid.querySelector(".sheet-load-more") as HTMLButtonElement).click();
    expect(grid.querySelectorAll(":scope > .sheet-cell")).toHaveLength(1200);
    expect(grid.querySelector(".sheet-load-more")?.textContent).toContain("400 of 500");
    dispose();
  });

  it("renders a read-only positional grid with widths, holes, headers, hidden tine props, and nested grids", () => {
    loadMdSheetDoc();
    const { root, dispose } = mount(() => <Block id="grid" />);

    const grid = root.querySelector(".block-sheet-container > .sheet-scroll > .sheet-grid") as HTMLElement | null;
    expect(grid).not.toBeNull();
    expect(grid!.style.gridTemplateColumns).toBe("120px max-content 88px");
    expect(grid!.style.gridTemplateColumns.trim().split(/\s+/)).toHaveLength(3);
    expect(grid!.querySelectorAll(":scope > .sheet-hole")).toHaveLength(2);
    expect(grid!.querySelectorAll(":scope > .sheet-header-cell")).toHaveLength(3);
    expect(grid!.querySelector('.sheet-cell[data-col="0"]')?.classList.contains("sheet-sticky-left")).toBe(true);
    expect(grid!.querySelector('.sheet-cell[data-col="1"]')?.classList.contains("sheet-sticky-left")).toBe(false);
    expect(root.textContent).not.toContain("tine.view");
    expect(root.querySelector(".sheet-cell .sheet-grid")).not.toBeNull();

    dispose();
  });

  it("toggles and centers breakout when the natural sheet width exceeds the column", async () => {
    let naturalWidth = 640;
    const layout = mockSheetLayout(() => naturalWidth);
    loadMdSheetDoc();
    const { root, dispose } = mount(() => (
      <div class="main-content">
        <Block id="grid" />
      </div>
    ));
    try {
      await settledMeasure();
      const container = root.querySelector(".block-sheet-container") as HTMLElement | null;
      expect(container).not.toBeNull();
      expect(container!.classList.contains("sheet-breakout")).toBe(true);
      expect(container!.style.getPropertyValue("--sheet-breakout-width")).toBe("640px");
      expect(container!.style.getPropertyValue("--sheet-breakout-shift")).toBe("220px");
      expect(root.querySelector(".sheet-cell .block-sheet-container")).toBeNull();

      naturalWidth = 160;
      layout.trigger();
      await settledMeasure();

      expect(container!.classList.contains("sheet-breakout")).toBe(false);
    } finally {
      dispose();
      layout.restore();
    }
  });

  it("renders block highlight colors and writes them from the cell menu", () => {
    loadMdSheetDoc();
    const { root, dispose } = mount(() => (
      <>
        <Block id="grid" />
        <ContextMenu />
      </>
    ));
    const cell = root.querySelector('.sheet-cell[data-block-id="c1"]') as HTMLElement | null;
    expect(cell).not.toBeNull();

    contextMenu(cell!);
    (document.querySelector('.ctx-color[title="yellow"]') as HTMLButtonElement).click();

    expect(blockProperty("c1", "background-color")).toBe("yellow");
    expect(cell!.style.background).toContain("rgba");
    dispose();
  });

  it("cell menu switches children between outline/grid/table and zooms into the cell", () => {
    loadMdSheetDoc();
    const { root, dispose } = mount(() => (
      <>
        <Block id="grid" />
        <ContextMenu />
      </>
    ));
    const cell = root.querySelector('.sheet-cell[data-block-id="c2"]') as HTMLElement | null;
    expect(cell).not.toBeNull();

    contextMenu(cell!);
    ([...document.querySelectorAll(".ctx-item")].find((el) => el.textContent?.trim() === "Table") as HTMLElement).click();
    expect(blockProperty("c2", "tine.view")).toBe("table");

    contextMenu(cell!);
    ([...document.querySelectorAll(".ctx-item")].find((el) => el.textContent?.trim() === "Outline") as HTMLElement).click();
    expect(blockProperty("c2", "tine.view")).toBeNull();

    contextMenu(cell!);
    ([...document.querySelectorAll(".ctx-item")].find((el) => el.textContent?.includes("Zoom into cell")) as HTMLElement).click();
    expect(route()).toMatchObject({ kind: "page", name: "Sheet", pageKind: "page" });
    expect((route() as { block?: string }).block).toBeTruthy();

    dispose();
  });

  it("renders a nested field table inside a positional grid cell", () => {
    const pageName = "Sheet";
    setDoc({
      byId: {
        grid: node("grid", "Grid parent\ntine.view:: grid", pageName, null, ["r1"]),
        r1: node("r1", "", pageName, "grid", ["cell"]),
        cell: node("cell", "Nested table\ntine.view:: table", pageName, "r1", ["tr1"]),
        tr1: node("tr1", "TODO Nested row\nowner:: Martin", pageName, "cell"),
      },
      pages: [page("md", ["grid"])],
      feed: [pageName],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="grid" />);

    expect(root.querySelector(".sheet-cell .sheet-table")).not.toBeNull();
    expect(root.textContent).toContain("Martin");

    dispose();
  });

  it("leaves non-grid blocks on the existing vertical children renderer", () => {
    loadMdSheetDoc();
    const { root, dispose } = mount(() => <Block id="plain" />);

    expect(root.querySelector(".block-children-container")).not.toBeNull();
    expect(root.querySelector(".sheet-grid")).toBeNull();
    expect(root.textContent).toContain("plain child");

    dispose();
  });

  it("detects org property drawers through format-aware facets", () => {
    loadOrgSheetDoc();
    const { root, dispose } = mount(() => <Block id="org-grid" />);

    expect(root.querySelector(".sheet-grid")).not.toBeNull();
    expect(root.querySelectorAll(".sheet-header-cell")).toHaveLength(2);
    expect(root.textContent).not.toContain("tine.view");

    dispose();
  });

  it("renders a configured positional aggregate footer", () => {
    const pageName = "Sheet";
    setDoc({
      byId: {
        grid: node("grid", "Grid parent\ntine.view:: grid\ntine.col-aggregates:: 1=sum", pageName, null, ["r1", "r2"]),
        r1: node("r1", "", pageName, "grid", ["a1", "a2"]),
        a1: node("a1", "Alpha", pageName, "r1"),
        a2: node("a2", "2", pageName, "r1"),
        r2: node("r2", "", pageName, "grid", ["b1", "b2"]),
        b1: node("b1", "Beta", pageName, "r2"),
        b2: node("b2", "5", pageName, "r2"),
      },
      pages: [page("md", ["grid"])],
      feed: [pageName],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="grid" />);

    expect(root.textContent).toContain("7");
    expect(root.querySelector(".sheet-aggregate-corner-toggle")).toBeNull();

    dispose();
  });

  it("pins, unpins, and writes positional aggregates from the corner toggle without hover reflow", async () => {
    const pageName = "Sheet";
    setDoc({
      byId: {
        grid: node("grid", "Grid parent\ntine.view:: grid", pageName, null, ["r1"]),
        r1: node("r1", "", pageName, "grid", ["a1"]),
        a1: node("a1", "2", pageName, "r1"),
      },
      pages: [page("md", ["grid"])],
      feed: [pageName],
      loaded: true,
    });
    const layout = mockSheetLayout(() => 200);
    const { root, dispose } = mount(() => (
      <div class="main-content">
        <Block id="grid" />
        <ContextMenu />
      </div>
    ));
    try {
      await settledMeasure();
      const grid = root.querySelector(".sheet-grid") as HTMLElement | null;
      const container = root.querySelector(".block-sheet-container") as HTMLElement | null;
      const scroll = root.querySelector(".block-sheet-container > .sheet-scroll") as HTMLElement | null;
      expect(grid).not.toBeNull();
      expect(container).not.toBeNull();
      expect(scroll?.firstElementChild).toBe(grid);
      expect(root.querySelector(".sheet-footer-cell")).toBeNull();
      expect(root.querySelector(".sheet-aggregate-corner-toggle")).toBeNull();

      const beforeHover = rectSnapshot(container!);
      pointerEnter(container!);
      expect(root.querySelector(".sheet-footer-cell")).toBeNull();
      let toggle = root.querySelector(".sheet-aggregate-corner-toggle") as HTMLButtonElement | null;
      expect(toggle).not.toBeNull();
      expect(toggle!.parentElement).toBe(container);
      expect(scroll!.contains(toggle!)).toBe(false);
      expect(rectSnapshot(container!)).toEqual(beforeHover);

      toggle!.click();
      let footer = root.querySelector(".sheet-grid > .sheet-footer-cell") as HTMLElement | null;
      expect(footer).not.toBeNull();
      expect(root.querySelector(".sheet-footer-overlay")).toBeNull();
      expect(footer!.style.position).not.toBe("absolute");
      toggle = root.querySelector(".sheet-aggregate-corner-toggle") as HTMLButtonElement | null;
      expect(toggle?.getAttribute("aria-pressed")).toBe("true");

      pointerLeave(grid!);
      expect(root.querySelector(".sheet-grid > .sheet-footer-cell")).not.toBeNull();
      toggle = root.querySelector(".sheet-aggregate-corner-toggle") as HTMLButtonElement | null;
      toggle!.click();
      expect(root.querySelector(".sheet-footer-cell")).toBeNull();

      pointerEnter(container!);
      (root.querySelector(".sheet-aggregate-corner-toggle") as HTMLButtonElement).click();
      const add = root.querySelector(".sheet-grid > .sheet-footer-cell .sheet-aggregate-add") as HTMLButtonElement | null;
      expect(add).not.toBeNull();
      add!.click();
      // The picker is an in-DOM action menu (a native <select>'s popup is a
      // separate GTK window in WebKitGTK — it blurs the select and collapses).
      const items = [...document.querySelectorAll(".ctx-item")];
      expect(items.length).toBeGreaterThan(0);
      const sum = items.find((el) => el.textContent?.trim() === "Sum") as HTMLElement | undefined;
      expect(sum).toBeTruthy();

      pointerLeave(grid!); // hover loss must not kill the menu or the pick
      sum!.click();

      expect(blockProperty("grid", "tine.col-aggregates")).toBe("0=sum");
    } finally {
      dispose();
      layout.restore();
    }
  });

  it("opens a sheet block as a full page from the sheet context menu", () => {
    loadMdSheetDoc();
    const { root, dispose } = mount(() => (
      <>
        <Block id="grid" />
        <ContextMenu />
      </>
    ));
    const grid = root.querySelector(".sheet-grid") as HTMLElement | null;
    expect(grid).not.toBeNull();

    contextMenu(grid!);
    ([...document.querySelectorAll(".ctx-item")].find((el) => el.textContent?.trim() === "Open as full page") as HTMLElement).click();

    expect(route()).toMatchObject({ kind: "page", name: "Sheet", pageKind: "page" });
    expect((route() as { block?: string }).block).toBeTruthy();
    dispose();
  });
});
