import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { initSettingsLayout, setSettingsMaximized, settingsMaximized } from "./settingsLayout";

afterEach(() => { vi.restoreAllMocks(); setSettingsMaximized(false); });

it("hydrates a remembered Settings size and saves later changes", async () => {
  const read = vi.spyOn(backend(), "getAppBool").mockResolvedValue(true);
  const write = vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  await initSettingsLayout();
  expect(read).toHaveBeenCalledWith("settings_dialog_maximized", false);
  expect(settingsMaximized()).toBe(true);
  setSettingsMaximized(false);
  expect(write).toHaveBeenCalledWith("settings_dialog_maximized", false);
});
