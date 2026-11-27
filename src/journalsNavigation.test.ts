import { afterEach, describe, expect, it, vi } from "vitest";
import { openJournalsFromClick } from "./journalsNavigation";
import { focusedPaneId, layoutPaneIds, paneRouter, resetPaneLayoutToSingle, restorePaneLayout } from "./panes";
import { tabRoute, type Route } from "./router";
import { applySidebarSession, rightSidebar } from "./ui";
import { appNow, journalTitle } from "./journal";

const viewport = vi.hoisted(() => ({ singlePane: false }));
vi.mock("./viewportClass", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./viewportClass")>()),
  isSinglePaneShell: () => viewport.singlePane,
}));

// GH #1050 (and #1014): Ctrl/Cmd, middle-click and Alt on Journals take the page-link destinations, after
// bringing forward a feed some tab of the window already shows.
const feed: Route = { kind: "journals" };
const project: Route = { kind: "page", name: "Project", pageKind: "page" };
const other: Route = { kind: "page", name: "Other", pageKind: "page" };
const tab = (route: Route) => ({ history: [route], pos: 0, pinned: false });

/** `main` is focused; `pane2`, when given, is a second pane beside it. Each lists its tabs; the first is active. */
function layout(main: Route[], pane2?: Route[]) {
  if (!pane2) return resetPaneLayoutToSingle({ tabs: main.map(tab), activeIndex: 0 });
  restorePaneLayout({ kind: "split", dir: "row", ratio: 0.5, children: [{ kind: "pane", paneId: "main" }, { kind: "pane", paneId: "pane-2" }] },
    new Map([["main", { tabs: main.map(tab), activeIndex: 0 }], ["pane-2", { tabs: pane2.map(tab), activeIndex: 0 }]]), "main");
}
const feedTabs = () => layoutPaneIds().flatMap((id) => paneRouter(id).tabs().filter((t) => tabRoute(t).kind === "journals"));
const click = (init: MouseEventInit) => openJournalsFromClick(
  { button: 0, shiftKey: false, ctrlKey: false, metaKey: false, altKey: false, ...init } as MouseEvent);

const gestures = [["Ctrl", { ctrlKey: true }], ["Cmd", { metaKey: true }], ["middle", { button: 1 }], ["Alt", { altKey: true }]] as const;

afterEach(() => {
  viewport.singlePane = false;
  applySidebarSession({ right: false, items: [] });
  resetPaneLayoutToSingle({ tabs: [tab(feed)], activeIndex: 0 });
});

describe("a feed the window already holds is brought forward", () => {
  const holders = [
    ["the same pane, active", () => layout([feed]), "main"],
    ["the same pane, in a background tab", () => layout([project, feed]), "main"],
    ["another pane, active", () => layout([project], [feed]), "pane-2"],
    ["another pane, in a background tab", () => layout([project], [other, feed]), "pane-2"],
  ] as const;
  for (const [where, setUp, holder] of holders) {
    it.each(gestures)(`%s-click with the feed in ${where}`, (_name, init) => {
      setUp();
      const panes = layoutPaneIds();
      const tabCounts = panes.map((id) => paneRouter(id).tabs().length);
      click(init);
      expect(layoutPaneIds()).toEqual(panes);
      expect(panes.map((id) => paneRouter(id).tabs().length)).toEqual(tabCounts);
      expect(feedTabs()).toHaveLength(1);
      expect(focusedPaneId()).toBe(holder);
      expect(paneRouter(holder).route()).toEqual(feed);
      if (holder !== "main") expect(paneRouter("main").route()).toEqual(project);
    });
  }
});

it.each(gestures)("%s-click with the feed active in a later pane looks at no background tab", (_name, init) => {
  layout([project, feed], [feed]);
  const backgroundLookups = vi.spyOn(paneRouter("main"), "tabs");
  click(init);
  expect(focusedPaneId()).toBe("pane-2");
  expect(paneRouter("main").route()).toEqual(project);
  expect(backgroundLookups).not.toHaveBeenCalled();
  backgroundLookups.mockRestore();
});

describe("with no feed open", () => {
  it.each(gestures.slice(0, 3))("%s-click opens it in a background tab and the page stays", (_name, init) => {
    layout([project], [other]);
    click(init);
    expect(paneRouter("main").route()).toEqual(project);
    expect(paneRouter("main").tabs().map(tabRoute)).toEqual([project, feed]);
    expect(paneRouter("pane-2").tabs().map(tabRoute)).toEqual([other]);
    expect(focusedPaneId()).toBe("main");
  });

  it("Alt-click opens it in the other pane and the page stays", () => {
    layout([project], [other]);
    click({ altKey: true });
    expect(paneRouter("main").tabs().map(tabRoute)).toEqual([project]);
    expect(paneRouter("pane-2").route()).toEqual(feed);
    expect(focusedPaneId()).toBe("main");
  });

  it("Alt-click on a single pane splits it and opens the feed beside the page", () => {
    layout([project]);
    click({ altKey: true });
    expect(layoutPaneIds()).toHaveLength(2);
    expect(paneRouter("main").route()).toEqual(project);
    expect(paneRouter(layoutPaneIds()[1]).route()).toEqual(feed);
    expect(feedTabs()).toHaveLength(1);
  });

  it("Alt-click on a layout that cannot split opens the feed here, as a plain click does", () => {
    viewport.singlePane = true;
    layout([project]);
    click({ altKey: true });
    expect(layoutPaneIds()).toEqual(["main"]);
    expect(paneRouter("main").route()).toEqual(feed);
    expect(feedTabs()).toHaveLength(1);
  });
});

it.each([["Shift+Alt", { shiftKey: true, altKey: true }], ["Shift+Ctrl", { shiftKey: true, ctrlKey: true }]] as const)(
  "%s-click still opens today in the right sidebar and leaves the panes alone", (_name, init) => {
    layout([project], [other]);
    click(init);
    expect(rightSidebar()).toContainEqual(expect.objectContaining({ kind: "page", name: journalTitle(appNow()), pageKind: "journal" }));
    expect(paneRouter("main").tabs().map(tabRoute)).toEqual([project]);
    expect(paneRouter("pane-2").tabs().map(tabRoute)).toEqual([other]);
    expect(feedTabs()).toHaveLength(0);
  });
