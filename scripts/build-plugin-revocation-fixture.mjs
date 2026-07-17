#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const fixtureRoot = path.join(root, "fixtures/plugin-revocation");
const crateRoot = path.join(fixtureRoot, "sentinel-src");
const buildArgs = ["build", "--release", "--locked"];
if (process.env.TINE_PLUGIN_OFFLINE === "1") buildArgs.push("--offline");
const result = spawnSync("cargo", buildArgs, {
  cwd: crateRoot,
  stdio: "inherit",
  env: process.env,
});
if (result.status !== 0) process.exit(result.status ?? 1);

const source = path.join(crateRoot, "target/wasm32-unknown-unknown/release/tine_plugin_revocation_sentinel.wasm");
const destination = path.join(fixtureRoot, "plugin.wasm");
fs.copyFileSync(source, destination);
console.log(`wrote ${path.relative(root, destination)} (${fs.statSync(destination).size} bytes)`);
