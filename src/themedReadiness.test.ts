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

describe("themed readiness before the app mounts (GH #299, GH #401)", () => {
  it("paints a themed readiness shell before the application modules load", () => {
    const shell = index.indexOf('class="startup-shell"');
    const module = index.indexOf('type="module"');

    expect(index).toContain('id="tine-startup-style"');
    expect(index).toContain('localStorage.getItem("logseq-claude.theme")');
    expect(index).toContain('matchMedia("(prefers-color-scheme: dark)")');
    expect(index).toContain('role="status"');
    expect(shell).toBeGreaterThanOrEqual(0);
    expect(module).toBeGreaterThan(shell);
    expect(main).toContain("root.replaceChildren();");
    expect(main.indexOf("root.replaceChildren();")).toBeLessThan(main.indexOf("render(() => <App />"));
  });

  it("lets the built-in palette tokens win over the pre-CSS fallbacks (GH #401)", () => {
    expect(index).toContain("background: var(--bg-primary, #ffffff);");
    expect(index).toContain("background: var(--bg-primary, #1a1b1e);");
    expect(index).toContain("color: var(--text-primary, #c6c8cc);");
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
