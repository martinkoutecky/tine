import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { Backend } from "../backend";
import { closeSettings, openSettings } from "../ui";
import { primeQuickShareGate, resetQuickShareGateForTest } from "../quickShareGate";

const sharingBackend = vi.hoisted(() => ({
  quickShareList: vi.fn(async () => ({ enabled: true, account: null, shares: [], error: null })),
}));

vi.mock("../backend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../backend")>();
  return { ...actual, backend: () => sharingBackend as unknown as Backend, isTauri: () => false };
});

import { Settings } from "./Settings";

afterEach(() => {
  closeSettings();
  document.body.innerHTML = "";
  resetQuickShareGateForTest();
  vi.clearAllMocks();
});

// UI-SETTINGS-SHARING-TAB-SHIFT: Settings read the quick share gate on every
// open and inserted the Sharing tab after the list had painted, pushing
// Plugins and every tab below it down one row under a click aimed at one of
// them (the 0.9.0-beta.1 plugin-revocation journey's click on Plugins opened
// Sharing). Graph load primes the gate, so the list is final when it appears.
describe("Settings tab list with quick share offered", () => {
  it("shows Sharing in its place as soon as the dialog opens", async () => {
    await primeQuickShareGate();
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("appearance");
    const labels = [...root.querySelectorAll(".settings-nav-item")].map((node) => node.textContent?.trim() ?? "");
    const sharing = labels.findIndex((label) => label.startsWith("Sharing"));
    expect(sharing, labels.join(" | ")).toBeGreaterThanOrEqual(0);
    expect(labels.indexOf("Graph")).toBe(sharing - 1);
    dispose();
  });

  it("reads the gate once per session, not once per open", async () => {
    await primeQuickShareGate();
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("appearance");
    closeSettings();
    openSettings("appearance");
    await Promise.resolve();
    expect(sharingBackend.quickShareList).toHaveBeenCalledTimes(1);
    dispose();
  });
});
