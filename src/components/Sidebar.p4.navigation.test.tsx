import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { installMobileDrawerMode } from "../mobileDrawers";
import { openPage, resetTabsToJournals, route } from "../router";
import type { LoadGraphPathOutcome } from "../graph";
import type { PageEntry } from "../types";
import {
  activeDrawer,
  bumpGraphEpoch,
  closeContextMenu,
  closeSwitcher,
  completeActiveLeftNavigation,
  resetLeftSidebarSections,
  setAliasMap,
  setFavorites,
  setLeftSidebarOpen,
  setRecentPages,
  setRightSidebar,
  setRightSidebarOpen,
  sidebarOpen,
} from "../ui";
import { Sidebar, type GraphNavigationActions } from "./Sidebar";

type MutableMedia = MediaQueryList & { matches: boolean; emit(): void };

function mobileMedia(): MutableMedia {
  const listeners = new Set<(event: MediaQueryListEvent) => void>();
  const media = {
    matches: true,
    media: "(max-width: 639px)",
    onchange: null,
    addEventListener: (_type: string, listener: EventListenerOrEventListenerObject) => {
      if (typeof listener === "function") listeners.add(listener as (event: MediaQueryListEvent) => void);
    },
    removeEventListener: (_type: string, listener: EventListenerOrEventListenerObject) => {
      if (typeof listener === "function") listeners.delete(listener as (event: MediaQueryListEvent) => void);
    },
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => true,
    emit() {
      const event = { matches: media.matches, media: media.media } as MediaQueryListEvent;
      listeners.forEach((listener) => listener(event));
    },
  } as MutableMedia;
  return media;
}

function page(name: string, path = `pages/${name.replaceAll("/", "___")}.md`): PageEntry {
  return { name, path, kind: "page", date_key: null };
}

function findText(root: ParentNode, selector: string, text: string): HTMLElement {
  const found = [...root.querySelectorAll<HTMLElement>(selector)]
    .find((element) => element.textContent?.trim() === text || element.textContent?.includes(text));
  if (!found) throw new Error(`Missing ${selector} containing ${JSON.stringify(text)}`);
  return found;
}

function dispatch(element: Element, type = "click", init: MouseEventInit = {}) {
  element.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, ...init }));
}

