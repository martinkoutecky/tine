#!/usr/bin/env node
// Flip the app identity switch: `node scripts/set-app-identity.mjs release|experiment`.
// Rewrites src-tauri/app-identity.json and every file derived from it
// (docs/app-identity.md). `--check` writes nothing and exits 1 on drift.
import fs from "node:fs";
import path from "node:path";
import { ROOT, deriveIdentityFiles, readSwitch } from "./lib/app-identity.mjs";

const args = process.argv.slice(2);
const check = args.includes("--check");
const ship = args.find((arg) => !arg.startsWith("--")) ?? readSwitch().ship;
const files = deriveIdentityFiles(ROOT, ship);
let drift = 0;
for (const [file, text] of Object.entries(files)) {
  const full = path.join(ROOT, file);
  if (fs.readFileSync(full, "utf8") === text) continue;
  drift += 1;
  if (check) console.error(`drift: ${file}`);
  else fs.writeFileSync(full, text);
}
if (check && drift) process.exit(1);
console.log(check ? `identity "${ship}": no drift` : `identity "${ship}": ${drift} file(s) rewritten`);
