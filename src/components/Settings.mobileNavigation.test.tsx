import { afterEach, describe, expect, it, vi } from "vitest";

// GH #716 (D-20261018-N1): "Navigation buttons at the bottom" is a device-local
// setting, default off, offered (and searchable) only on a phone or tablet app.
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

async function open(platform: "android" | "desktop") {
  vi.resetModules();
  globalThis.__TINE_PLATFORM__ = platform;
  const { render } = await import("solid-js/web");
  const { backend } = await import("../backend");
  const { Settings } = await import("./Settings");
  const ui = await import("../ui");
  const nav = await import("../navSettings");
  nav.resetMobileNavigationAtBottomForTests(false);
  const saved = vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  ui.openSettings("appearance");
  await tick();
  return { root, dispose, saved, nav, ui };
}

const field = (root: HTMLElement) => root.querySelector<HTMLElement>('[data-setting-label="Navigation buttons at the bottom"]');

describe("Settings: Navigation buttons at the bottom (GH #716)", () => {
  afterEach(() => {
    delete globalThis.__TINE_PLATFORM__;
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  it("android: off by default, the switch turns the bottom row on and remembers it on this device", async () => {
    const { root, dispose, saved, nav } = await open("android");
    try {
      const row = field(root)!;
      expect(row).not.toBeNull();
      const toggle = row.querySelector<HTMLButtonElement>('button[role="switch"]')!;
      expect(toggle.getAttribute("aria-checked")).toBe("false");
      expect(nav.mobileNavigationAtBottom()).toBe(false);
      toggle.click();
      await tick();
      expect(nav.mobileNavigationAtBottom()).toBe(true);
      expect(saved).toHaveBeenCalledWith("mobile_navigation_at_bottom", true);
      expect(field(root)!.querySelector('button[role="switch"]')!.getAttribute("aria-checked")).toBe("true");
    } finally { dispose(); }
  });

  it("android: Settings search finds it", async () => {
    const { root, dispose } = await open("android");
    try {
      const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
      search.value = "thumb";
      search.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await tick();
      expect(root.querySelector(".settings-search-result")?.textContent).toContain("Navigation buttons at the bottom");
    } finally { dispose(); }
  });

  it("desktop: neither the switch nor a search hit is offered", async () => {
    const { root, dispose } = await open("desktop");
    try {
      expect(field(root)).toBeNull();
      const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
      search.value = "thumb";
      search.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await tick();
      expect(root.querySelector(".settings-search-result")).toBeNull();
    } finally { dispose(); }
  });
});
