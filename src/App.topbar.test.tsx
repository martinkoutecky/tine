import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { App } from "./App";
import { setLeftSidebarOpen } from "./ui";
import { closeShareDialog, shareTarget } from "./shareState";
import { mainPaneRouter, resetTabsToJournals } from "./router";
import { applySidebarSession, rightSidebar } from "./ui";
import { appNow, journalTitle } from "./journal";
import { saveToolbarChoice } from "./toolbarChoice";
import { focusedPaneId, layoutPaneIds, paneRouter, resetPaneLayoutToSingle, restorePaneLayout } from "./panes";
import { applyParsedSession, buildPersistedSession, parsePersistedSession } from "./session";

let dispose = () => {};

afterEach(() => {
  dispose();
  dispose = () => {};
  saveToolbarChoice(null);
  setLeftSidebarOpen(true);
  applySidebarSession({ right: false, items: [] });
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  resetTabsToJournals();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("topbar workspace placement", () => {
  it.each(["button", "overflow"])("GH #740: Shift-click Journals in the %s opens today beside the current page", async (surface) => {
    const host = document.body.appendChild(document.createElement("div"));
    if (surface === "overflow") saveToolbarChoice({ bar: ["settings"], menu: ["journals", "calendar", "share", "theme", "right-sidebar", "workspaces"] });
    mainPaneRouter.openPage("Project", "page", { inPlace: true });
    const before = mainPaneRouter.route();
    dispose = render(() => <App />, host);
    if (surface === "overflow") host.querySelector<HTMLButtonElement>("[data-topbar-overflow-trigger]")!.click();
    const journals = host.querySelector<HTMLElement>(surface === "button"
      ? 'header.topbar button[title="Journals"]'
      : '[data-topbar-overflow-action="journals"]')!;
    journals.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
    expect(rightSidebar()).toContainEqual(expect.objectContaining({ kind: "page", name: journalTitle(appNow()), pageKind: "journal" }));
    expect(mainPaneRouter.route()).toEqual(before);

    // The ordinary click/tap still opens the multi-day feed.
    if (surface === "overflow") host.querySelector<HTMLButtonElement>("[data-topbar-overflow-trigger]")!.click();
    host.querySelector<HTMLElement>(surface === "button"
      ? 'header.topbar button[title="Journals"]'
      : '[data-topbar-overflow-action="journals"]')!.click();
    expect(mainPaneRouter.route()).toMatchObject({ kind: "journals" });
  });

  // GH #1050: the toolbar Journals controls follow the page-link contract (linkGesture.ts) for Ctrl/Cmd and Alt;
  // the button also takes the link's middle-click.
  it.each([
    ["button", "Ctrl", { ctrlKey: true }, "background"],
    ["button", "middle", { button: 1 }, "background"],
    ["button", "Alt", { altKey: true }, "pane"],
    ["overflow", "Ctrl", { ctrlKey: true }, "background"],
    ["overflow", "Alt", { altKey: true }, "pane"],
  ] as const)("GH #1050: %s %s-click Journals opens the feed like a page link", async (surface, gesture, modifier, destination) => {
    const host = document.body.appendChild(document.createElement("div"));
    if (surface === "overflow") saveToolbarChoice({ bar: ["settings"], menu: ["journals", "calendar", "share", "theme", "right-sidebar", "workspaces"] });
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name: "Project", pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
    const before = paneRouter("main").route();
    dispose = render(() => <App />, host);
    if (surface === "overflow") host.querySelector<HTMLButtonElement>("[data-topbar-overflow-trigger]")!.click();
    const journals = host.querySelector<HTMLElement>(surface === "button"
      ? 'header.topbar button[title="Journals"]'
      : '[data-topbar-overflow-action="journals"]')!;
    journals.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, ...modifier }));
    journals.dispatchEvent(new MouseEvent(gesture === "middle" ? "auxclick" : "click", { bubbles: true, cancelable: true, ...modifier }));
    expect(paneRouter("main").route()).toEqual(before);
    if (destination === "background") {
      expect(paneRouter("main").tabs()).toHaveLength(2);
      expect(paneRouter("main").tabs()[1].history[0]).toEqual({ kind: "journals" });
    } else {
      expect(layoutPaneIds()).toHaveLength(2);
      expect(paneRouter(layoutPaneIds()[1]).route()).toEqual({ kind: "journals" });
    }
  });

  // GH #1050 round 2: with the feed already open in the other pane, the toolbar's modifier and middle clicks focus
  // that feed (ADR 0032's one feed per window): no split, no second feed tab, and the feed survives restore.
  it.each([
    ["button", "Ctrl", { ctrlKey: true }],
    ["button", "middle", { button: 1 }],
    ["button", "Alt", { altKey: true }],
    ["overflow", "Ctrl", { ctrlKey: true }],
    ["overflow", "Alt", { altKey: true }],
  ] as const)("GH #1050: %s %s-click Journals with the feed open in the other pane focuses it", async (surface, gesture, modifier) => {
    const host = document.body.appendChild(document.createElement("div"));
    if (surface === "overflow") saveToolbarChoice({ bar: ["settings"], menu: ["journals", "calendar", "share", "theme", "right-sidebar", "workspaces"] });
    const project = { kind: "page", name: "Project", pageKind: "page" } as const;
    restorePaneLayout({ kind: "split", dir: "row", ratio: 0.5, children: [{ kind: "pane", paneId: "main" }, { kind: "pane", paneId: "pane-2" }] },
      new Map([["main", { tabs: [{ history: [project], pos: 0, pinned: false }], activeIndex: 0 }],
        ["pane-2", { tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 }]]), "main");
    dispose = render(() => <App />, host);
    if (surface === "overflow") host.querySelector<HTMLButtonElement>("[data-topbar-overflow-trigger]")!.click();
    const journals = host.querySelector<HTMLElement>(surface === "button"
      ? 'header.topbar button[title="Journals"]'
      : '[data-topbar-overflow-action="journals"]')!;
    journals.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, ...modifier }));
    journals.dispatchEvent(new MouseEvent(gesture === "middle" ? "auxclick" : "click", { bubbles: true, cancelable: true, ...modifier }));
    expect(layoutPaneIds()).toEqual(["main", "pane-2"]);
    expect(focusedPaneId()).toBe("pane-2");
    expect(paneRouter("main").tabs()).toHaveLength(1);
    expect(paneRouter("pane-2").tabs()).toHaveLength(1);
    applyParsedSession(parsePersistedSession(JSON.stringify(buildPersistedSession()))!);
    expect(layoutPaneIds()).toEqual(["main", "pane-2"]);
    expect(paneRouter("main").route()).toEqual(project);
    expect(paneRouter("pane-2").route()).toEqual({ kind: "journals" });
  });

  it("puts the full workspace switcher in the sidebar and only mounts a compact fallback while it is closed", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    setLeftSidebarOpen(true);
    dispose = render(() => <App />, host);

    const sidebarSwitcher = await vi.waitFor(() => {
      const switcher = host.querySelector<HTMLElement>("[data-workspace-switcher-sidebar] [data-workspace-switcher]");
      expect(switcher).not.toBeNull();
      return switcher!;
    });
    const topbar = host.querySelector<HTMLElement>("header.topbar")!;
    expect(sidebarSwitcher.closest(".left-sidebar-scroll")).not.toBeNull();
    expect(topbar.querySelector("[data-workspace-switcher]")).toBeNull();

    setLeftSidebarOpen(false);
    await vi.waitFor(() => expect(topbar.querySelector('[data-workspace-switcher-compact="true"]')).not.toBeNull());
    expect(host.querySelector("[data-workspace-switcher-sidebar] [data-workspace-switcher]")).toBeNull();
  });

  // GH #1024: in the toolbar the Workspace control is one of the grey icon buttons, without a caret; the
  // sidebar's labelled switcher keeps its caret.
  it.each(["fallback", "chosen"])("draws the %s toolbar Workspace control as an icon button without a caret", async (mount) => {
    const host = document.body.appendChild(document.createElement("div"));
    if (mount === "chosen") saveToolbarChoice({ bar: ["workspaces", "settings"], menu: ["journals", "calendar", "share", "theme", "right-sidebar"] });
    setLeftSidebarOpen(mount === "chosen");
    dispose = render(() => <App />, host);
    const control = await vi.waitFor(() => {
      const found = host.querySelector<HTMLElement>('header.topbar [data-workspace-switcher-compact="true"]');
      expect(found).not.toBeNull();
      return found!;
    });
    const button = control.querySelector<HTMLButtonElement>("button[aria-haspopup='menu']")!;
    expect(button.classList.contains("icon-btn")).toBe(true);
    expect(button.getAttribute("aria-label")).toMatch(/workspace/i);
    expect(control.querySelector(".workspace-switcher-caret")).toBeNull();
    expect(button.querySelector(".workspace-switcher-mark")?.textContent).toBe("W");
    if (mount === "chosen") expect(host.querySelector("[data-workspace-switcher-sidebar] .workspace-switcher-caret")).not.toBeNull();
  });

  it("shares the page in view from the topbar, and is disabled when no page is open", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    resetTabsToJournals();
    dispose = render(() => <App />, host);
    try {
      const button = await vi.waitFor(() => {
        const found = host.querySelector<HTMLButtonElement>("header.topbar .topbar-share");
        expect(found).not.toBeNull();
        return found!;
      });
      expect(button.disabled, "the journals feed is not a page").toBe(true);

      mainPaneRouter.openPage("Project", "page", { inPlace: true });
      await vi.waitFor(() => expect(button.disabled).toBe(false));
      button.click();
      expect(shareTarget()).toEqual({ page: "Project" });
    } finally {
      closeShareDialog();
      resetTabsToJournals();
    }
  });
});
