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
});
