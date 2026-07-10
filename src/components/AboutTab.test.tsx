import { beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { AboutTab } from "./AboutTab";

const { isTauriMock, isMobileMock } = vi.hoisted(() => ({
  isTauriMock: vi.fn(() => false),
  isMobileMock: vi.fn(async () => false),
}));

vi.mock("../backend", () => ({
  isTauri: isTauriMock,
  backend: () => ({
    openExternal: async () => {},
  }),
}));
vi.mock("../platform", () => ({ isMobile: isMobileMock }));
vi.mock("../update", () => ({
  checkForUpdateNow: async () => ({ kind: "current", version: "0.5.2" }),
  openReleasesPage: () => {},
}));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: async () => "0.5.2" }));

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

// Guards the deliberate role-credit phrasing (GH #32 discussion): Martin is the
// author/director, Claude Code & Codex are collaborators — NOT "created by …"
// (erases him) nor "created with …" (reduces them to tools). If someone rewrites
// this line, this test makes them do it on purpose.
describe("AboutTab", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    isTauriMock.mockReturnValue(false);
    isMobileMock.mockResolvedValue(false);
  });

  it("renders the role-based credits and project links", () => {
    const host = document.createElement("div");
    const dispose = render(() => <AboutTab />, host);
    try {
      const text = host.textContent ?? "";
      expect(text).toContain("Martin Koutecký");
      expect(text).toContain("direction, design, and authorship");
      expect(text).toContain("Claude Code & Codex");
      expect(text).toContain("engineering and analysis");
      // The three primary links #32 asked for + the phrasing must stay neutral.
      expect(text).toContain("tine.page");
      expect(text).toContain("GitHub");
      expect(text).toContain("Ko-fi");
      expect(text).not.toMatch(/created (by|with)/i);
    } finally {
      dispose();
    }
  });

  it("shows the explicit update check on desktop Tauri", async () => {
    isTauriMock.mockReturnValue(true);
    isMobileMock.mockResolvedValue(false);
    const host = document.createElement("div");
    const dispose = render(() => <AboutTab />, host);
    try {
      await flush();
      const text = host.textContent ?? "";
      expect(text).toContain("Version 0.5.2");
      expect(text).toContain("Check for updates");
      expect(text).not.toContain("distribution channel");
    } finally {
      dispose();
    }
  });

  it("hides the update check on mobile Tauri and explains distribution-channel updates", async () => {
    isTauriMock.mockReturnValue(true);
    isMobileMock.mockResolvedValue(true);
    const host = document.createElement("div");
    const dispose = render(() => <AboutTab />, host);
    try {
      await flush();
      const text = host.textContent ?? "";
      expect(text).not.toContain("Check for updates");
      expect(text).toContain("Updates arrive through your app's distribution channel");
      expect(text).not.toContain("F-Droid / Play Store");
    } finally {
      dispose();
    }
  });
});
