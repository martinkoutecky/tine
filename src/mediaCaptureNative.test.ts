import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("Android voice-recording bounds", () => {
  it("caps duration and bytes, then hands the native temp to Rust without base64", () => {
    const source = readFileSync(
      "src-tauri/gen/android/app/src/main/java/page/tine/app/MediaCapturePlugin.kt",
      "utf8"
    );
    expect(source).toMatch(/setMaxDuration\(MAX_RECORDING_DURATION_MS\)/);
    expect(source).toMatch(/setMaxFileSize\(MAX_RECORDING_BYTES\)/);
    const sizeGuard = source.indexOf("out.length() > MAX_RECORDING_BYTES");
    const stop = source.slice(source.indexOf("fun stopRecording"), source.indexOf("fun cancelRecording"));
    expect(sizeGuard).toBeGreaterThan(-1);
    expect(stop).toContain('ret.put("path", out.absolutePath)');
    expect(stop).not.toContain("readBytes()");
    expect(stop).not.toContain("Base64.encodeToString");
    expect(source).toMatch(/MEDIA_RECORDER_INFO_MAX_DURATION_REACHED/);
    expect(source).toMatch(/MEDIA_RECORDER_INFO_MAX_FILESIZE_REACHED/);
    expect(source.indexOf("recorder = rec")).toBeLessThan(source.indexOf("rec.prepare()"));
    expect(source).toMatch(/listFiles\(\)[\s\S]*startsWith\("tine_memo_"\)[\s\S]*forEach \{ it\.delete\(\) \}/);

    const commands = readFileSync("src-tauri/src/commands.rs", "utf8");
    expect(commands).toMatch(/pub\(crate\) fn import_native_capture/);
    expect(commands).toMatch(/import_asset_file\(&mut capture, &name, max_bytes\)/);
    const bridge = readFileSync("src-tauri/src/android_media.rs", "utf8");
    const result = bridge.slice(
      bridge.indexOf("struct MediaCaptureResult"),
      bridge.indexOf("#[cfg(target_os = \"android\")]", bridge.indexOf("struct MediaCaptureResult"))
    );
    expect(result).toMatch(/path:\s*Option<String>/);
    const block = readFileSync("src/components/Block.tsx", "utf8");
    expect(block).toMatch(/backend\(\)\.importNativeCapture\(res\.path, candidate\)/);
  });

  it("streams captured and picked photos through a bounded native cache token", () => {
    const source = readFileSync(
      "src-tauri/gen/android/app/src/main/java/page/tine/app/MediaCapturePlugin.kt",
      "utf8"
    );
    const photo = source.slice(source.indexOf("fun photoResult"), source.indexOf("fun startRecording"));
    expect(source).toMatch(/MAX_PHOTO_BYTES/);
    expect(source).toMatch(/MAX_PHOTO_PIXELS/);
    expect(source).toMatch(/inJustDecodeBounds\s*=\s*true/);
    expect(photo).toContain('ret.put("path"');
    expect(photo).not.toContain("readBytes()");
    expect(photo).not.toContain("Base64.encodeToString");
    expect(source).toMatch(/copyPickedPhoto[\s\S]*MAX_PHOTO_BYTES/);

    const commands = readFileSync("src-tauri/src/commands.rs", "utf8");
    expect(commands).toMatch(/tine_photo_/);
    expect(commands).toMatch(/MAX_PHOTO_BYTES/);
    const block = readFileSync("src/components/Block.tsx", "utf8");
    expect(block).toMatch(/capturePhoto[\s\S]*importNativeCapture\(res\.path, candidate\)/);
  });
});
