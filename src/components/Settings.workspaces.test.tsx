import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { App } from "../App";
import { Settings } from "./Settings";
import * as ui from "../ui";
import * as ws from "../workspaces";
import { buildPersistedSession } from "../session";
import { mainPaneRouter } from "../router";
import { commandDefaults, paletteCommands, runGlobalCommand } from "../keybindings";
import { initNavSettings, setShowWorkspaces, showWorkspaces } from "../navSettings";

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); document.body.replaceChildren(); });

it.each([false, true])("GH #927: Settings hides Workspaces and restores their state (drawer=%s)", async (mobile) => {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: mobile && query === "(max-width: 639px)", addEventListener() {}, removeEventListener() {},
  }));
  vi.spyOn(backend(), "loadWorkspaces").mockResolvedValue(JSON.stringify({ version: 1, activeId: "default",
    workspaces: [{ id: "default", name: "Research", blob: buildPersistedSession() }] }));
  const saveWorkspaces = vi.spyOn(backend(), "saveWorkspaces").mockResolvedValue("durable");
  let shown = true;
  const readPreference = vi.spyOn(backend(), "getAppBool").mockImplementation(async (key, fallback) => key === "show_workspaces" ? shown : fallback);
  const savePreference = vi.spyOn(backend(), "setAppBool").mockImplementation(async (key, value) => { if (key === "show_workspaces") shown = value; });
  await ws.initializeWorkspaces();
  mainPaneRouter.openPage("Research", "page", { inPlace: true });
  ui.setLeftSidebarOpen(true);
  const host = document.body.appendChild(document.createElement("div"));
  const settingsHost = document.body.appendChild(document.createElement("div"));
  const dispose = render(() => <App />, host);
  const disposeSettings = render(() => <Settings />, settingsHost);
  ui.openSettings("appearance");
  try {
    const search = settingsHost.querySelector<HTMLInputElement>(".settings-search-input")!;
    search.value = "workspaces";
    search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    const result = await vi.waitFor(() => {
      const found = settingsHost.querySelector<HTMLButtonElement>('[data-setting-target="settings-show-workspaces"]');
      expect(found).not.toBeNull(); return found!;
    });
    result.click();
    const field = await vi.waitFor(() => {
      const found = settingsHost.querySelector('[data-setting-id="settings-show-workspaces"]');
      expect(found).not.toBeNull(); return found!;
    });
    const toggle = field.querySelector<HTMLButtonElement>('[role="switch"]')!;
    expect(field.closest(".settings-advanced")?.querySelector(".settings-advanced-toggle")?.getAttribute("aria-expanded")).toBe("true");
    await vi.waitFor(() => expect(readPreference).toHaveBeenCalledWith("show_workspaces", true));
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(host.querySelector("[data-workspace-switcher]")).not.toBeNull();
    const before = JSON.stringify(ws.workspaces());
    const active = ws.activeWorkspaceId();
    saveWorkspaces.mockClear();
    toggle.click();
    expect(showWorkspaces()).toBe(false);
    await vi.waitFor(() => {
      expect(showWorkspaces()).toBe(false);
      expect(host.querySelector("[data-workspace-switcher]")).toBeNull();
      expect(host.querySelector("[data-workspace-switcher-sidebar]")).toBeNull();
    });
    ui.setLeftSidebarOpen(false);
    expect(showWorkspaces()).toBe(false);
    expect(host.querySelector("[data-workspace-switcher]")).toBeNull();
    expect(paletteCommands().some(command => command.id === "workspace/new")).toBe(false);
    expect(commandDefaults().some(command => command.id === "workspace/new")).toBe(false);
    expect(runGlobalCommand("workspace/new")).toBe(false);
    await vi.waitFor(() => expect(savePreference).toHaveBeenCalledWith("show_workspaces", false));
    expect(saveWorkspaces).not.toHaveBeenCalled();
    toggle.click();
    await vi.waitFor(() => expect(host.querySelector('[data-workspace-switcher-compact="true"]')).not.toBeNull());
    expect(paletteCommands().some(command => command.id === "workspace/new")).toBe(true);
    expect(commandDefaults().some(command => command.id === "workspace/new")).toBe(true);
    expect(JSON.stringify(ws.workspaces())).toBe(before);
    expect(ws.activeWorkspaceId()).toBe(active);
    expect(saveWorkspaces).not.toHaveBeenCalled();
    await vi.waitFor(() => expect(savePreference.mock.calls.filter(([key]) => key === "show_workspaces")).toEqual([
      ["show_workspaces", false], ["show_workspaces", true],
    ]));
  } finally { ui.closeSettings(); disposeSettings(); dispose(); ws.resetWorkspacesForTest(); }
});

it("GH #927: restores the device preference and rolls back a failed visibility write", async () => {
  const read = vi.spyOn(backend(), "getAppBool").mockImplementation(async (key, fallback) => key === "show_workspaces" ? false : fallback);
  await initNavSettings();
  await vi.waitFor(() => expect(showWorkspaces()).toBe(false));
  expect(read.mock.calls.filter(([key]) => key === "show_workspaces")).toEqual([["show_workspaces", true]]);
  const write = vi.spyOn(backend(), "setAppBool").mockRejectedValue(new Error("disk unavailable"));
  setShowWorkspaces(true);
  await vi.waitFor(() => expect(showWorkspaces()).toBe(false));
  expect(write).toHaveBeenCalledExactlyOnceWith("show_workspaces", true);
  write.mockResolvedValue();
  setShowWorkspaces(true);
  await vi.waitFor(() => expect(write).toHaveBeenCalledTimes(2));
});
