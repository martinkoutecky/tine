// GH #468. A left-sidebar row runs the full width of the sidebar; the page name
// in it does not. The whole row is the link — clicking the blank space beside a
// short name opens that page, which is what a row-shaped hover highlight
// promises. v0.6.981 briefly gated navigation on the title alone for GH #464
// (Logseq's behaviour), which made a page called `test` a target a few
// characters wide; #468 reported it within days and Tine diverges from OG here
// on purpose.
//
// The drag is what makes this safe rather than a trade-off, so it is asserted in
// the same file: rowReorder's 4px threshold and post-drag click suppression mean
// a favourite is reordered by MOVING the pointer, not by finding a part of the
// row that is not a link. The `does not navigate` case here is the drop, not a
// region of the row.
//
// This is a render test because what is being asserted is which element answers
// a click and what a drag does to the order — both observable in jsdom. The
// cursor is presentation and is not asserted here; the geometry that decides
// which element is under a given pixel is in scripts/check-sidebar-row-hitbox.mjs.
import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { openJournals, resetTabsToJournals, route, tabs } from "../router";
import {
  closeContextMenu,
  contextMenu,
  favorites,
  resetLeftSidebarSections,
  seedFavorites,
  setFavorites,
  setRecentPages,
} from "../ui";
import { refreshPageIndex } from "../pageIndex";
import { Sidebar } from "./Sidebar";

function rect(left: number, top: number, width: number, height: number): DOMRect {
  return {
    x: left, y: top, width, height, left, top,
    right: left + width, bottom: top + height, toJSON: () => ({}),
  } as DOMRect;
}

function setRect(el: Element | null, left: number, top: number, width: number, height: number) {
  Object.defineProperty(el, "getBoundingClientRect", {
    configurable: true,
    value: () => rect(left, top, width, height),
  });
}

function pointer(type: string, x: number, y: number): PointerEvent {
  const Ctor = (window as { PointerEvent?: typeof PointerEvent }).PointerEvent ?? MouseEvent;
  return new Ctor(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 0, buttons: 1 }) as PointerEvent;
}

/** A real click on the row's blank remainder: the row is the event target,
 *  because there is no child element out there to hit. This is the gesture
 *  GH #468 reported doing nothing. */
function clickRow(row: HTMLElement) {
  row.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
}

/** The element inside the row that holds the page name. og rows carry no
 *  dedicated label span (master's `.nav-page-label` only exists to ellipsise a
 *  long name), so the title is the row's last child element, else the row. */
function titleOf(row: HTMLElement): HTMLElement {
  return (row.lastElementChild as HTMLElement | null) ?? row;
}

/** A real click on the page title. */
function clickTitle(row: HTMLElement) {
  titleOf(row).dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
}

afterEach(async () => {
  await new Promise((r) => setTimeout(r, 5));
  closeContextMenu();
  setFavorites([]);
  setRecentPages([]);
  resetLeftSidebarSections();
  resetTabsToJournals();
  openJournals();
  document.body.innerHTML = "";
  localStorage.clear();
  vi.restoreAllMocks();
});

function mount() {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <Sidebar />, root);
  return { root, dispose };
}

function mountFavorites() {
  seedFavorites(["Alpha", "Beta", "Gamma"]);
  setFavorites([
    { name: "Alpha", kind: "page" },
    { name: "Beta", kind: "page" },
    { name: "Gamma", kind: "page" },
  ]);
  const { root, dispose } = mount();
  const rows = () => [...root.querySelectorAll<HTMLElement>("#sidebar-favorites-list .nav-page")];
  return { root, dispose, rows };
}

