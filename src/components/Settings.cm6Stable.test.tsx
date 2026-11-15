import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
vi.mock("../appIdentity", () => ({ APP_UPDATE_CHANNEL: "stable", APP_PRODUCT_NAME: "Tine" }));
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";
import { settingEntries } from "./settingsRegistry";
import { render as message } from "../i18n";

afterEach(() => { closeSettings(); document.body.innerHTML = ""; });
it("stable exposes the classic-editor alternative and its Settings search entry", async () => {
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    openSettings("editor");
    await new Promise(resolve => setTimeout(resolve, 0));
    root.querySelector<HTMLButtonElement>(".settings-advanced-toggle")!.click();
    expect(root.querySelector('[data-setting-id="settings-cm6-label"]')?.textContent).toContain("Use classic editor");
    expect(settingEntries().some(entry => (typeof entry.label === "string" ? entry.label : message(entry.label)).includes("Use classic editor"))).toBe(true);
  } finally { dispose(); }
});
