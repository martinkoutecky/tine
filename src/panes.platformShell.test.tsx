import { afterEach, describe, expect, it, vi } from "vitest";

// GH #446 required neighbor: an iPad must keep split panes. Panes ask for the
// single-pane SHELL (a touch-first screen on a phone-shaped viewport), never the
// device's name (D-20261018-W1): a finger as the primary pointer decides, not
// the OS.
async function loadPanes(platform: "ios" | "desktop", width: number, height: number) {
  vi.resetModules();
  globalThis.__TINE_PLATFORM__ = platform;
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: platform === "ios" && query === "(pointer: coarse)",
    addEventListener: () => {}, removeEventListener: () => {},
  }));
  Object.defineProperty(window, "innerWidth", { configurable: true, value: width });
  Object.defineProperty(window, "innerHeight", { configurable: true, value: height });
  const { initParser } = await import("./render/parse");
  await initParser();
  const panes = await import("./panes");
  panes.resetPaneLayoutToSingle({
    tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
    activeIndex: 0,
  });
  return panes;
}

describe("split panes follow the device shell, not the OS (GH #446)", () => {
  afterEach(() => {
    delete globalThis.__TINE_PLATFORM__;
    vi.unstubAllGlobals();
  });

  it("splits on an iPad-sized iOS viewport", async () => {
    const panes = await loadPanes("ios", 820, 1180);
    expect(panes.splitPane("main", "row")).not.toBeNull();
    expect(panes.layoutPaneIds()).toHaveLength(2);
  });

  it("keeps the single-pane shell on an iPhone-sized iOS viewport", async () => {
    const panes = await loadPanes("ios", 430, 932);
    expect(panes.splitPane("main", "row")).toBeNull();
    expect(panes.layoutPaneIds()).toHaveLength(1);
  });

  it("splits on desktop regardless of window size", async () => {
    const panes = await loadPanes("desktop", 400, 300);
    expect(panes.splitPane("main", "row")).not.toBeNull();
  });
});
