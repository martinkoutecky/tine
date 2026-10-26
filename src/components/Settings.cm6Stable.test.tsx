import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
vi.mock("../appIdentity", () => ({ APP_UPDATE_CHANNEL: "stable", APP_PRODUCT_NAME: "Tine" }));
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";
import { SETTING_SEARCH } from "./settingsSearch";
import { cm6EditorAvailable } from "../editor/cm6Setting";
import { render as message } from "../i18n";

afterEach(() => { closeSettings(); document.body.innerHTML = ""; });
it("stable hides the CodeMirror toggle and its Settings search entry", async () => {
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    openSettings("editor");
    await new Promise(resolve => setTimeout(resolve, 0));
    root.querySelector<HTMLButtonElement>(".settings-advanced-toggle")!.click();
    expect(cm6EditorAvailable).toBe(false);
    expect(root.textContent).not.toContain("CodeMirror");
    expect(SETTING_SEARCH.some(entry => (typeof entry.label === "string" ? entry.label : message(entry.label)).includes("CodeMirror"))).toBe(false);
  } finally { dispose(); }
});
