import { beforeEach, describe, expect, it } from "vitest";
import { buildPersistedSession, parsePersistedSession, type PersistedSession } from "./session";
import { resetPaneLayoutToSingle, restorePaneLayout, type LayoutNode } from "./panes";
import type { PaneSnapshot } from "./router";
import {
  applySidebarSession,
  favoritesSectionExpanded,
  recentSectionExpanded,
  rightSidebar,
  setRightSidebar,
  setFavoritesSectionExpanded,
  setRecentSectionExpanded,
} from "./ui";

const journals = (): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
  activeIndex: 0,
  scrolls: [12],
});

const page = (name: string): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
  scrolls: [34],
});

beforeEach(() => {
  resetPaneLayoutToSingle(journals());
  applySidebarSession({});
});

describe("persisted split session", () => {
  it("round-trips a two-pane layout with pane tabs and scrolls", () => {
    const root: LayoutNode = {
      kind: "split" as const,
      dir: "row" as const,
      ratio: 0.4,
      children: [
        { kind: "pane" as const, paneId: "main" },
        { kind: "pane" as const, paneId: "pane-2" },
      ],
    };
    restorePaneLayout(root, new Map([["main", journals()], ["pane-2", page("Side")]]), "pane-2");

    const raw = JSON.stringify(buildPersistedSession());
    const parsed = parsePersistedSession(raw)!;

    expect(parsed.layout).toEqual(root);
    expect(parsed.focusedPaneId).toBe("pane-2");
    expect(parsed.snapshots.get("main")?.tabs[0].history[0]).toEqual({ kind: "journals" });
    expect(parsed.snapshots.get("pane-2")?.tabs[0].history[0]).toEqual({
      kind: "page",
      name: "Side",
      pageKind: "page",
    });
    expect(parsed.snapshots.get("pane-2")?.scrolls).toEqual([34]);
  });

  it("parses a legacy flat session as a single main pane", () => {
    const legacy: PersistedSession = {
      tabs: [{ history: [{ kind: "page", name: "Legacy", pageKind: "page" }], pos: 0, pinned: true }],
      activeIndex: 0,
      scrolls: [99],
    };

    const parsed = parsePersistedSession(JSON.stringify(legacy))!;

    expect(parsed.layout).toEqual({ kind: "pane", paneId: "main" });
    expect(parsed.snapshots.get("main")?.tabs[0]).toMatchObject({
      history: [{ kind: "page", name: "Legacy", pageKind: "page" }],
      pinned: true,
    });
    expect(parsed.snapshots.get("main")?.scrolls).toEqual([99]);
  });

  it("round-trips a bounded virtual query workspace without persisting results", () => {
    const raw = JSON.stringify({
      tabs: [{
        history: [{
          kind: "query",
          id: "query-1",
          sourceKind: "search",
          source: "alpha -draft",
          presentation: "search",
        }],
        pos: 0,
        pinned: true,
      }],
      activeIndex: 0,
    });

    const parsed = parsePersistedSession(raw)!;
    expect(parsed.snapshots.get("main")?.tabs[0]).toMatchObject({
      pinned: true,
      history: [{
        kind: "query",
        id: "query-1",
        sourceKind: "search",
        source: "alpha -draft",
        presentation: "search",
      }],
    });
    expect(JSON.stringify(parsed)).not.toContain("results");
  });

  it("round-trips independent empty query routes in split panes with the chosen focused owner", () => {
    const empty = (id: string, source: string, presentation: "search" | "table"): PaneSnapshot => ({
      tabs: [{ history: [{ kind: "query", id, sourceKind: "search", source, presentation }], pos: 0, pinned: false }],
      activeIndex: 0,
    });
    const root: LayoutNode = {
      kind: "split", dir: "row", ratio: 0.5,
      children: [{ kind: "pane", paneId: "main" }, { kind: "pane", paneId: "pane-2" }],
    };
    restorePaneLayout(root, new Map([["main", empty("query-empty", "", "search")], ["pane-2", empty("query-alpha", "alpha", "table")]]), "pane-2");
    const parsed = parsePersistedSession(JSON.stringify(buildPersistedSession()))!;
    expect(parsed.focusedPaneId).toBe("pane-2");
    expect(parsed.snapshots.get("main")?.tabs[0].history[0]).toEqual({ kind: "query", id: "query-empty", sourceKind: "search", source: "", presentation: "search" });
    expect(parsed.snapshots.get("pane-2")?.tabs[0].history[0]).toEqual({ kind: "query", id: "query-alpha", sourceKind: "search", source: "alpha", presentation: "table" });
    expect(JSON.stringify(parsed)).not.toContain("results");
  });

  it("round-trips graph-scoped Favorites and Recent disclosure state and defaults legacy sessions open", () => {
    setFavoritesSectionExpanded(false);
    setRecentSectionExpanded(true);
    const persisted = buildPersistedSession();
    expect(persisted.favoritesSectionExpanded).toBe(false);
    expect(persisted.recentSectionExpanded).toBe(true);

    const parsed = parsePersistedSession(JSON.stringify(persisted))!;
    setFavoritesSectionExpanded(true);
    setRecentSectionExpanded(false);
    applySidebarSession(parsed.sidebar);
    expect(favoritesSectionExpanded()).toBe(false);
    expect(recentSectionExpanded()).toBe(true);

    applySidebarSession({});
    expect(favoritesSectionExpanded()).toBe(true);
    expect(recentSectionExpanded()).toBe(true);
  });

  it("round-trips each right-sidebar item's graph-local disclosure state", () => {
    setRightSidebar([
      { kind: "page", name: "Expanded", pageKind: "page", collapsed: false },
      { kind: "block", uuid: "stable-block", page: "Source", pageKind: "page", collapsed: true },
    ]);
    const persisted = buildPersistedSession();
    expect(persisted.rightSidebarItems?.map((item) => item.collapsed)).toEqual([false, true]);

    const parsed = parsePersistedSession(JSON.stringify(persisted))!;
    setRightSidebar([]);
    applySidebarSession(parsed.sidebar);
    expect(rightSidebar()).toEqual(persisted.rightSidebarItems);

    applySidebarSession({ items: [{ kind: "page", name: "Legacy", pageKind: "page" }] });
    expect(rightSidebar()[0].collapsed).toBeUndefined();
  });

  it("rewrites duplicate restored journals panes to a previous page route", () => {
    const raw = JSON.stringify({
      tabs: journals().tabs,
      activeIndex: 0,
      layout: {
        kind: "split",
        dir: "row",
        ratio: 0.5,
        children: [
          { kind: "pane", paneId: "main", ...journals() },
          {
            kind: "pane",
            paneId: "pane-2",
            tabs: [
              {
                history: [
                  { kind: "page", name: "Previous", pageKind: "page" },
                  { kind: "journals" },
                ],
                pos: 1,
                pinned: false,
              },
            ],
            activeIndex: 0,
          },
        ],
      },
    });

    const parsed = parsePersistedSession(raw)!;

    expect(parsed.layout).toEqual({
      kind: "split",
      dir: "row",
      ratio: 0.5,
      children: [
        { kind: "pane", paneId: "main" },
        { kind: "pane", paneId: "pane-2" },
      ],
    });
    expect(parsed.snapshots.get("main")?.tabs[0].history[0]).toEqual({ kind: "journals" });
    expect(parsed.snapshots.get("pane-2")?.tabs[0].history[1]).toEqual({
      kind: "page",
      name: "Previous",
      pageKind: "page",
    });
  });
});
