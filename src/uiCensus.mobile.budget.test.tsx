// UI census budget test, phone/tablet app (GA5). The platform is read once at
// module load, so this file declares it before anything else evaluates and
// measures the surfaces that differ on mobile: the top and bottom navigation
// rows, the keyboard toolbar and the context menus (no new tab/window/PDF
// there, plus the page header's Find in page). Budgets: scripts/lib/ui-census-budget.mjs.
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
vi.hoisted(() => { globalThis.__TINE_PLATFORM__ = "android"; });
// A phone is a touch-first device (D-20261018-W1: UI follows input, not the OS
// name), so the shell's touch surfaces (keyboard toolbar) mount as on a phone.
vi.mock("./inputCapability", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./inputCapability")>()),
  touchPrimary: () => true,
  touchCapable: () => true,
}));
import { checkSurfaceBudgets } from "../scripts/lib/ui-census-budget.mjs";
import { App } from "./App";
import { initParser } from "./render/parse";
import { resetMobileNavigationAtBottomForTests } from "./navSettings";
import { isMobilePlatform } from "./nativeChrome";
import { registerFocusedEditorCommandBridge } from "./editorCommandBridge";
import { describeControl, controlsIn, measureContextMenus, mount, required, settled } from "./uiCensus.test.support";

const counts: Record<string, number> = {};
const labels: Record<string, string[]> = {};

describe("UI census budgets (mobile)", () => {
  beforeAll(async () => {
    await initParser();
    expect(isMobilePlatform, "the mobile census must run as a mobile build").toBe(true);
    for (const bottom of [false, true]) {
      resetMobileNavigationAtBottomForTests(bottom);
      const app = mount(() => <App />);
      try {
        if (!bottom) {
          // The keyboard toolbar is mounted (hidden) with the shell: its buttons are the surface.
          const keyboard = await vi.waitFor(() => required(app.root.querySelector<HTMLElement>("[data-mobile-keyboard-toolbar]"), "the mobile keyboard toolbar"));
          counts["mobile.keyboardToolbar"] = controlsIn(keyboard, "button").length;
          labels["mobile.keyboardToolbar"] = controlsIn(keyboard, "button").map(describeControl);
        } else {
          // "Navigation buttons at the bottom" (default off) moves the four navigation buttons to a row.
          const row = await vi.waitFor(() => required(app.root.querySelector<HTMLElement>(".mobile-navigation-toolbar"), "the bottom navigation row"));
          counts["mobile.bottomBar"] = await settled(() => controlsIn(row).length);
          labels["mobile.bottomBar"] = controlsIn(row).map(describeControl);
          // The same setting puts the group in the keyboard toolbar while the keyboard is up (GH #598).
          // (Seed again: App's startup preference load can land after the first seed.)
          resetMobileNavigationAtBottomForTests(true);
          Object.defineProperty(window, "innerHeight", { value: 800, configurable: true });
          Object.defineProperty(window, "visualViewport", { value: { height: 520, offsetTop: 0, addEventListener() {}, removeEventListener() {} }, configurable: true });
          const unregister = registerFocusedEditorCommandBridge({ blockId: "census", dispatch: () => true, blur() {} });
          try {
            const up = await vi.waitFor(() => required(app.root.querySelector<HTMLElement>("[data-mobile-keyboard-toolbar]:not([hidden])"), "the keyboard toolbar with the keyboard up"));
            counts["mobile.keyboardToolbar.withNavigation"] = await settled(() => controlsIn(up, "button").length);
            labels["mobile.keyboardToolbar.withNavigation"] = controlsIn(up, "button").map(describeControl);
          } finally { unregister(); }
        }
      } finally { app.dispose(); }
    }
    resetMobileNavigationAtBottomForTests(false);
    measureContextMenus(counts, labels, "mobile.menu", false);
  }, 60_000);
  afterAll(() => {
    if (process.env.UI_CENSUS_PRINT) console.info(`UI_CENSUS ${JSON.stringify({ counts, labels }, null, 1)}`);
  });

  it("holds every mobile surface within its budget", () => {
    const { slack } = checkSurfaceBudgets(counts, "mobile");
    for (const line of slack) console.warn(`UI census: ${line}`);
  });
});
