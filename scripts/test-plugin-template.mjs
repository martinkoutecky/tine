#!/usr/bin/env node

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const source = path.join(root, "plugin-sdk", "templates", "rust");
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "tine-plugin-template-"));
const plugin = path.join(temporary, "my-tine-plugin");

function run(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, stdio: "inherit", env: process.env });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status}`);
  }
}

try {
  fs.cpSync(source, plugin, { recursive: true });
  run("cargo", ["build", "--release", "--locked"], plugin);
  run("node", [path.join(root, "scripts", "tine-plugin.mjs"), "check", plugin, "--json"], root);
  console.log("standalone plugin template: build and conformance check passed");
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}
