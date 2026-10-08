import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// GH #654: the image copy button said "Couldn't copy the image" on Android
// because `tauri-plugin-clipboard-manager` 2.x implements mobile `write_image`
// as `Err("Unsupported on this platform")`. Android publishes the image through
// its own plugin (staged PNG + FileProvider content URI). The Android runtime
// cannot run in this suite, so these pins hold the three seams together; the
// staging itself is tested in src-tauri/src/android_clipboard.rs.
const read = (path: string) => readFileSync(path, "utf8");
const KOTLIN = "src-tauri/gen/android/app/src/main/java/page/tine/app/ClipboardImagePlugin.kt";

describe("Android image copy (GH #654)", () => {
  it("never reaches the clipboard plugin's unsupported mobile write_image", () => {
    const platform = read("src-tauri/src/platform.rs");
    const body = platform.slice(platform.indexOf("pub(crate) async fn copy_image_to_clipboard"));
    const command = body.slice(0, body.indexOf("\n}\n"));
    const android = command.indexOf('#[cfg(target_os = "android")]');
    const route = command.indexOf("crate::android_clipboard::copy_png(&app, &bytes)");
    const others = command.indexOf('#[cfg(not(target_os = "android"))]');
    const pluginWrite = command.indexOf(".write_image(");
    expect(android).toBeGreaterThan(-1);
    expect(route).toBeGreaterThan(android);
    expect(others).toBeGreaterThan(route);
    // The plugin write is compiled only for the non-Android arm.
    expect(pluginWrite).toBeGreaterThan(others);
  });

  it("registers the Kotlin plugin the bridge calls, on Android only", () => {
    const lib = read("src-tauri/src/lib.rs");
    expect(lib).toMatch(/mod android_clipboard;/);
    expect(lib).toMatch(
      /#\[cfg\(target_os = "android"\)\]\s*let builder = builder\.plugin\(android_clipboard::init\(\)\);/
    );
    const bridge = read("src-tauri/src/android_clipboard.rs");
    expect(bridge).toContain('register_android_plugin(PLUGIN_IDENTIFIER, "ClipboardImagePlugin")');
    expect(bridge).toContain('"copyImage"');
    expect(bridge).toContain("stage_clipboard_png(&cache, png)");
  });

  it("publishes only a staged cache PNG, as a FileProvider URI clip", () => {
    const kotlin = read(KOTLIN);
    expect(kotlin).toMatch(/class ClipboardImagePlugin\(private val activity: Activity\) : Plugin\(activity\)/);
    expect(kotlin).toMatch(/@Command\s+fun copyImage\(invoke: Invoke\)/);
    expect(kotlin).toContain("activity.cacheDir.canonicalFile");
    expect(kotlin).toContain('startsWith("tine_clip_")');
    expect(kotlin).toContain('"${activity.packageName}.fileprovider"');
    expect(kotlin).toMatch(/setPrimaryClip\(ClipData\.newUri\(/);
    // Same prefix on both sides of the bridge.
    expect(read("src-tauri/src/android_clipboard.rs")).toContain('STAGED_PREFIX: &str = "tine_clip_"');
    // The manifest's FileProvider must still cover the app cache directory.
    const paths = read("src-tauri/gen/android/app/src/main/res/xml/file_paths.xml");
    expect(paths).toMatch(/<cache-path[^>]*path="\."/);
  });
});
