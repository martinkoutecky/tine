import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("Android voice-recording bounds", () => {
  it("caps duration and bytes before materializing the native temp file", () => {
    const source = readFileSync(
      "src-tauri/gen/android/app/src/main/java/page/tine/app/MediaCapturePlugin.kt",
      "utf8"
    );
    expect(source).toMatch(/setMaxDuration\(MAX_RECORDING_DURATION_MS\)/);
    expect(source).toMatch(/setMaxFileSize\(MAX_RECORDING_BYTES\)/);
    const sizeGuard = source.indexOf("out.length() > MAX_RECORDING_BYTES");
    const wholeRead = source.indexOf("out.readBytes()", sizeGuard);
    expect(sizeGuard).toBeGreaterThan(-1);
    expect(wholeRead).toBeGreaterThan(sizeGuard);
    expect(source).toMatch(/MEDIA_RECORDER_INFO_MAX_DURATION_REACHED/);
    expect(source).toMatch(/MEDIA_RECORDER_INFO_MAX_FILESIZE_REACHED/);
  });
});
