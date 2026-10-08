import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// Android and iOS start the app through the symbol tauri::mobile_entry_point
// generates. On any other function the release APK fails Tauri's runtime-symbol
// validation (og Beta release run 36794348930), and no local desktop gate sees it.
describe("mobile entry point", () => {
  it("marks run(), the one function every platform starts through", () => {
    const lib = readFileSync("src-tauri/src/lib.rs", "utf8");
    const marks = lib.match(/#\[cfg_attr\(mobile, tauri::mobile_entry_point\)\]\n([^\n]*)/g) ?? [];
    expect(marks, "exactly one mobile entry attribute, directly on `pub fn run()`; exemplar: master src-tauri/src/lib.rs").toEqual([
      "#[cfg_attr(mobile, tauri::mobile_entry_point)]\npub fn run() {",
    ]);
  });
});
