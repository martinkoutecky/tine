// @vitest-environment jsdom
import fs from "node:fs";
import path from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";

const native = vi.hoisted(() => ({
  setSystemBarAppearance: vi.fn(async (_dark: boolean, _background?: string) => {}),
  appPlatform: vi.fn(async () => "desktop"),
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

  it("extracts the painted background and follows live stylesheet and mode changes", async () => {
    const style = document.createElement("style");
    style.textContent = "body { background-color: rgb(0, 43, 54); }";
    document.head.append(style);
    const ui = await import("./ui");
    const { watchSystemBarBackground, syncSystemBarAppearance } = await import("./systemBars");
    const refresh = () => syncSystemBarAppearance(
      document.documentElement.getAttribute("data-theme") === "dark", () => {});
    const stop = watchSystemBarBackground(refresh);
    ui.applyTheme();
    await vi.waitFor(() => expect(native.setSystemBarAppearance).toHaveBeenLastCalledWith(false, "#002b36"));
    style.textContent = "body { background-color: rgb(250, 240, 220); }";
    await vi.waitFor(() => expect(native.setSystemBarAppearance).toHaveBeenLastCalledWith(false, "#faf0dc"));
    ui.toggleTheme();
    await vi.waitFor(() => expect(native.setSystemBarAppearance).toHaveBeenLastCalledWith(true, "#faf0dc"));
    document.body.style.backgroundColor = "rgb(30, 60, 50)";
    await vi.waitFor(() => expect(native.setSystemBarAppearance).toHaveBeenLastCalledWith(true, "#1e3c32"));
    document.body.style.backgroundColor = "transparent";
    await vi.waitFor(() => expect(native.setSystemBarAppearance).toHaveBeenLastCalledWith(true, undefined));
    document.body.style.removeProperty("background-color");
    style.remove();
    stop();
  });

  it("accepts opaque RGB and rejects transparent or invalid backgrounds", async () => {
    const { systemBarBackground } = await import("./systemBars");
    expect(systemBarBackground("rgb(0, 43, 54)")).toBe("#002b36");
    expect(systemBarBackground("rgba(250, 240, 220, 1)")).toBe("#faf0dc");
    for (const value of ["transparent", "rgba(0, 0, 0, 0)", "rgba(0, 0, 0, 0.5)", "rgb(999, 0, 0)", "nonsense", ""]) {
      expect(systemBarBackground(value)).toBeNull();
    }
  });

  it("keeps native appearance Android-only and leaves every other shipped target a no-op", () => {
    const source = fs.readFileSync(path.resolve(import.meta.dirname,
      "../src-tauri/src/android_system_bars.rs"), "utf8");
    expect(source).toContain('#[cfg(target_os = "android")]');
    // The complement includes Linux, Windows, macOS and iOS without an
    // enumerated list that can accidentally omit one shipped platform.
    expect(source).toMatch(/#\[cfg\(not\(target_os = "android"\)\)\][\s\S]*?let _ = \(dark, background\);\s*Ok\(\(\)\)/);
  });

  it("sends the resolved startup and toggled appearance to the native host", async () => {
    localStorage.setItem("logseq-claude.theme", "light");
    const ui = await import("./ui");

    ui.applyTheme();
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
    expect(native.setSystemBarAppearance).toHaveBeenLastCalledWith(false, undefined);

    ui.toggleTheme();
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    expect(native.setSystemBarAppearance).toHaveBeenLastCalledWith(true, undefined);

    ui.toggleTheme();
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
    expect(native.setSystemBarAppearance).toHaveBeenLastCalledWith(false, undefined);
  });

  it("restores the persisted appearance before frontend sync and on resume", () => {
    const root = path.resolve(import.meta.dirname, "..");
    const plugin = fs.readFileSync(path.join(root,
      "src-tauri/gen/android/app/src/main/java/page/tine/app/SystemBarsPlugin.kt"), "utf8");
    const activity = fs.readFileSync(path.join(root,
      "src-tauri/gen/android/app/src/main/java/page/tine/app/MainActivity.kt"), "utf8");

    expect(plugin).toContain("isAppearanceLightStatusBars = !lightIcons");
    expect(plugin).toContain("isAppearanceLightNavigationBars = !lightIcons");
    expect(plugin).toContain("getSharedPreferences");
    expect(activity.match(/SystemBarAppearance\.restore\(this\)/g)).toHaveLength(2);
    expect(activity.indexOf("SystemBarAppearance.restore(this)")).toBeGreaterThan(activity.indexOf("super.onCreate"));
  });
  // GH #205 (master 945337d03): WebView reports env(safe-area-inset-*) as zero
  // on API 35, so the Activity content root is the one inset owner. The IME is
  // part of that native viewport (master d5412929a): edge-to-edge WebView can
  // leave visualViewport unchanged under keyboard occlusion, so the bottom pad is
  // the larger of the navigation bar and the keyboard. The insets themselves
  // stay unconsumed so descendants still observe IME visibility.
  it("bounds the WebView by the native system-bar and cutout insets", () => {
    const root = path.resolve(import.meta.dirname, "..");
    const activity = fs.readFileSync(path.join(root,
      "src-tauri/gen/android/app/src/main/java/page/tine/app/MainActivity.kt"), "utf8");
    expect(activity).toContain("ViewCompat.setOnApplyWindowInsetsListener(content)");
    expect(activity).toContain("WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()");
    expect(activity).toContain("val ime = insets.getInsets(WindowInsetsCompat.Type.ime())");
    expect(activity).toContain("view.setPadding(safe.left, safe.top, safe.right, maxOf(safe.bottom, ime.bottom))");
    // Not consumed: the listener hands the original insets back to descendants.
    expect(activity).toMatch(/maxOf\(safe\.bottom, ime\.bottom\)\)\s*\n\s*insets\s*\n\s*\}/);
    expect(activity).toContain("ViewCompat.requestApplyInsets(content)");
    expect(activity.indexOf("setOnApplyWindowInsetsListener")).toBeGreaterThan(activity.indexOf("super.onCreate"));
  });

  // GH #716. The padded WebView sees the IME only as a smaller window, so the
  // keyboard dock needs MainActivity's own IME signal to tell it from a resize.
  it("reports soft-keyboard visibility to the page under the event the dock listens for", async () => {
    const root = path.resolve(import.meta.dirname, "..");
    const activity = fs.readFileSync(path.join(root,
      "src-tauri/gen/android/app/src/main/java/page/tine/app/MainActivity.kt"), "utf8");
    const { NATIVE_IME_EVENT } = await import("./components/MobileKeyboardToolbar");
    expect(activity).toContain("publishIme(insets.isVisible(WindowInsetsCompat.Type.ime()))");
    expect(activity).toContain(`window.dispatchEvent(new CustomEvent('${NATIVE_IME_EVENT}',{detail:$visible}))`);
    expect(activity).toContain("override fun onWebViewCreate(webView: WebView)");
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