afterEach(() => {
  closeContextMenu();
  closeSwitcher();
  setAliasMap({});
  setFavorites([]);
  setRecentPages([]);
  setRightSidebar([]);
  setRightSidebarOpen(false);
  setLeftSidebarOpen(false);
  resetLeftSidebarSections();
  resetTabsToJournals();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

describe("GH #161 successful left-navigation boundary", () => {
  it("closes and focuses only after a mounted active in-window destination succeeds", async () => {
    const media = mobileMedia();
    vi.stubGlobal("matchMedia", vi.fn(() => media));
    const uninstallMode = installMobileDrawerMode();

    const pages = [
      page("A path target", "pages/nested/a-path-target.md"),
      page("Namespace"),
      page("Namespace/Child"),
      ...Array.from({ length: 303 }, (_, index) => page(`Filler ${String(index).padStart(3, "0")}`)),
    ];
    vi.spyOn(backend(), "listPages").mockResolvedValue(pages);
    vi.spyOn(backend(), "listKnownGraphs").mockResolvedValue([
      { name: "Known graph", path: "/graphs/known" },
    ]);
    bumpGraphEpoch();

    let knownOutcome: LoadGraphPathOutcome = { kind: "aborted" };
    let pickedOutcome: LoadGraphPathOutcome = { kind: "aborted" };
    let createdOutcome: LoadGraphPathOutcome = { kind: "aborted" };
    let knownFailure: unknown;
    let pickedFailure: unknown;
    let createdFailure: unknown;
    const actions: GraphNavigationActions = {
      openKnown: vi.fn(async () => {
        if (knownFailure) throw knownFailure;
        return knownOutcome;
      }),
      openPicked: vi.fn(async () => {
        if (pickedFailure) throw pickedFailure;
        return pickedOutcome;
      }),
      createNew: vi.fn(async () => {
        if (createdFailure) throw createdFailure;
        return createdOutcome;
      }),
    };

    setFavorites([{ name: "Favorite same page", kind: "page" }]);
    setRecentPages([{ name: "Recent destination", kind: "page" }]);
    const completion = vi.fn(completeActiveLeftNavigation);
    const root = document.createElement("div");
    const main = document.createElement("main");
    main.className = "main-content";
    main.tabIndex = -1;
    document.body.append(root, main);
    const dispose = render(() => (
      <Sidebar onActiveNavigationComplete={completion} graphActions={actions} />
    ), root);

    const armLeft = () => {
      setRightSidebarOpen(false);
      setLeftSidebarOpen(true);
      expect(activeDrawer()).toBe("left");
    };
    const expectCompleted = async (before: number) => {
      await vi.waitFor(() => expect(completion).toHaveBeenCalledTimes(before + 1));
      expect(sidebarOpen()).toBe(false);
      expect(document.activeElement).toBe(main);
    };
    const expectNotCompleted = async (before: number) => {
      await Promise.resolve();
      await Promise.resolve();
      expect(completion).toHaveBeenCalledTimes(before);
    };
    const graphMenu = async () => {
      dispatch(root.querySelector(".graph-switch-btn")!);
      return vi.waitFor(() => {
        const menu = root.querySelector<HTMLElement>(".graph-switch-menu");
        expect(menu).not.toBeNull();
        return menu!;
      });
    };

    try {
      // Positive synchronous destinations, including same-page navigation and a
      // path-backed All Pages row, all close only through the shared callback.
      openPage("Before journals", "page");
      armLeft();
      let before = completion.mock.calls.length;
      dispatch(findText(root, ".nav-item", "Journals"));
      await expectCompleted(before);
      expect(route().kind).toBe("journals");

      openPage("Favorite same page", "page");
      armLeft();
      before = completion.mock.calls.length;
      dispatch(findText(root, "#sidebar-favorites-list .nav-page", "Favorite same page"));
      await expectCompleted(before);
      expect(route()).toMatchObject({ kind: "page", name: "Favorite same page" });

      armLeft();
      before = completion.mock.calls.length;
      dispatch(findText(root, "#sidebar-recent-list .nav-page", "Recent destination"));
      await expectCompleted(before);
      expect(route()).toMatchObject({ kind: "page", name: "Recent destination" });

      armLeft();
      before = completion.mock.calls.length;
      dispatch(findText(root, ".nav-section-header", "ALL PAGES"));
      await expectNotCompleted(before);
      expect(sidebarOpen()).toBe(true);
      const pathRow = await vi.waitFor(() => findText(root, ".nav-page", "A path target"));
      dispatch(pathRow);
      await expectCompleted(before);
      expect(route()).toMatchObject({
        kind: "page",
        name: "A path target",
        path: "pages/nested/a-path-target.md",
      });

      armLeft();
      before = completion.mock.calls.length;
      dispatch(findText(root, ".nav-section-header", "NAMESPACES"));
      await expectNotCompleted(before);
      expect(sidebarOpen()).toBe(true);
      const namespace = await vi.waitFor(() => findText(root, ".ns-node-label", "Namespace"));
      dispatch(namespace);
      await expectCompleted(before);
      expect(route()).toMatchObject({ kind: "page", name: "Namespace" });

      // Disclosures and search are not navigation completion. Namespace-node
      // disclosure is exercised separately from its adjacent page label.
      armLeft();
      before = completion.mock.calls.length;
      dispatch(root.querySelector('[data-sidebar-section="favorites"]')!);
      dispatch(root.querySelector('[data-sidebar-section="recent"]')!);
      dispatch(findText(root, ".nav-section-header", "ALL PAGES"));
      dispatch(findText(root, ".nav-section-header", "NAMESPACES"));
      await expectNotCompleted(before);
      expect(sidebarOpen()).toBe(true);

      // Re-open both lists; the oversized resource exposes the +more/search row.
      dispatch(findText(root, ".nav-section-header", "ALL PAGES"));
      dispatch(findText(root, ".nav-section-header", "NAMESPACES"));
      const namespaceToggle = await vi.waitFor(() => root.querySelector<HTMLElement>(".ns-node-toggle")!);
      dispatch(namespaceToggle);
      const more = await vi.waitFor(() => root.querySelector<HTMLElement>(".nav-page-more")!);
      dispatch(more);
      await expectNotCompleted(before);
      expect(sidebarOpen()).toBe(true);
      closeSwitcher();

      // Modified rows preserve the semantic surface they opened: Shift replaces
      // the left drawer with the right drawer; middle/new-tab and context menu
      // leave the left drawer active. None reports active left navigation.
      resetLeftSidebarSections();
      armLeft();
      before = completion.mock.calls.length;
      dispatch(findText(root, "#sidebar-favorites-list .nav-page", "Favorite same page"), "click", { shiftKey: true });
      await expectNotCompleted(before);
      expect(activeDrawer()).toBe("right");

      armLeft();
      before = completion.mock.calls.length;
      dispatch(findText(root, "#sidebar-favorites-list .nav-page", "Favorite same page"), "auxclick", { button: 1 });
      await expectNotCompleted(before);
      expect(activeDrawer()).toBe("left");

      dispatch(findText(root, "#sidebar-favorites-list .nav-page", "Favorite same page"), "contextmenu", { clientX: 10, clientY: 20 });
      await expectNotCompleted(before);
      expect(activeDrawer()).toBe("left");
      closeContextMenu();

      // Known graph: loaded/already-current are the only in-place outcomes that
      // complete. Shift/peer, focused-existing, aborted, and rejection retain the
      // drawer even though the graph menu itself closes immediately.
      knownOutcome = { kind: "loaded", root: "/graphs/known" };
      before = completion.mock.calls.length;
      let menu = await graphMenu();
      dispatch(findText(menu, ".graph-switch-row", "Known graph"));
      await expectCompleted(before);
      expect(root.querySelector(".graph-switch-menu")).toBeNull();

      armLeft();
      knownOutcome = { kind: "loaded", root: "/graphs/known" };
      before = completion.mock.calls.length;
      menu = await graphMenu();
      dispatch(findText(menu, ".graph-switch-row", "Known graph"), "click", { shiftKey: true });
      await expectNotCompleted(before);
      expect(activeDrawer()).toBe("left");

      for (const outcome of [
        { kind: "aborted" } as const,
        { kind: "focused_existing" } as const,
      ]) {
        knownOutcome = outcome;
        before = completion.mock.calls.length;
        menu = await graphMenu();
        dispatch(findText(menu, ".graph-switch-row", "Known graph"));
        await expectNotCompleted(before);
        expect(activeDrawer()).toBe("left");
      }

      knownFailure = new Error("load failed");
      before = completion.mock.calls.length;
      menu = await graphMenu();
      dispatch(findText(menu, ".graph-switch-row", "Known graph"));
      await expectNotCompleted(before);
      expect(activeDrawer()).toBe("left");
      knownFailure = undefined;

      // Picker cancel/failed load are negative. already_current and a completed
      // new-graph load are positive awaited outcomes.
      pickedOutcome = { kind: "aborted" };
      before = completion.mock.calls.length;
      menu = await graphMenu();
      dispatch(findText(menu, ".ctx-item", "Open graph…"));
      await expectNotCompleted(before);
      expect(activeDrawer()).toBe("left");

      pickedFailure = new Error("picker load failed");
      before = completion.mock.calls.length;
      menu = await graphMenu();
      dispatch(findText(menu, ".ctx-item", "Open graph…"));
      await expectNotCompleted(before);
      expect(activeDrawer()).toBe("left");
      pickedFailure = undefined;

      pickedOutcome = { kind: "already_current", root: "/graphs/known" };
      before = completion.mock.calls.length;
      menu = await graphMenu();
      dispatch(findText(menu, ".ctx-item", "Open graph…"));
      await expectCompleted(before);

      armLeft();
      createdOutcome = { kind: "loaded", root: "/graphs/new" };
      before = completion.mock.calls.length;
      menu = await graphMenu();
      dispatch(findText(menu, ".ctx-item", "New graph…"));
      await expectCompleted(before);

      armLeft();
      createdOutcome = { kind: "aborted" };
      before = completion.mock.calls.length;
      menu = await graphMenu();
      dispatch(findText(menu, ".ctx-item", "New graph…"));
      await expectNotCompleted(before);
      expect(activeDrawer()).toBe("left");

      createdFailure = new Error("create failed");
      before = completion.mock.calls.length;
      menu = await graphMenu();
      dispatch(findText(menu, ".ctx-item", "New graph…"));
      await expectNotCompleted(before);
      expect(activeDrawer()).toBe("left");

      // The same completion callback is deliberately a no-op at the 640+
      // persistent-sidebar boundary: navigation still succeeds, but it neither
      // closes the sidebar nor steals focus from an ordinary desktop target.
      media.matches = false;
      media.emit();
      setLeftSidebarOpen(true);
      const desktopFocus = document.createElement("button");
      document.body.appendChild(desktopFocus);
      desktopFocus.focus();
      before = completion.mock.calls.length;
      dispatch(findText(root, "#sidebar-favorites-list .nav-page", "Favorite same page"));
      await vi.waitFor(() => expect(completion).toHaveBeenCalledTimes(before + 1));
      expect(route()).toMatchObject({ kind: "page", name: "Favorite same page" });
      expect(sidebarOpen()).toBe(true);
      expect(document.activeElement).toBe(desktopFocus);
      expect(completion.mock.results.at(-1)?.value).toBe(false);
    } finally {
      dispose();
      media.matches = false;
      media.emit();
      uninstallMode();
    }
  });
});
