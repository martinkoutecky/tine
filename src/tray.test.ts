import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import {
  KEY_TRAY_MINIMIZE,
  KEY_TRAY_SHOW,
  KEY_TRAY_START_MINIMIZED,
  trayNote,
} from "./tray";

const root = path.resolve(import.meta.dirname, "..");
const rust = fs.readFileSync(path.join(root, "src-tauri/src/tray.rs"), "utf8");

describe("system tray settings (GH #625)", () => {
  it("the note appears only for a requested icon the desktop could not show", () => {
    const problem = { supported: true, active: false, problem: "No system tray was found." };
    expect(trayNote(true, problem)).toBe("No system tray was found.");
    expect(trayNote(false, problem)).toBeNull();
    expect(trayNote(true, { supported: true, active: true, problem: null })).toBeNull();
    expect(trayNote(true, { supported: false, active: false, problem: "x" })).toBeNull();
    expect(trayNote(true, null)).toBeNull();
  });

  it("the keys are the ones the native side reads", () => {
    for (const [name, key] of [
      ["SHOW_KEY", KEY_TRAY_SHOW],
      ["MINIMIZE_KEY", KEY_TRAY_MINIMIZE],
      ["START_MINIMIZED_KEY", KEY_TRAY_START_MINIMIZED],
    ]) {
      expect(rust, `${name} must equal the frontend key`).toContain(`pub(crate) const ${name}: &str = "${key}";`);
    }
  });
});
