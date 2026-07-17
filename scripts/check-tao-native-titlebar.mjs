#!/usr/bin/env node

// Native GTK titlebar buttons on Wayland require tao PR #1218. Keep this gate
// until Tauri resolves to a released tao version containing that merge.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const lock = fs.readFileSync(path.join(root, "Cargo.lock"), "utf8");
const tao = lock.match(/\[\[package\]\]\nname = "tao"\nversion = "([^"]+)"\nsource = "([^"]+)"/);
if (!tao) throw new Error("Cargo.lock has no sourced tao package");

const [, version, source] = tao;
const [major, minor] = version.split(".").map(Number);
const releasedFix = major > 0 || minor >= 36;
const pinnedFix = source.includes("07f3742b1833b64be27b1ef991e38d557d4276c9");
if (!releasedFix && !pinnedFix) {
  throw new Error(
    `tao ${version} lacks the Wayland native-titlebar propagation fix; ` +
    "use upstream merge 07f3742b or a released tao 0.36+"
  );
}

console.log(`tao native-titlebar fix OK: ${version} (${pinnedFix ? "upstream merge" : "released"})`);
