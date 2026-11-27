import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { applySidebarSession, rightSidebar, setFavorites, setRecentPages } from "../ui";
import { setToasts, toasts, toUiText } from "../toasts";
import { renderEnglish } from "../i18n";
import { GraphSwitcher, Sidebar } from "./Sidebar";
import { mainPaneRouter, openJournals, openPageInNewTab, resetTabsToJournals, route, tabRoute, type Route } from "../router";
import { appNow, journalTitle } from "../journal";
import { runGlobalCommand } from "../keybindings";
import { readFileSync } from "node:fs";
import { focusedPaneId, layoutPaneIds, paneRouter, resetPaneLayoutToSingle, restorePaneLayout, type LayoutNode } from "../panes";
import { applyParsedSession, buildPersistedSession, parsePersistedSession } from "../session";

it("GH #926: the named close icon shares the graph picker row and first-level rows share a casing class", () => {
  setFavorites([{ name: "Tine", kind: "page" }]);
  setRecentPages([{ name: "Tine", kind: "page" }]);
  mainPaneRouter.openPage("Project", "page", { inPlace: false });
  openPageInNewTab("Second project", "page");
  const close = vi.fn();
  const host = document.body.appendChild(document.createElement("div"));
  const dispose = render(() => <Sidebar onClose={close} />, host);
  try {
    const button = host.querySelector<HTMLButtonElement>('button[aria-label="Close navigation sidebar"]');
    expect(button).not.toBeNull();
    expect(button!.textContent).toBe("");
    expect(button!.querySelector('svg[aria-hidden="true"]')).not.toBeNull();
    expect(button!.parentElement).toBe(host.querySelector(".graph-switch")!.parentElement);
    button!.click();
    expect(close).toHaveBeenCalledOnce();
    const rows = [...host.querySelectorAll(".nav-section-header, .nav-contents > .nav-item")];
    expect(rows.length).toBeGreaterThanOrEqual(5);
    for (const row of rows) expect(row.classList.contains("nav-first-level")).toBe(true);
    expect(host.textContent).toContain("Favorites");
    expect(host.textContent).toContain("Recent");
  } finally { dispose(); }
});

it("GH #740: Shift-click the top-left Journals entry keeps the main page and opens today in the right sidebar", () => {
  mainPaneRouter.openPage("Project", "page", { inPlace: true });
  const before = route();
  const completed = vi.fn();
  const root = document.body.appendChild(document.createElement("div"));
  const dispose = render(() => <Sidebar onActiveNavigationComplete={completed} />, root);
  try {
    const journals = [...root.querySelectorAll<HTMLElement>(".nav-item")].find(el => el.textContent === "Journals")!;
    journals.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
    expect(rightSidebar()).toContainEqual(expect.objectContaining({ kind: "page", name: journalTitle(appNow()), pageKind: "journal" }));
    expect(route()).toEqual(before);
    expect(completed).not.toHaveBeenCalled();

    // A plain click/tap still completes active navigation (closing the mobile drawer).
    journals.click();
    expect(route()).toMatchObject({ kind: "journals" });
    expect(completed).toHaveBeenCalledOnce();

    mainPaneRouter.openPage("Project", "page", { inPlace: true });
    expect(runGlobalCommand("go/journals")).toBe(true);
    expect(route()).toMatchObject({ kind: "journals" });
  } finally { dispose(); }
});

// GH #1050: Ctrl/Cmd+click and Alt+click on Journals do what they do on a page link (linkGesture.ts): the feed
// opens in a background tab or the other pane, and the main page stays. Logseq ignores both modifiers here.
it.each([
  ["Ctrl", { ctrlKey: true }, "background"],
  ["Cmd", { metaKey: true }, "background"],
  ["Alt", { altKey: true }, "pane"],
] as const)("GH #1050: %s-click the left sidebar Journals entry opens the feed like a page link", (_name, modifier, destination) => {
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name: "Project", pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
  const before = paneRouter("main").route();
  const completed = vi.fn();
  const root = document.body.appendChild(document.createElement("div"));
  const dispose = render(() => <Sidebar onActiveNavigationComplete={completed} />, root);
  try {
    const journals = [...root.querySelectorAll<HTMLElement>(".nav-item")].find(el => el.textContent === "Journals")!;
    journals.dispatchEvent(new MouseEvent("click", { bubbles: true, ...modifier }));
    expect(paneRouter("main").route()).toEqual(before);
    if (destination === "background") {
      expect(paneRouter("main").tabs()).toHaveLength(2);
      expect(paneRouter("main").tabs()[1].history[0]).toEqual({ kind: "journals" });
    } else {
      expect(layoutPaneIds()).toHaveLength(2);
      expect(paneRouter(layoutPaneIds()[1]).route()).toEqual({ kind: "journals" });
    }
    expect(completed).not.toHaveBeenCalled();
  } finally {
    dispose();
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  }
});