describe("left-sidebar row hitbox (GH #468)", () => {
  it("navigates when the blank part of a favorite row is clicked, as it does from the title", () => {
    const { dispose, rows } = mountFavorites();
    try {
      clickRow(rows()[1]!);
      expect(route()).toMatchObject({ kind: "page", name: "Beta" });
      openJournals();
      clickTitle(rows()[1]!);
      expect(route()).toMatchObject({ kind: "page", name: "Beta" });
    } finally {
      dispose();
    }
  });

  it("navigates when the blank part of a Recent row is clicked, as it does from the title", () => {
    setRecentPages([{ name: "Recent destination", kind: "page" }]);
    const { root, dispose } = mount();
    try {
      const row = root.querySelector<HTMLElement>("#sidebar-recent-list .nav-page")!;
      clickRow(row);
      expect(route()).toMatchObject({ kind: "page", name: "Recent destination" });
      openJournals();
      clickTitle(row);
      expect(route()).toMatchObject({ kind: "page", name: "Recent destination" });
    } finally {
      dispose();
    }
  });

  it("navigates when the blank part of an All pages row is clicked, as it does from the title", async () => {
    vi.spyOn(backend(), "pageInventory").mockResolvedValue({
      rev: "1",
      entries: [{
        key: "listed page", name: "Listed page", is_journal: false, day: null,
        target: { kind: "existing", id: "pages/listed-page.md", others: [] },
      }],
    });
    await refreshPageIndex();
    const { root, dispose } = mount();
    try {
      const header = [...root.querySelectorAll<HTMLElement>(".nav-section-header")]
        .find((el) => el.textContent?.includes("ALL PAGES"))!;
      header.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
      const row = await vi.waitFor(() => {
        const found = [...root.querySelectorAll<HTMLElement>(".nav-page")]
          .find((el) => el.textContent?.includes("Listed page"));
        expect(found).toBeDefined();
        return found!;
      });
      clickRow(row);
      expect(route()).toMatchObject({ kind: "page", name: "Listed page" });
      openJournals();
      clickTitle(row);
      expect(route()).toMatchObject({ kind: "page", name: "Listed page" });
    } finally {
      dispose();
    }
  });

  it("middle-clicking the blank part of a row opens a background tab, as the title does", () => {
    const { dispose, rows } = mountFavorites();
    try {
      const beta = rows()[1]!;
      const before = tabs().length;
      beta.dispatchEvent(new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }));
      expect(tabs()).toHaveLength(before + 1);
      titleOf(beta)
        .dispatchEvent(new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }));
      expect(tabs()).toHaveLength(before + 2);
    } finally {
      dispose();
    }
  });

  it("right-clicking anywhere on the row still opens that page's menu", () => {
    const { dispose, rows } = mountFavorites();
    try {
      rows()[1]!.dispatchEvent(
        new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 10, clientY: 20 }),
      );
      expect(contextMenu()).toMatchObject({ name: "Beta" });
    } finally {
      dispose();
    }
  });

  it("dragging a favorite by its blank space reorders it and does not open it", () => {
    vi.spyOn(backend(), "setFavorites").mockResolvedValue();
    const { dispose, rows } = mountFavorites();
    try {
      const [first, second] = rows();
      setRect(first, 0, 0, 200, 30);
      setRect(second, 0, 30, 200, 30);

      // Grabbed at the far right of the row — the natural grab point, and the
      // one that is a link again.
      first!.dispatchEvent(pointer("pointerdown", 180, 10));
      const prev = document.elementFromPoint;
      try {
        document.elementFromPoint = () => second!;
        document.dispatchEvent(pointer("pointermove", 180, 55));
        document.dispatchEvent(pointer("pointerup", 180, 55));
      } finally {
        document.elementFromPoint = prev;
      }

      expect(favorites().map((f) => f.name)).toEqual(["Beta", "Alpha", "Gamma"]);
      // The click the browser synthesizes on the grabbed row after the drop must
      // not navigate. This is the assertion the full-row link depends on.
      clickRow(rows()[0]!);
      expect(route()).toMatchObject({ kind: "journals" });
    } finally {
      dispose();
    }
  });
});
