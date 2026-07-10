import { beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { AboutTab } from "./AboutTab";

const { isTauriMock, platformKindMock } = vi.hoisted(() => ({
  isTauriMock: vi.fn(() => false),
  platformKindMock: vi.fn(async (): Promise<"desktop" | "android" | "ios"> => "desktop"),
}));

vi.mock("../backend", () => ({
  isTauri: isTauriMock,
  backend: () => ({ openExternal: async () => {} }),
}));
vi.mock("../platform", () => ({ platformKind: platformKindMock }));
vi.mock("../update", () => ({
  checkForUpdateNow: async () => ({ kind: "current", version: "0.5.3" }),
  openReleasesPage: () => {},
}));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: async () => "0.5.3" }));

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

// Guards the deliberate role-credit phrasing (GH #32 discussion): Martin is the
// author/director, Claude Code & Codex are collaborators — NOT "created by …"
// (erases him) nor "created with …" (reduces them to tools). If someone rewrites
// this line, this test makes them do it on purpose.
describe("AboutTab", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    isTauriMock.mockReturnValue(false);
    platformKindMock.mockResolvedValue("desktop");
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
    platformKindMock.mockResolvedValue("desktop");
    const host = document.createElement("div");
    const dispose = render(() => <AboutTab />, host);
    try {
      await flush();
      expect(host.textContent).toContain("Version 0.5.3");
      expect(host.textContent).toContain("Check for updates");
      expect(host.textContent).not.toContain("distribution channel");
    } finally {
      dispose();
    }
  });

  it.each(["android", "ios"] as const)("hides self-update controls on %s", async (platform) => {
    isTauriMock.mockReturnValue(true);
    platformKindMock.mockResolvedValue(platform);
    const host = document.createElement("div");
    const dispose = render(() => <AboutTab />, host);
    try {
      await flush();
      expect(host.textContent).not.toContain("Check for updates");
      expect(host.textContent).toContain("Updates arrive through your app's distribution channel");
    } finally {
      dispose();
    }
  });

  it("fails closed when native platform detection fails", async () => {
    isTauriMock.mockReturnValue(true);
    platformKindMock.mockRejectedValue(new Error("platform unavailable"));
    const host = document.createElement("div");
    const dispose = render(() => <AboutTab />, host);
    try {
      await flush();
      expect(host.textContent).not.toContain("Check for updates");
      expect(host.textContent).not.toContain("distribution channel");
    } finally {
      dispose();
    }
  });
});
