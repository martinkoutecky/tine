import fs from "node:fs";
import path from "node:path";
import { APP_ID } from "./app-identity.mjs";

/** Seed the runner's scratch device preference before native startup, preserving
 * explicit choices and other settings. One bounded JSON read, at most one write;
 * no graph IO. Errors abort launch rather than falling back to account storage. */
export function seedClassicEditorProfile(env, platform = process.platform) {
  if (env.E2E_CLASSIC_PROFILE !== "1") return;
  const data = platform === "win32" ? env.APPDATA : env.XDG_DATA_HOME;
  const directory = env.E2E_CLASSIC_SETTINGS_DIR || (data && path.join(data, APP_ID));
  if (!directory || !path.isAbsolute(directory)) throw new Error("Classic E2E profile requires an absolute scratch app-data directory");
  const file = path.join(directory, "tine-settings.json");
  let settings = {};
  try { settings = JSON.parse(fs.readFileSync(file, "utf8")); }
  catch (error) { if (error.code !== "ENOENT") throw error; }
  if (Object.hasOwn(settings, "experimental_cm6_editor")) return;
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, `${JSON.stringify({ ...settings, experimental_cm6_editor: false })}\n`);
}

if (process.argv[1] === import.meta.filename) seedClassicEditorProfile(process.env);