// GH #1050 round 2: a window holds one feed (ADR 0032), so when one is already open, in any pane or tab, a
// modifier click focuses it like a plain click; no split and no second feed tab, and the feed survives restore.
describe("GH #1050: a modifier click on Journals with the feed already open focuses that feed", () => {
  const tab = (route: Route) => ({ history: [route], pos: 0, pinned: false });
  const project: Route = { kind: "page", name: "Project", pageKind: "page" };
  const feed: Route = { kind: "journals" };
  const panes = (...routes: Route[]) => {
    const ids = routes.map((_, i) => i === 0 ? "main" : `pane-${i + 1}`);
    const node = (i: number): LayoutNode => i === ids.length - 1
      ? { kind: "pane", paneId: ids[i] }
      : { kind: "split", dir: "row", ratio: 0.5, children: [{ kind: "pane", paneId: ids[i] }, node(i + 1)] };
    restorePaneLayout(node(0), new Map(ids.map((id, i) => [id, { tabs: [tab(routes[i])], activeIndex: 0 }])), "main");
  };
  const feedTabs = () => layoutPaneIds().flatMap(id => paneRouter(id).tabs().filter(t => tabRoute(t).kind === "journals"));
  const clickJournals = (modifier: MouseEventInit) => {
    const root = document.body.appendChild(document.createElement("div"));
    const dispose = render(() => <Sidebar />, root);
    try {
      const journals = [...root.querySelectorAll<HTMLElement>(".nav-item")].find(el => el.textContent === "Journals")!;
      journals.dispatchEvent(new MouseEvent("click", { bubbles: true, ...modifier }));
    } finally { dispose(); }
  };
  const restoreSavedSession = () => applyParsedSession(parsePersistedSession(JSON.stringify(buildPersistedSession()))!);
  afterEach(() => resetPaneLayoutToSingle({ tabs: [tab(feed)], activeIndex: 0 }));

  it("Alt-click with the feed in the only pane creates no split", () => {
    resetPaneLayoutToSingle({ tabs: [tab(feed)], activeIndex: 0 });
    clickJournals({ altKey: true });
    expect(layoutPaneIds()).toEqual(["main"]);
    expect(paneRouter("main").tabs()).toHaveLength(1);
    expect(paneRouter("main").route()).toEqual(feed);
  });

  it("Alt-click with the feed in a background tab of the only pane shows that tab", () => {
    resetPaneLayoutToSingle({ tabs: [tab(project), tab(feed)], activeIndex: 0 });
    clickJournals({ altKey: true });
    expect(layoutPaneIds()).toEqual(["main"]);
    expect(paneRouter("main").tabs()).toHaveLength(2);
    expect(paneRouter("main").route()).toEqual(feed);
  });

  it.each([
    ["the other pane", [project, feed], "pane-2"],
    ["a third pane", [project, project, feed], "pane-3"],
  ] as const)("Alt-click with the feed in %s focuses it there", (_where, routes, feedPane) => {
    panes(...routes);
    clickJournals({ altKey: true });
    expect(layoutPaneIds()).toHaveLength(routes.length);
    expect(focusedPaneId()).toBe(feedPane);
    expect(feedTabs()).toHaveLength(1);
    expect(paneRouter("main").route()).toEqual(project);
  });

  it.each([
    ["Ctrl", { ctrlKey: true }],
    ["Cmd", { metaKey: true }],
  ] as const)("%s-click opens no second feed tab, and the visible feed survives restore", (_name, modifier) => {
    panes(project, feed);
    clickJournals(modifier);
    expect(paneRouter("main").tabs()).toHaveLength(1);
    expect(feedTabs()).toHaveLength(1);
    expect(focusedPaneId()).toBe("pane-2");
    restoreSavedSession();
    expect(layoutPaneIds()).toEqual(["main", "pane-2"]);
    expect(paneRouter("pane-2").route()).toEqual(feed);
    expect(paneRouter("main").route()).toEqual(project);
  });

  // The singleton sees background tabs, so a plain click and the sidebar's middle-click answer the same way.
  it("a plain click with the feed in a background tab shows that tab instead of a second feed", () => {
    resetPaneLayoutToSingle({ tabs: [tab(feed), tab(project)], activeIndex: 1 });
    clickJournals({});
    expect(paneRouter("main").tabs()).toHaveLength(2);
    expect(paneRouter("main").route()).toEqual(feed);
    expect(feedTabs()).toHaveLength(1);
  });

  it("middle-click with the feed in the other pane focuses it and adds no tab", () => {
    panes(project, feed);
    const root = document.body.appendChild(document.createElement("div"));
    const dispose = render(() => <Sidebar />, root);
    try {
      const journals = [...root.querySelectorAll<HTMLElement>(".nav-item")].find(el => el.textContent === "Journals")!;
      journals.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 1 }));
      journals.dispatchEvent(new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }));
    } finally { dispose(); }
    expect(paneRouter("main").tabs()).toHaveLength(1);
    expect(feedTabs()).toHaveLength(1);
    expect(focusedPaneId()).toBe("pane-2");
  });
});

