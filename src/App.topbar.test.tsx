import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { App } from "./App";
import { setLeftSidebarOpen } from "./ui";
import { closeShareDialog, shareTarget } from "./shareState";
import { mainPaneRouter, resetTabsToJournals } from "./router";

let dispose = () => {};

afterEach(() => {
  dispose();
  dispose = () => {};
  setLeftSidebarOpen(true);
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("topbar workspace placement", () => {
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
