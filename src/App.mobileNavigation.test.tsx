import { afterEach, describe, expect, it, vi } from "vitest";

// GH #716 (D-20261018-N1): on a phone or tablet app, Sidebar / Search / Back /
// Forward are in the top bar by default, the same place whether or not the user
// is editing. "Navigation buttons at the bottom" (default off) restores the
// GH #598 bottom row. Platform is read once at module load, so each case loads a
// fresh module graph under the platform it needs.
async function mount(platform: "android" | "ios" | "desktop", bottom: boolean) {
  vi.resetModules();
  globalThis.__TINE_PLATFORM__ = platform;
  const { render } = await import("solid-js/web");
  const { initParser } = await import("./render/parse");
  await initParser();
  const nav = await import("./navSettings");
  nav.resetMobileNavigationAtBottomForTests(bottom);
  const { App } = await import("./App");
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <App />, host);
  return { host, dispose, nav };
}

const SELECTORS = ["[data-search-trigger]", 'button[title="Go back"]', 'button[title="Go forward"]', 'button[title="Toggle sidebar (t l)"]'];

describe("mobile navigation placement (GH #716)", { timeout: 30_000 }, () => {
  afterEach(() => {
    delete globalThis.__TINE_PLATFORM__;
    document.documentElement.removeAttribute("data-mobile-editing-dock");
    document.body.innerHTML = "";
  });

  for (const platform of ["android", "ios"] as const) {
    it(`${platform}: the four buttons are in the top bar by default, with no bottom row`, async () => {
      const { host, dispose } = await mount(platform, false);
      try {
        const top = host.querySelector("header.topbar")!;
        for (const selector of SELECTORS) expect(top.querySelector(selector), selector).not.toBeNull();
        expect(host.querySelector(".mobile-navigation-toolbar")).toBeNull();
        // The same place while editing: the editing dock does not move them.
        document.documentElement.setAttribute("data-mobile-editing-dock", "");
        for (const selector of SELECTORS) expect(top.querySelector(selector), selector).not.toBeNull();
        expect(host.querySelectorAll("[data-search-trigger]")).toHaveLength(1);
      } finally { dispose(); }
    });

    it(`${platform}: with Navigation buttons at the bottom on, they are in the bottom row only, and the setting is live`, async () => {
      const { host, dispose, nav } = await mount(platform, true);
      try {
        const bottom = host.querySelector(".mobile-navigation-toolbar")!;
        expect(bottom).not.toBeNull();
        for (const selector of SELECTORS) expect(bottom.querySelector(selector), selector).not.toBeNull();
        expect(host.querySelector("header.topbar [data-search-trigger]")).toBeNull();
        expect(host.querySelectorAll("[data-search-trigger]")).toHaveLength(1);
        nav.resetMobileNavigationAtBottomForTests(false);
        await vi.waitFor(() => expect(host.querySelector("header.topbar [data-search-trigger]")).not.toBeNull());
        expect(host.querySelector(".mobile-navigation-toolbar")).toBeNull();
      } finally { dispose(); }
    });
  }

  it("desktop: the setting changes nothing (top bar, no bottom row)", async () => {
    const { host, dispose } = await mount("desktop", true);
    try {
      expect(host.querySelector("header.topbar [data-search-trigger]")).not.toBeNull();
      expect(host.querySelector(".mobile-navigation-toolbar")).toBeNull();
    } finally { dispose(); }
  });
});
