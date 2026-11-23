import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { closeSettings, openSettings, type SettingsTabId } from "../ui";
import { setToasts, toasts } from "../toasts";
import { Settings } from "./Settings";

afterEach(() => {
  closeSettings();
  document.body.innerHTML = "";
  setToasts([]);
});

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

// The browser preview (ADR 0081) leaves absent what it cannot answer, so
// opening a Settings tab asks it nothing it refuses: no error the user did
// not cause (round 2 blocker 2: Backups listed journal renames on mount).
it("opens every Settings tab in the browser preview without an error", async () => {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    openSettings("appearance");
    for (let i = 0; i < 10; i++) await tick();
    // The tabs the preview offers (Sharing is gated on its account check).
    const tabs = [...root.querySelectorAll<HTMLElement>(".settings-nav-item")].map((item) => item.dataset.settingsTab as SettingsTabId);
    expect(tabs).toContain("backups");
    for (const tab of tabs) {
      openSettings(tab);
      for (let i = 0; i < 10; i++) await tick();
      expect(toasts().filter((toast) => toast.kind === "error").map((toast) => [tab, toast.message])).toEqual([]);
    }
  } finally { dispose(); }
});
