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

  // I-22 (og C3 L08): Tauri 2 panics on an error returned from `.setup`, so a
  // remembered graph that resolves but fails to open (synced dangling `assets`
  // link, unsafe synced `:pages-directory`, disk error) crashed every launch.
  // The webview opens the startup graph through `load_graph` and shows the
  // Welcome open-failure card instead. Exemplar: master lib.rs setup.
  it("never opens a graph or propagates an error out of Tauri setup (I-22)", () => {
    // Matched by shape, so a `move` closure cannot silently disable this guard.
    const setupStart = native.search(/\.setup\((move\s+)?\|app\|/);
    const setupEnd = native.indexOf(".invoke_handler", setupStart);
    expect(setupStart).toBeGreaterThanOrEqual(0);
    expect(setupEnd).toBeGreaterThan(setupStart);
    const setup = native.slice(setupStart, setupEnd).replace(/\/\/.*$/gm, "");
    for (const opener of ["load_graph_for_label", "open_graph_for_load", "Store::open"]) {
      expect(setup, `setup must defer ${opener} to the webview (I-22)`).not.toContain(opener);
    }
    expect(setup, "an error returned from setup panics Tauri at every launch (I-22)").not.toMatch(/\?\s*;/);
    expect(setup).toContain("defers graph open to the visible webview");
  });
});
