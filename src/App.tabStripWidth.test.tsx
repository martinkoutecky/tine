import { afterEach, describe, expect, it, vi } from "vitest";

// D-20261018-W1: the top bar's tab strip shows when there is room (a window
// 640 px or wider) on ANY platform, and hides below that, where the sidebar's
// Tabs section carries the tabs. The platform name is not consulted: the same
// matrix runs under android, ios and desktop.
async function mount(platform: "android" | "ios" | "desktop", narrow: boolean) {
  vi.resetModules();
  globalThis.__TINE_PLATFORM__ = platform;
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: query === "(max-width: 639px)" ? narrow : false,
    addEventListener: () => {}, removeEventListener: () => {},
  }));
  const { render } = await import("solid-js/web");
  const { initParser } = await import("./render/parse");
  await initParser();
  const { App } = await import("./App");
  const host = document.body.appendChild(document.createElement("div"));
  const dispose = render(() => <App />, host);
  return { host, dispose };
}

describe("tab strip follows available room, not the platform (D-20261018-W1)", { timeout: 30_000 }, () => {
  afterEach(() => {
    delete globalThis.__TINE_PLATFORM__;
    vi.unstubAllGlobals();
    document.body.replaceChildren();
  });

  for (const platform of ["android", "ios", "desktop"] as const) {
    it(`${platform}: wide window shows the strip`, async () => {
      const { host, dispose } = await mount(platform, false);
      try {
        expect(host.querySelector("header.topbar .tab-bar")).not.toBeNull();
      } finally { dispose(); }
    });
    it(`${platform}: narrow window hides it`, async () => {
      const { host, dispose } = await mount(platform, true);
      try {
        expect(host.querySelector("header.topbar .tab-bar")).toBeNull();
        expect(host.querySelector("header.topbar .topbar-spacer")).not.toBeNull();
      } finally { dispose(); }
    });
  }
});
