import { beforeEach, describe, expect, it } from "vitest";
import { buildPersistedSession, parsePersistedSession, type PersistedSession } from "./session";
import { resetPaneLayoutToSingle, restorePaneLayout, type LayoutNode } from "./panes";
import type { PaneSnapshot } from "./router";

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