it("GH #761: sidebar sections and graph switcher share SVG disclosures that follow folding", () => {
  setFavorites([{ name: "Tine", kind: "page" }]);
  setRecentPages([{ name: "Tine", kind: "page" }]);
  const host = document.body.appendChild(document.createElement("div"));
  const dispose = render(() => <Sidebar />, host);
  try {
    const controls = [...host.querySelectorAll<HTMLElement>(".nav-section-toggle")];
    expect(controls.length).toBeGreaterThanOrEqual(2);
    for (const control of controls) {
      const caret = control.querySelector(".nav-toggle-caret")!;
      expect(caret.querySelector("svg")).not.toBeNull();
      const collapsed = caret.classList.contains("collapsed");
      control.click();
      expect(caret.classList.contains("collapsed")).toBe(!collapsed);
      control.click();
    }
    expect(host.querySelector(".graph-switch-caret svg")).not.toBeNull();
  } finally { dispose(); }
});

it("GH #335: All pages opens the full view directly", () => {
  openJournals();
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Sidebar />, root);
  try {
    const entry = [...root.querySelectorAll<HTMLElement>(".nav-section-header")].find(el => /all pages/i.test(el.textContent ?? ""))!;
    entry.click();
    expect(route()).toMatchObject({ kind: "all-pages" });
    expect(entry.hasAttribute("aria-expanded")).toBe(false);
    const guide = readFileSync("crates/tine-guide/src/templates/where-things-are.md", "utf8");
    expect(guide).toContain("**All pages** entry opens");
  } finally { dispose(); }
});

afterEach(() => {
  applySidebarSession({ right: false, items: [] });
  resetTabsToJournals();
  setFavorites([]);
  setRecentPages([]);
  setToasts([]);
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("graph open recovery (master 9a9122b1544d)", () => {
  it("keeps a known graph's open failure sticky and retries its same target", async () => {
    vi.spyOn(backend(), "listKnownGraphs").mockResolvedValue([
      { name: "Shared notes", path: "\\\\?\\C:\\shared-notes", display_path: "C:\\shared-notes" },
    ]);
    const openKnown = vi.fn(async () => {
      throw new Error("graph root is not readable");
    });
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <GraphSwitcher actions={{
      openKnown,
      openPicked: vi.fn(async () => ({ kind: "aborted" as const })),
      createNew: vi.fn(async () => ({ kind: "aborted" as const })),
    }} />, root);

    try {
      (root.querySelector(".graph-switch-btn") as HTMLButtonElement).click();
      const row = await vi.waitFor(() => {
        const current = root.querySelector<HTMLElement>(".graph-switch-row");
        expect(current).not.toBeNull();
        return current!;
      });
      row.click();
      expect(row.title).toBe("C:\\shared-notes");
      await vi.waitFor(() => expect(toasts()).toHaveLength(1));
      const failure = toasts()[0]!;
      expect(failure.kind).toBe("error");
      expect(failure.message).toContain("graph root is not readable");
      expect(failure.sticky).toBe(true);
      expect(renderEnglish(toUiText(failure.action!.label))).toBe("Retry");

      failure.action!.run();
      await vi.waitFor(() => expect(openKnown).toHaveBeenCalledTimes(2));
      expect(openKnown).toHaveBeenLastCalledWith("\\\\?\\C:\\shared-notes", false);
    } finally {
      dispose();
    }
  });
});

describe("left sidebar section disclosures", () => {
  it("collapses Favorites and Recent independently with semantic controls and counts", async () => {
    setFavorites([
      { name: "Favorite one", kind: "page" },
      { name: "Favorite two", kind: "page" },
    ]);
    setRecentPages([{ name: "Recent one", kind: "page" }]);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <Sidebar />, root);

    try {
      const favorites = await vi.waitFor(() => {
        const control = root.querySelector<HTMLButtonElement>('[data-sidebar-section="favorites"]');
        expect(control).not.toBeNull();
        return control!;
      });
      const recent = root.querySelector<HTMLButtonElement>('[data-sidebar-section="recent"]');
      expect(recent).not.toBeNull();
      expect(favorites.tagName).toBe("BUTTON");
      expect(favorites.getAttribute("aria-expanded")).toBe("true");
      expect(favorites.textContent).toContain("2");
      expect(recent!.getAttribute("aria-expanded")).toBe("true");
      expect(recent!.textContent).toContain("1");
      expect(root.textContent).toContain("Favorite one");
      expect(root.textContent).toContain("Recent one");

      favorites.click();
      expect(favorites.getAttribute("aria-expanded")).toBe("false");
      expect(root.textContent).not.toContain("Favorite one");
      expect(root.textContent).toContain("Recent one");

      recent!.click();
      expect(recent!.getAttribute("aria-expanded")).toBe("false");
      expect(root.textContent).not.toContain("Recent one");

      setFavorites([]);
      setRecentPages([]);
      expect(root.querySelector('[data-sidebar-section="favorites"]')).toBeNull();
      expect(root.querySelector('[data-sidebar-section="recent"]')).toBeNull();
    } finally {
      dispose();
    }
  });
});
