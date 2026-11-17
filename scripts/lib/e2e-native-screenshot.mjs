import { spawnSync } from "node:child_process";
import fs from "node:fs";

/** Capture the isolated X11 surface. WebKitGTK's screenshot endpoint can hang
 * on live editors or paused drawer animations. One native capture, no WebDriver
 * request; throws if the image cannot be produced within five seconds. */
export function nativeScreenshot(file) {
  const result = spawnSync("import", ["-window", "root", file], {
    env: process.env, encoding: "utf8", timeout: 5_000,
  });
  if (result.status !== 0 || !fs.existsSync(file) || fs.statSync(file).size === 0) {
    throw new Error(`native X11 screenshot failed: ${JSON.stringify({ file, status: result.status, error: String(result.error || result.stderr) })}`);
  }
}
