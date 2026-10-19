import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { paneRouter, resetPaneLayoutToSingle } from "../panes";
import { resetStore } from "../document";
import type { PaneSnapshot } from "../router";
import { SidebarTabs } from "./SidebarTabs";
import { TabBar } from "./TabBar";
import { contextMenu, setContextMenu } from "../ui";

// GH #704 v1 / D-20261018-W1: the open tabs as a sidebar list at every width
// (list, active highlight, switch, close). The component asks nothing about the
// device, so these cases hold on a phone exactly as on a desktop.

const snap = (names: string[], activeIndex = 0): PaneSnapshot => ({
  tabs: names.map((name) => ({ history: [{ kind: "page" as const, name, pageKind: "page" as const }], pos: 0, pinned: false })),
  activeIndex,
});

function mount(names: string[], activeIndex = 0) {
  resetPaneLayoutToSingle(snap(names, activeIndex));
  const host = document.body.appendChild(document.createElement("div"));
  const dispose = render(() => <SidebarTabs />, host);
  return { host, dispose };
}

const rows = (host: HTMLElement) => [...host.querySelectorAll<HTMLElement>(".nav-tab")];

afterEach(() => {
  resetStore();
  resetPaneLayoutToSingle(snap(["Home"]));
  document.body.replaceChildren();
});

describe("SidebarTabs (GH #704 v1)", () => {
  it("renders nothing for a single tab", () => {
    const { host, dispose } = mount(["Only"]);
    try {
      expect(host.querySelector('[data-sidebar-section="tabs"]')).toBeNull();
    } finally { dispose(); }
  });

  it("lists every tab with its title and highlights the active one", () => {
    const { host, dispose } = mount(["Alpha", "Beta", "Gamma"], 1);
    try {
      expect(rows(host).map((r) => r.querySelector(".nav-tab-title")?.textContent)).toEqual(["Alpha", "Beta", "Gamma"]);
      expect(rows(host).map((r) => r.classList.contains("active"))).toEqual([false, true, false]);
      expect(host.querySelector(".nav-section-count")?.textContent).toBe("3");
    } finally { dispose(); }
  });

  it("clicking a row switches the active tab and highlights it", () => {
    const { host, dispose } = mount(["Alpha", "Beta"], 0);
    try {
      rows(host)[1]!.querySelector<HTMLElement>(".nav-tab-main")!.click();
      const router = paneRouter("main");
      expect(router.activeId()).toBe(router.tabs()[1]!.id);
      expect(rows(host).map((r) => r.classList.contains("active"))).toEqual([false, true]);
    } finally { dispose(); }
  });

  it("the close button removes a tab and the section hides at one tab", async () => {
    const { host, dispose } = mount(["Alpha", "Beta"], 0);
    try {
      rows(host)[1]!.querySelector<HTMLElement>(".nav-tab-close")!.click();
      await Promise.resolve();
      expect(paneRouter("main").tabs()).toHaveLength(1);
      expect(host.querySelector('[data-sidebar-section="tabs"]')).toBeNull();
    } finally { dispose(); }
  });

  it("middle-click closes a tab; the header toggle collapses the list", async () => {
    const { host, dispose } = mount(["Alpha", "Beta", "Gamma"], 0);
    try {
      rows(host)[2]!.dispatchEvent(new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }));
      await Promise.resolve();
      expect(paneRouter("main").tabs()).toHaveLength(2);
      host.querySelector<HTMLElement>("[data-sidebar-section-toggle]")!.click();
      expect(rows(host)).toHaveLength(0);
      host.querySelector<HTMLElement>("[data-sidebar-section-toggle]")!.click();
      expect(rows(host)).toHaveLength(2);
    } finally { dispose(); }
  });
});

// GH #704 v2 (D-20261019-VT1): drag to reorder and to nest/unnest (the
// Favorites depth rule, through beginRowReorderDrag), collapse hides sidebar
// rows only, "Move out of group" on a nested row's context menu.
function pointer(type: string, x: number, y: number, pointerType = "mouse"): Event {
  const Ctor = (window as { PointerEvent?: typeof PointerEvent }).PointerEvent ?? MouseEvent;
  const event = new Ctor(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 0, buttons: 1 });
  Object.defineProperty(event, "pointerType", { value: pointerType });
  return event;
}

