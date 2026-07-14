// @vitest-environment jsdom
import fs from "node:fs";
import path from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";

const native = vi.hoisted(() => ({
  setSystemBarAppearance: vi.fn(async (_dark: boolean) => {}),
}));

vi.mock("./backend", () => ({
  backend: () => native,
  isTauri: () => true,
}));

describe("Android system-bar theme synchronization", () => {
  beforeEach(() => {
    native.setSystemBarAppearance.mockClear();
    localStorage.clear();
    document.documentElement.removeAttribute("data-theme");
    vi.resetModules();
  });

  it("sends the resolved startup and toggled appearance to the native host", async () => {
    localStorage.setItem("logseq-claude.theme", "light");
    const ui = await import("./ui");

    ui.applyTheme();
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
    expect(native.setSystemBarAppearance).toHaveBeenLastCalledWith(false);

    ui.toggleTheme();
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    expect(native.setSystemBarAppearance).toHaveBeenLastCalledWith(true);

    ui.toggleTheme();
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
    expect(native.setSystemBarAppearance).toHaveBeenLastCalledWith(false);
  });

  it("restores the persisted appearance before frontend sync and on resume", () => {
    const root = path.resolve(import.meta.dirname, "..");
    const plugin = fs.readFileSync(path.join(root,
      "src-tauri/gen/android/app/src/main/java/page/tine/app/SystemBarsPlugin.kt"), "utf8");
    const activity = fs.readFileSync(path.join(root,
      "src-tauri/gen/android/app/src/main/java/page/tine/app/MainActivity.kt"), "utf8");

    expect(plugin).toContain("isAppearanceLightStatusBars = !dark");
    expect(plugin).toContain("isAppearanceLightNavigationBars = !dark");
    expect(plugin).toContain("getSharedPreferences");
    expect(activity.match(/SystemBarAppearance\.restore\(this\)/g)).toHaveLength(2);
    expect(activity.indexOf("SystemBarAppearance.restore(this)")).toBeGreaterThan(activity.indexOf("super.onCreate"));
  });
});
