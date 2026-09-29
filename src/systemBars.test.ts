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
  // GH #205 (master 945337d03): WebView reports env(safe-area-inset-*) as zero
  // on API 35, so the Activity content root is the one inset owner. IME stays
  // unconsumed and out of this padding (lane 17a owns the IME viewport).
  it("bounds the WebView by the native system-bar and cutout insets", () => {
    const root = path.resolve(import.meta.dirname, "..");
    const activity = fs.readFileSync(path.join(root,
      "src-tauri/gen/android/app/src/main/java/page/tine/app/MainActivity.kt"), "utf8");
    expect(activity).toContain("ViewCompat.setOnApplyWindowInsetsListener(content)");
    expect(activity).toContain("WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()");
    expect(activity).toContain("view.setPadding(safe.left, safe.top, safe.right, safe.bottom)");
    expect(activity).toContain("ViewCompat.requestApplyInsets(content)");
    expect(activity.indexOf("setOnApplyWindowInsetsListener")).toBeGreaterThan(activity.indexOf("super.onCreate"));
  });

  // GH #467. Once the insets pad the content root, the WINDOW paints the strip
  // behind the status bar while the bar ICONS follow Tine's theme. The strip
  // colour is set from the same `dark` flag, through resources that carry no
  // values-night variant so the resolver cannot reintroduce the device setting.
  it("paints the system-bar strip from Tine's theme, not the device night setting", () => {
    const root = path.resolve(import.meta.dirname, "..");
    const res = path.join(root, "src-tauri/gen/android/app/src/main/res");
    const lightColors = fs.readFileSync(path.join(res, "values/colors.xml"), "utf8");
    const nightDir = path.join(res, "values-night");
    const nightXml = fs.readdirSync(nightDir).map((f) => fs.readFileSync(path.join(nightDir, f), "utf8")).join("\n");
    const plugin = fs.readFileSync(path.join(root,
      "src-tauri/gen/android/app/src/main/java/page/tine/app/SystemBarsPlugin.kt"), "utf8");

    expect(lightColors).toContain('<color name="tine_system_bar_light">#FFFFFFFF</color>');
    expect(lightColors).toContain('<color name="tine_system_bar_dark">#FF1A1B1E</color>');
    expect(nightXml).not.toContain("tine_system_bar_light");
    expect(nightXml).not.toContain("tine_system_bar_dark");
    expect(plugin).toContain("if (dark) R.color.tine_system_bar_dark else R.color.tine_system_bar_light");
    expect(plugin).toContain("activity.window.setBackgroundDrawable(");
  });
});
