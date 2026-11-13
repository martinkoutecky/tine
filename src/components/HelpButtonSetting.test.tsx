// GH #926 (D-20261111-QP2): the corner "?" can be hidden per device, and help
// stays reachable from the palette command and Settings → Help & diagnostics.
import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { Settings } from "./Settings";
import { HelpPopup } from "./HelpShortcuts";
import * as ui from "../ui";
import * as guide from "../guide";
import { runGlobalCommand } from "../keybindings";
import { initNavSettings, setShowHelpButton, showHelpButton } from "../navSettings";

afterEach(() => { vi.restoreAllMocks(); ui.closeHelpPopup(); ui.closeSettings(); document.body.replaceChildren(); });

it("GH #926: Settings → Appearance → Advanced hides the help button; the palette still opens help", async () => {
  let shown = true;
  vi.spyOn(backend(), "getAppBool").mockImplementation(async (key, fallback) => key === "show_help_button" ? shown : fallback);
  const save = vi.spyOn(backend(), "setAppBool").mockImplementation(async (key, value) => { if (key === "show_help_button") shown = value; });
  await initNavSettings();
  const helpHost = document.body.appendChild(document.createElement("div"));
  const settingsHost = document.body.appendChild(document.createElement("div"));
  const disposeHelp = render(() => <HelpPopup />, helpHost);
  const disposeSettings = render(() => <Settings />, settingsHost);
  ui.openSettings("appearance");
  try {
    expect(helpHost.querySelector(".help-corner-btn")).not.toBeNull();
    const search = await vi.waitFor(() => { const s = settingsHost.querySelector<HTMLInputElement>(".settings-search-input"); expect(s).not.toBeNull(); return s!; });
    search.value = "help button";
    search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    const result = await vi.waitFor(() => {
      const found = settingsHost.querySelector<HTMLButtonElement>('[data-setting-target="settings-show-help-button"]');
      expect(found).not.toBeNull(); return found!;
    });
    result.click();
    const toggle = await vi.waitFor(() => {
      const found = settingsHost.querySelector('[data-setting-id="settings-show-help-button"] [role="switch"]');
      expect(found).not.toBeNull(); return found as HTMLButtonElement;
    });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    toggle.click();
    expect(showHelpButton()).toBe(false);
    expect(helpHost.querySelector(".help-corner-btn")).toBeNull();
    await vi.waitFor(() => expect(save).toHaveBeenCalledWith("show_help_button", false));
    ui.closeSettings();
    expect(runGlobalCommand("ui/toggle-help")).toBe(true);
    await vi.waitFor(() => expect(helpHost.querySelectorAll(".help-menu-item").length).toBeGreaterThan(0));
  } finally { setShowHelpButton(true); disposeSettings(); disposeHelp(); }
});

it("GH #926: Settings → Help & diagnostics lists the help menu's entries", async () => {
  const openGuide = vi.spyOn(guide, "openGuide").mockResolvedValue(undefined as never);
  const settingsHost = document.body.appendChild(document.createElement("div"));
  const dispose = render(() => <Settings />, settingsHost);
  ui.openSettings("diagnostics");
  try {
    const links = await vi.waitFor(() => { const d = settingsHost.querySelector<HTMLDetailsElement>("details.help-links"); expect(d).not.toBeNull(); return d!; });
    links.open = true;
    links.dispatchEvent(new Event("toggle"));
    const rows = await vi.waitFor(() => {
      const found = settingsHost.querySelectorAll<HTMLButtonElement>(".help-links button");
      expect(found.length).toBe(6); return found;
    });
    rows[0].click();
    expect(openGuide).toHaveBeenCalledOnce();
    expect(ui.settingsOpen()).toBe(false);
  } finally { dispose(); }
});

it("GH #926: the help-button preference survives a restart and defaults to shown", async () => {
  const read = vi.spyOn(backend(), "getAppBool").mockImplementation(async (key, fallback) => key === "show_help_button" ? false : fallback);
  await initNavSettings();
  await vi.waitFor(() => expect(showHelpButton()).toBe(false));
  expect(read.mock.calls.filter(([key]) => key === "show_help_button")).toEqual([["show_help_button", true]]);
  vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  setShowHelpButton(true);
});
