import { beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { AboutTab } from "./AboutTab";
import { setToasts, toasts } from "../toasts";

const { getVersionMock, copyVersionMock, isTauriMock, platformKindMock, openExternalMock, checkNowMock, getAppBoolMock, setAppBoolMock } = vi.hoisted(() => ({
  getAppBoolMock: vi.fn(async () => true),
  setAppBoolMock: vi.fn(async () => {}),
  getVersionMock: vi.fn(async () => "0.5.3"),
  copyVersionMock: vi.fn(async (_text: string) => {}),
  checkNowMock: vi.fn(async (): Promise<{ kind: string; version?: string; current?: string }> => ({ kind: "current", version: "0.5.3" })),
  isTauriMock: vi.fn(() => false),
  platformKindMock: vi.fn(async (): Promise<"desktop" | "android" | "ios"> => "desktop"),
  openExternalMock: vi.fn(async () => {}),
}));

vi.mock("../backend", () => ({
  isTauri: isTauriMock,
  backend: () => ({ openExternal: openExternalMock, getAppBool: getAppBoolMock, setAppBool: setAppBoolMock }),
}));
vi.mock("../platform", () => ({ platformKind: platformKindMock }));
vi.mock("../update", () => ({
  checkForUpdateNow: checkNowMock,
  openReleasesPage: () => {},
}));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: getVersionMock }));
vi.mock("../clipboard", () => ({ writeClipboardTextStrict: copyVersionMock }));
import { IDENTITY } from "../../scripts/lib/app-identity.mjs";

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
    openExternalMock.mockResolvedValue(undefined);
    setToasts([]);
  });

  it("GH #618: exposes a device-local automatic-check toggle and keeps manual checks", async () => {
    isTauriMock.mockReturnValue(true);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <AboutTab />, host);
    try {
      await flush();
      const checkbox = host.querySelector<HTMLButtonElement>('[role="switch"][aria-label="Check for updates automatically"]');
      expect(checkbox, "automatic updates must be configurable in About").not.toBeNull();
      expect(checkbox!.getAttribute("aria-checked")).toBe("true");
      expect(host.textContent).toContain("Check for updates automatically");
      checkbox!.click();
      await flush();
      expect(checkbox!.getAttribute("aria-checked")).toBe("false");
      expect(setAppBoolMock).toHaveBeenCalledWith("check_for_updates_automatically", false);
      const manual = [...host.querySelectorAll("button")].find((b) => b.textContent === "Check for updates");
      manual!.click();
      await flush();
      expect(checkNowMock).toHaveBeenCalledOnce();
    } finally { dispose(); host.remove(); }
  });

  it("I-20: a platform lookup that lands after the tab closed starts no update-settings read", async () => {
    isTauriMock.mockReturnValue(true);
    let release!: (kind: "desktop") => void;
    platformKindMock.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <AboutTab />, host);
    await flush();
    dispose();
    host.remove();
    getAppBoolMock.mockClear();
    getVersionMock.mockClear();
    release("desktop");
    await flush();
    expect(getAppBoolMock).not.toHaveBeenCalled();
    expect(getVersionMock).not.toHaveBeenCalled();
  });

  it("displays and copies the channel with the full prerelease version", async () => {
    isTauriMock.mockReturnValue(true);
    getVersionMock.mockResolvedValueOnce("0.7.0-beta.1");
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <AboutTab />, host);
    try {
      await flush();
      const label = `${IDENTITY.productName} 0.7.0-beta.1`;
      expect(host.querySelector(".about-name")?.textContent).toBe(IDENTITY.productName);
      expect(host.querySelector(".about-ver-num")?.textContent).toBe(label);
      const copy = [...host.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === "Copy version");
      expect(copy).toBeDefined();
      copy!.click();
      await flush();
      expect(copyVersionMock).toHaveBeenCalledWith(label);
    } finally { dispose(); host.remove(); }
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
      expect(host.textContent).toContain(`${IDENTITY.productName} 0.5.3`);
      expect(host.textContent).toContain("Check for updates");
      expect(host.textContent).not.toContain("distribution channel");
    } finally {
      dispose();
    }
  });

  it("points an available update at the Install update action instead of claiming a download (GH #241)", async () => {
    isTauriMock.mockReturnValue(true);
    checkNowMock.mockResolvedValueOnce({ kind: "available", version: "0.6.0", current: "0.5.3" });
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <AboutTab />, host);
    try {
      await flush();
      const button = [...host.querySelectorAll("button")].find((b) => b.textContent?.includes("Check for updates"));
      button?.click();
      await flush();
      expect(host.textContent).toContain(`${IDENTITY.productName} 0.6.0 is available — choose Install update in the notification.`);
      expect(host.textContent).not.toContain("downloading");
    } finally {
      dispose();
      host.remove();
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
      // App Store builds carry no donation link (master b89a9e77f).
      if (platform === "ios") expect(host.textContent).not.toContain("Ko-fi");
      else expect(host.textContent).toContain("Ko-fi");
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
      expect(host.textContent).not.toContain("Ko-fi");
    } finally {
      dispose();
    }
  });

  it("reports an external-link failure with fixed text", async () => {
    openExternalMock.mockRejectedValue(new Error("private path /graph/secret"));
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <AboutTab />, host);
    try {
      (host.querySelector(".about-link") as HTMLButtonElement).click();
      await flush();
      expect(toasts().map((toast) => toast.message)).toContain("Couldn't open the link.");
      expect(toasts().map((toast) => toast.message).join(" ")).not.toContain("/graph/secret");
    } finally {
      dispose();
      host.remove();
    }
  });
});
