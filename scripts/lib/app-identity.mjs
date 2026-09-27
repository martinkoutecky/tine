// The app identifier names the app-data, desktop-entry and Wayland app-ID paths.
// og ships its own identifier so it can coexist with master; journeys must read
// it from the Tauri config rather than spell it (src/e2eAppIdentity.guard.test.ts).
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const conf = path.join(path.dirname(fileURLToPath(import.meta.url)), "../../src-tauri/tauri.conf.json");
export const APP_ID = JSON.parse(fs.readFileSync(conf, "utf8")).identifier;
