import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const root = path.resolve(import.meta.dirname, "..");
const config = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8"));
const capability = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/capabilities/default.json"), "utf8"));
const main = fs.readFileSync(path.join(root, "src/main.tsx"), "utf8");
const native = fs.readFileSync(path.join(root, "src-tauri/src/lib.rs"), "utf8");

describe("stable desktop startup reveal (GH #132)", () => {
  it("starts the main window hidden and reveals it after a stable themed frame", () => {
    expect(config.app.windows.find((window: { label: string }) => window.label === "main")?.visible).toBe(false);
    expect(capability.permissions).toContain("core:window:allow-show");
    expect(main).toContain("revealMainWindowAfterStableFrame");
    expect(main).toContain("queueMicrotask");
    expect(main.indexOf("applyTheme();")).toBeLessThan(main.indexOf("revealMainWindowAfterStableFrame"));
  });

  it("has a bounded native fallback so frontend failure cannot leave an invisible app", () => {
    expect(native).toContain("MAIN_WINDOW_REVEAL_FALLBACK_MS");
    expect(native).toContain('get_webview_window("main")');
    expect(native).toContain("window.show()");
  });
});