const ROW_H = 30;
/** Lay the rows out in a column (jsdom has no layout) and route hit tests to them. */
function layOut(host: HTMLElement) {
  rows(host).forEach((row, i) => {
    Object.defineProperty(row, "getBoundingClientRect", {
      configurable: true,
      value: () => ({ x: 0, y: i * ROW_H, left: 0, top: i * ROW_H, width: 200, height: ROW_H, right: 200, bottom: (i + 1) * ROW_H, toJSON: () => ({}) }),
    });
  });
  document.elementFromPoint = ((_x: number, y: number) => rows(host)[Math.floor(y / ROW_H)] ?? null) as typeof document.elementFromPoint;
}

/** Drag the row titled `from` to the point (dx right of the grab, y). */
async function drag(host: HTMLElement, from: string, dx: number, y: number) {
  layOut(host);
  const index = titles(host).indexOf(from);
  const startY = index * ROW_H + ROW_H / 2;
  rows(host)[index]!.querySelector(".nav-tab-main")!.dispatchEvent(pointer("pointerdown", 50, startY));
  document.dispatchEvent(pointer("pointermove", 50 + dx / 2, (startY + y) / 2));
  document.dispatchEvent(pointer("pointermove", 50 + dx, y));
  document.dispatchEvent(pointer("pointerup", 50 + dx, y));
  await new Promise((r) => setTimeout(r, 0)); // the drop's click suppression ends
}

const titles = (host: HTMLElement) => rows(host).map((r) => r.querySelector(".nav-tab-title")?.textContent ?? "");
const depths = (host: HTMLElement) => rows(host).map((r) => Number(r.dataset.tabDepth));
const stripTitles = (host: HTMLElement) => [...host.querySelectorAll(".tab[data-tab-id] .tab-title")].map((n) => n.textContent);

function mountWithStrip(names: string[]) {
  resetPaneLayoutToSingle(snap(names));
  const host = document.body.appendChild(document.createElement("div"));
  const sidebar = host.appendChild(document.createElement("div"));
  const strip = host.appendChild(document.createElement("div"));
  const d1 = render(() => <SidebarTabs />, sidebar);
  const d2 = render(() => <TabBar router={paneRouter("main")} />, strip);
  return { host: sidebar, strip, dispose: () => { d1(); d2(); } };
}

