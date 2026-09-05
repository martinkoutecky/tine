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

  it("uses a light or night Tine backing color before the WebView paints", () => {
    const root = path.resolve(import.meta.dirname, "..");
    const values = path.join(root, "src-tauri/gen/android/app/src/main/res/values");
    const night = path.join(root, "src-tauri/gen/android/app/src/main/res/values-night");
    const lightTheme = fs.readFileSync(path.join(values, "themes.xml"), "utf8");
    const darkTheme = fs.readFileSync(path.join(night, "themes.xml"), "utf8");
    const lightColors = fs.readFileSync(path.join(values, "colors.xml"), "utf8");
    const darkColors = fs.readFileSync(path.join(night, "colors.xml"), "utf8");

    expect(lightTheme).toContain('<item name="android:windowBackground">@color/tine_window_background</item>');
    expect(darkTheme).toContain('<item name="android:windowBackground">@color/tine_window_background</item>');
    expect(lightColors).toContain('<color name="tine_window_background">#FFFFFFFF</color>');
    expect(darkColors).toContain('<color name="tine_window_background">#FF1A1B1E</color>');
  });

  // GH #467. The window background above is only the pre-restore initial paint,
  // and it is resolved by the ANDROID night setting. Once the system-bar insets
  // began padding the content root, that same window background became the strip
  // behind the status bar -- while the bar ICONS follow TINE's theme. When the
  // two disagreed the strip went white under white icons. The strip colour is
  // now set from the same `dark` flag, through resources that carry no
  // values-night variant so the resolver cannot reintroduce the device setting.
  it("paints the system-bar strip from Tine's theme, not the device night setting", () => {
    const root = path.resolve(import.meta.dirname, "..");
    const values = path.join(root, "src-tauri/gen/android/app/src/main/res/values");
    const night = path.join(root, "src-tauri/gen/android/app/src/main/res/values-night");
    const lightColors = fs.readFileSync(path.join(values, "colors.xml"), "utf8");
    const nightColors = fs.readFileSync(path.join(night, "colors.xml"), "utf8");
    const plugin = fs.readFileSync(path.join(root,
      "src-tauri/gen/android/app/src/main/java/page/tine/app/SystemBarsPlugin.kt"), "utf8");

    expect(lightColors).toContain('<color name="tine_system_bar_light">#FFFFFFFF</color>');
    expect(lightColors).toContain('<color name="tine_system_bar_dark">#FF1A1B1E</color>');
    // A values-night override would put the device setting back in charge.
    expect(nightColors).not.toContain("tine_system_bar_light");
    expect(nightColors).not.toContain("tine_system_bar_dark");
    // One authority: the same `dark` that chooses the icon appearance.
    expect(plugin).toContain("if (dark) R.color.tine_system_bar_dark else R.color.tine_system_bar_light");
    expect(plugin).toContain("activity.window.setBackgroundDrawable(");
  });
});
