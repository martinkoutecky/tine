// GH #299 / GH #401 (master 51185bbe3 + fcb812397): Tine paints a themed
// readiness surface before any application module loads, on desktop and on
// the Android window behind the WebView. Kept out of startupReveal.test.ts
// (another lane's write set in og batch 17/18).
import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const root = path.resolve(import.meta.dirname, "..");
const read = (file: string) => fs.readFileSync(path.join(root, file), "utf8");
const index = read("index.html");
const main = read("src/main.tsx");
const startupCss = read("src/styles/startup.css");
const startupTheme = read("public/assets/startup-theme.js");

describe("themed readiness before the app mounts (GH #299, GH #401)", () => {
  it("paints a themed readiness shell before the application modules load", () => {
    const shell = index.indexOf('class="startup-shell"');
    const module = index.indexOf('type="module"');

    // Linked, not inlined (I-22: no inline <style>; script-src is 'self').
    expect(index).toContain('<link rel="stylesheet" href="/src/styles/startup.css" />');
    // Under assets/ so every app-bundle consumer (live export, CLI, demo) ships it.
    expect(index).toContain('<script src="/assets/startup-theme.js"></script>');
    expect(index.indexOf("/assets/startup-theme.js")).toBeLessThan(module);
    expect(startupCss).toContain(".startup-shell {");
    expect(startupTheme).toContain('localStorage.getItem("logseq-claude.theme")');
    expect(startupTheme).toContain('matchMedia("(prefers-color-scheme: dark)")');
    expect(index).toContain('role="status"');
    expect(shell).toBeGreaterThanOrEqual(0);
    expect(module).toBeGreaterThan(shell);
    expect(main).toContain("root.replaceChildren();");
    expect(main.indexOf("root.replaceChildren();")).toBeLessThan(main.indexOf("render(() => <App />"));
  });

  it("lets the built-in palette tokens win over the pre-CSS fallbacks (GH #401)", () => {
    expect(startupCss).toContain("background: var(--bg-primary, #ffffff);");
    expect(startupCss).toContain("background: var(--bg-primary, #1a1b1e);");
    expect(startupCss).toContain("color: var(--text-primary, #c6c8cc);");
    // The fallbacks are og's own palette values from theme.css.
    const theme = read("src/styles/theme.css");
    for (const value of ["#ffffff", "#433f38", "#1a1b1e", "#c6c8cc"]) expect(theme).toContain(value);
  });

  it("uses a light or night Tine backing color before the WebView paints", () => {
    const res = "src-tauri/gen/android/app/src/main/res";
    const item = '<item name="android:windowBackground">@color/tine_window_background</item>';
    expect(read(`${res}/values/themes.xml`)).toContain(item);
    expect(read(`${res}/values-night/themes.xml`)).toContain(item);
    expect(read(`${res}/values/colors.xml`)).toContain('<color name="tine_window_background">#FFFFFFFF</color>');
    expect(read(`${res}/values-night/colors.xml`)).toContain('<color name="tine_window_background">#FF1A1B1E</color>');
  });
});
