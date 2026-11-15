// GH #926 (D-20261111-QP2): the left drawer ends with a fixed Settings row, so
// Settings is two taps away whatever the toolbar shows.
import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { App } from "../App";
import * as ui from "../ui";

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); ui.closeSettings(); document.body.replaceChildren(); });

it.each([true, false])("GH #926: drawer mode=%s shows a Settings row outside the drawer's scroll only in the drawer", async (mobile) => {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: mobile && query === "(max-width: 639px)", addEventListener() {}, removeEventListener() {},
  }));
  ui.setLeftSidebarOpen(true);
  const host = document.body.appendChild(document.createElement("div"));
  const dispose = render(() => <App />, host);
  try {
    await vi.waitFor(() => expect(host.querySelector(".left-sidebar-scroll")).not.toBeNull());
    const row = host.querySelector<HTMLButtonElement>(".left-sidebar > .mobile-drawer-settings");
    if (!mobile) { expect(row).toBeNull(); return; }
    expect(row).not.toBeNull();
    expect(row!.closest(".left-sidebar-scroll")).toBeNull();
    row!.click();
    expect(ui.settingsOpen()).toBe(true);
    expect(ui.sidebarOpen()).toBe(false);
  } finally { ui.setLeftSidebarOpen(false); dispose(); }
});
