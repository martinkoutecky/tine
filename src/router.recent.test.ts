import { beforeEach, describe, expect, it } from "vitest";
import { createPaneRouter, type AdoptedTab, type PaneSnapshot } from "./router";
import { focusPane, paneRouter, resetPaneLayoutToSingle, splitPane } from "./panes";
import { clearRecent, recentPages } from "./ui";

const page = (name: string) => ({ kind: "page" as const, name, pageKind: "page" as const });

beforeEach(() => {
  clearRecent();
  resetPaneLayoutToSingle({
    tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
    activeIndex: 0,
  });
});

describe("foreground route activation RECENT semantics (GH #170)", () => {
  it("promotes foreground navigation but not background opens, restore, or Guide routes", () => {
    const router = createPaneRouter("recent-navigation");

    router.openPage("Alpha");
    router.openInNewTab(page("Background"), false);
    expect(recentPages().map((item) => item.name)).toEqual(["Alpha"]);

    router.openInNewTab(page("Foreground"), true);
    expect(recentPages().map((item) => item.name)).toEqual(["Foreground", "Alpha"]);

    clearRecent();
    const restored: PaneSnapshot = {
      tabs: [{ history: [page("Restored")], pos: 0, pinned: false }],
      activeIndex: 0,
    };
    expect(router.restoreSnapshot(restored)).toBe(true);
    router.openPage("Tine-guide/Tine Guide");
    expect(recentPages()).toEqual([]);
  });

  it("promotes Back, Forward, and activation of an already-open tab", () => {
    const router = createPaneRouter("recent-history");
    router.openPage("Alpha");
    router.openPage("Beta");
    const betaTab = router.activeId();
    router.openInNewTab(page("Gamma"), true);
    const gammaTab = router.activeId();

    clearRecent();
    router.setActiveTab(betaTab);
    expect(recentPages().map((item) => item.name)).toEqual(["Beta"]);
    router.goBack();
    expect(recentPages().map((item) => item.name)).toEqual(["Alpha", "Beta"]);
    router.goForward();
    expect(recentPages().map((item) => item.name)).toEqual(["Beta", "Alpha"]);
    router.setActiveTab(gammaTab);
    expect(recentPages().map((item) => item.name)).toEqual(["Gamma", "Beta", "Alpha"]);
  });

  it("promotes foreground reopen and adoption, but not background adoption", async () => {
    const router = createPaneRouter("recent-tab-lifecycle");
    router.openInNewTab(page("Closed"), true);
    const closedId = router.activeId();
    await router.closeTab(closedId);

    clearRecent();
    router.reopenClosedTab();
    expect(recentPages().map((item) => item.name)).toEqual(["Closed"]);

    const adopted: AdoptedTab = {
      history: [page("Adopted")],
      pos: 0,
      pinned: false,
      scroll: null,
    };
    clearRecent();
    router.adoptTab(adopted, false);
    expect(recentPages()).toEqual([]);
    router.adoptTab(adopted, true);
    expect(recentPages().map((item) => item.name)).toEqual(["Adopted"]);
  });

  it("promotes the page displayed by a newly focused split pane", () => {
    paneRouter("main").openPage("Left");
    const right = splitPane("main", "row", {
      focusNew: false,
      snapshot: {
        tabs: [{ history: [page("Right")], pos: 0, pinned: false }],
        activeIndex: 0,
      },
    })!;

    clearRecent();
    focusPane(right);
    expect(recentPages().map((item) => item.name)).toEqual(["Right"]);
    focusPane("main");
    expect(recentPages().map((item) => item.name)).toEqual(["Left", "Right"]);
  });
});
