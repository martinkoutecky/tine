import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { App } from "./App";
import { setLeftSidebarOpen } from "./ui";
import { closeShareDialog, shareTarget } from "./shareState";
import { mainPaneRouter, resetTabsToJournals } from "./router";
import { applySidebarSession, rightSidebar } from "./ui";
import { appNow, journalTitle } from "./journal";
import { saveToolbarChoice } from "./toolbarChoice";

let dispose = () => {};

afterEach(() => {
  dispose();
  dispose = () => {};
  saveToolbarChoice(null);
  setLeftSidebarOpen(true);
  applySidebarSession({ right: false, items: [] });
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