describe("SidebarTabs tree (GH #704 v2)", () => {
  const realElementFromPoint = document.elementFromPoint;
  afterEach(() => {
    document.elementFromPoint = realElementFromPoint;
    setContextMenu(null);
  });

  it("dragging a row reorders the tabs, and the top strip follows", async () => {
    const { host, strip, dispose } = mountWithStrip(["Alpha", "Beta", "Gamma"]);
    try {
      await drag(host, "Gamma", 0, 2); // upper half of Alpha's row
      expect(titles(host)).toEqual(["Gamma", "Alpha", "Beta"]);
      expect(stripTitles(strip)).toEqual(["Gamma", "Alpha", "Beta"]);
      expect(depths(host)).toEqual([0, 0, 0]);
      // A drag is not a click: the active tab did not change.
      expect(paneRouter("main").activeId()).toBe(paneRouter("main").tabs()[1]!.id);
    } finally { dispose(); }
  });

  it("dragging right of the grab nests under the row above; dragging left moves it out", async () => {
    const { host, strip, dispose } = mountWithStrip(["Alpha", "Beta", "Gamma"]);
    try {
      expect(host.querySelector(".nav-tab-toggle")).toBeNull(); // flat: no disclosure column
      await drag(host, "Beta", 20, ROW_H + ROW_H / 2 + 2); // its own slot, one step right
      expect(titles(host)).toEqual(["Alpha", "Beta", "Gamma"]);
      expect(depths(host)).toEqual([0, 1, 0]);
      expect(rows(host)[0]!.querySelector(".nav-tab-toggle")?.getAttribute("aria-expanded")).toBe("true");
      await drag(host, "Gamma", 40, 2 * ROW_H + 2);
      expect(depths(host)).toEqual([0, 1, 2]);
      expect(stripTitles(strip)).toEqual(["Alpha", "Beta", "Gamma"]); // the strip stays flat, in tree order
      await drag(host, "Beta", -20, ROW_H + ROW_H / 2 + 2);
      expect(depths(host)).toEqual([0, 0, 1]); // Beta moved out, with its child
      expect(paneRouter("main").tabs().map((t) => t.parent)).toEqual([null, null, paneRouter("main").tabs()[1]!.id]);
    } finally { dispose(); }
  });

  it("a drag left past the rows' edge still drops on the row at that height", async () => {
    const { host, dispose } = mountWithStrip(["Alpha", "Beta", "Gamma"]);
    try {
      await drag(host, "Beta", 20, ROW_H + ROW_H / 2 + 2);
      expect(depths(host)).toEqual([0, 1, 0]);
      layOut(host);
      // The hit test misses left of the rows (x < 0 here), as it does in the
      // real sidebar when an un-nest drag overshoots the row's left edge.
      const hit = document.elementFromPoint;
      document.elementFromPoint = ((x: number, y: number) => (x < 0 ? null : hit(x, y))) as typeof document.elementFromPoint;
      const startY = ROW_H + ROW_H / 2;
      rows(host)[1]!.querySelector(".nav-tab-main")!.dispatchEvent(pointer("pointerdown", 10, startY));
      document.dispatchEvent(pointer("pointermove", 0, startY + 1));
      document.dispatchEvent(pointer("pointermove", -14, startY + 2));
      document.dispatchEvent(pointer("pointerup", -14, startY + 2));
      await new Promise((r) => setTimeout(r, 0));
      expect(depths(host)).toEqual([0, 0, 0]);
    } finally { dispose(); }
  });

  it("collapsing a parent hides its children in the sidebar only", async () => {
    const { host, strip, dispose } = mountWithStrip(["Alpha", "Beta", "Gamma"]);
    try {
      await drag(host, "Beta", 20, ROW_H + ROW_H / 2 + 2);
      rows(host)[0]!.querySelector<HTMLElement>(".nav-tab-toggle")!.click();
      expect(titles(host)).toEqual(["Alpha", "Gamma"]);
      expect(rows(host)[0]!.querySelector(".nav-tab-toggle")?.getAttribute("aria-expanded")).toBe("false");
      expect(stripTitles(strip)).toEqual(["Alpha", "Beta", "Gamma"]);
      expect(host.querySelector(".nav-section-count")?.textContent).toBe("3");
      rows(host)[0]!.querySelector<HTMLElement>(".nav-tab-toggle")!.click();
      expect(titles(host)).toEqual(["Alpha", "Beta", "Gamma"]);
    } finally { dispose(); }
  });

  it("a nested row's context menu offers Move out of group; a root row's does not", async () => {
    const { host, dispose } = mountWithStrip(["Alpha", "Beta", "Gamma"]);
    try {
      await drag(host, "Beta", 20, ROW_H + ROW_H / 2 + 2);
      const root = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 5, clientY: 5 });
      rows(host)[2]!.querySelector(".nav-tab-main")!.dispatchEvent(root);
      expect(root.defaultPrevented).toBe(false);
      expect(contextMenu()).toBeFalsy();
      rows(host)[1]!.querySelector(".nav-tab-main")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 5, clientY: 40 }));
      const menu = contextMenu();
      expect(menu?.kind).toBe("action-menu");
      const items = menu?.kind === "action-menu" ? menu.items : [];
      expect(items.map((item) => item.label)).toEqual(["Move out of group"]);
      items[0]!.run!();
      expect(depths(host)).toEqual([0, 0, 0]);
      expect(titles(host)).toEqual(["Alpha", "Beta", "Gamma"]);
    } finally { dispose(); }
  });

  it("touch: a finger that moves at once scrolls; a still hold then drag reorders", async () => {
    vi.useFakeTimers();
    const { host, dispose } = mountWithStrip(["Alpha", "Beta", "Gamma"]);
    try {
      layOut(host);
      const main = (i: number) => rows(host)[i]!.querySelector(".nav-tab-main")!;
      main(2).dispatchEvent(pointer("pointerdown", 50, 75, "touch"));
      document.dispatchEvent(pointer("pointermove", 50, 40, "touch"));
      document.dispatchEvent(pointer("pointermove", 50, 2, "touch"));
      document.dispatchEvent(pointer("pointerup", 50, 2, "touch"));
      vi.advanceTimersByTime(1000);
      expect(titles(host)).toEqual(["Alpha", "Beta", "Gamma"]);
      main(2).dispatchEvent(pointer("pointerdown", 50, 75, "touch"));
      vi.advanceTimersByTime(600);
      document.dispatchEvent(pointer("pointermove", 50, 40, "touch"));
      document.dispatchEvent(pointer("pointermove", 50, 2, "touch"));
      document.dispatchEvent(pointer("pointerup", 50, 2, "touch"));
      vi.advanceTimersByTime(10);
      expect(titles(host)).toEqual(["Gamma", "Alpha", "Beta"]);
    } finally { dispose(); vi.useRealTimers(); }
  });
});
