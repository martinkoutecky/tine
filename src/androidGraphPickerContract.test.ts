import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("GH #630 sends an unresolved folder as a typed status through the existing Rust DTO", () => {
  const plugin = readFileSync(new URL("../src-tauri/gen/android/app/src/main/java/page/tine/app/GraphFolderPickerPlugin.kt", import.meta.url), "utf8");
  const unresolved = plugin.slice(plugin.indexOf("if (path.isNullOrEmpty())"), plugin.indexOf('ret.put("status", "picked")'));
  expect(unresolved).toContain('ret.put("status", "local-folder-required")');
  expect(unresolved).toContain("invoke.resolve(ret)");
  expect(unresolved).not.toContain("invoke.reject");
  const bridge = readFileSync(new URL("../src-tauri/src/android_folder_picker.rs", import.meta.url), "utf8");
  expect(bridge).toContain("status: String");
  expect(bridge).toContain("run_mobile_plugin");
});
