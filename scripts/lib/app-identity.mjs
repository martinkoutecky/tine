// The app identity front door for scripts. The ONE switch is
// src-tauri/app-identity.json (docs/app-identity.md): `ship` selects the
// experiment identity (page.tine.TineOG, coexists with the released Tine) or
// the released one. Every identity-bearing file derives from it through
// `deriveIdentityFiles`; `node scripts/set-app-identity.mjs <ship>` writes them
// and src/appIdentity.guard.test.ts fails when any of them drifts. Journeys must
// read APP_ID from here rather than spell an identifier.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
export const SWITCH_FILE = "src-tauri/app-identity.json";

export function readSwitch(root = ROOT) {
  return JSON.parse(fs.readFileSync(path.join(root, SWITCH_FILE), "utf8"));
}

const SWITCH = readSwitch();
export const SHIP = SWITCH.ship;
export const IDENTITIES = SWITCH.identities;
export const IDENTITY = IDENTITIES[SHIP];
export const APP_ID = IDENTITY.identifier;

/**
 * The derived identity-bearing files for `ship`, as { relativePath: text },
 * computed from the files currently under `root`. Pure: writes nothing.
 */
export function deriveIdentityFiles(root, ship) {
  const sw = readSwitch(root);
  const identity = sw.identities[ship];
  if (!identity) throw new Error(`unknown ship "${ship}" (expected one of ${Object.keys(sw.identities).join(", ")})`);
  const read = (file) => fs.readFileSync(path.join(root, file), "utf8");
  const out = {};

  out[SWITCH_FILE] = JSON.stringify({ ...sw, ship }, null, 2) + "\n";

  // Targeted line edits keep the file's hand formatting (a JSON round trip would not).
  const confFile = "src-tauri/tauri.conf.json";
  let conf = read(confFile);
  const edits = [
    [/^(  "identifier": )"[^"]*"/m, identity.identifier],
    [/^(  "productName": )"[^"]*"/m, identity.productName],
    [/("label": "main",\s*"title": )"[^"]*"/, identity.productName],
  ];
  for (const [pattern, value] of edits) {
    if (!pattern.test(conf)) throw new Error(`${confFile}: no match for ${pattern}`);
    conf = conf.replace(pattern, `$1${JSON.stringify(value)}`);
  }
  out[confFile] = conf;

  const gradleFile = "src-tauri/gen/android/app/build.gradle.kts";
  const gradle = read(gradleFile);
  const applicationId = /^(\s*applicationId = )"[^"]*"$/m;
  if (!applicationId.test(gradle)) throw new Error(`${gradleFile}: no applicationId line`);
  out[gradleFile] = gradle.replace(applicationId, `$1"${identity.androidApplicationId}"`);
  return out;
}
