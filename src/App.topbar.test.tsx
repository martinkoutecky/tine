import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { App } from "./App";
import { setLeftSidebarOpen } from "./ui";

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
});
