import { afterEach, describe, expect, it, vi } from "vitest";

// A phone's left drawer must not cover the first page: it starts closed at
// launch and a restored session cannot reopen it (Play demo, 2026-10-14).
function mockViewport(phone: boolean) {
  Object.defineProperty(globalThis, "matchMedia", {
    configurable: true,
    value: () => ({ matches: phone, media: "(max-width: 639px)", addEventListener() {}, removeEventListener() {} }),
  });
}

async function freshUi(phone: boolean) {
  mockViewport(phone);
  vi.resetModules();
  return import("./ui");
}

describe("left drawer on first run", () => {
  afterEach(() => {
    localStorage.removeItem("logseq-claude.sidebarOpen");
    Reflect.deleteProperty(globalThis, "matchMedia");
  });

  it("starts closed on a phone with no stored preference", async () => {
    localStorage.removeItem("logseq-claude.sidebarOpen");
    const ui = await freshUi(true);
    expect(ui.sidebarOpen()).toBe(false);
  });

  it("keeps the desktop default open", async () => {
    localStorage.removeItem("logseq-claude.sidebarOpen");
    const ui = await freshUi(false);
    expect(ui.sidebarOpen()).toBe(true);
  });

  it("a restored session does not reopen it on a phone", async () => {
    const ui = await freshUi(true);
    ui.applySidebarSession({ left: true });
    expect(ui.sidebarOpen()).toBe(false);
    ui.setLeftSidebarOpen(true);
    expect(ui.sidebarOpen()).toBe(true);
  });

  it("a window narrow at launch gets its desktop sidebar back when it widens", async () => {
    localStorage.removeItem("logseq-claude.sidebarOpen");
    const listeners: (() => void)[] = [];
    let phone = true;
    Object.defineProperty(globalThis, "matchMedia", {
      configurable: true,
      value: () => ({
        get matches() { return phone; },
        media: "(max-width: 639px)",
        addEventListener: (_: string, f: () => void) => listeners.push(f),
        removeEventListener() {},
      }),
    });
    vi.resetModules();
    const ui = await import("./ui");
    const drawers = await import("./mobileDrawers");
    drawers.installMobileDrawerMode();
    ui.applySidebarSession({ left: true });
    expect(ui.sidebarOpen()).toBe(false);
    phone = false;
    listeners.forEach((f) => f());
    ui.normalizeSidebarDrawers();
    expect(ui.sidebarOpen()).toBe(true);
  });
});
