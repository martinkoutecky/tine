#!/usr/bin/env node
import fs from "node:fs";
import { spawnSync } from "node:child_process";
import { BETA_TAG, BETA_UPDATES, packagingProblems, releaseChannel } from "./release-policy.mjs";

const conf = JSON.parse(fs.readFileSync("src-tauri/tauri.conf.json", "utf8"));
const ship = JSON.parse(fs.readFileSync("src-tauri/app-identity.json", "utf8")).ship;
const channel = releaseChannel(conf);
const problems = packagingProblems(conf, ship);
if (problems.length) throw new Error(problems.join("\n"));
const env = { ...process.env };
// AppImage update information comes from release-workflow-inputs.mjs; refuse
// one that names the other channel's source (a Beta must read tine.page).
for (const name of ["UPDATE_INFORMATION", "LDAI_UPDATE_INFORMATION"]) {
  if (!env[name]) continue;
  const beta = env[name].startsWith(`zsync|${BETA_UPDATES}`);
  if (beta !== (channel === BETA_TAG)) throw new Error(`${name} does not match the ${channel} channel: ${env[name]}`);
}
const args = process.argv.slice(2).filter((arg) => arg !== "--");
const result = spawnSync(process.platform === "win32" ? "npm.cmd" : "npm", ["run", "tauri", "build", "--", ...args], { env, stdio: "inherit", shell: process.platform === "win32" });
if (result.error) throw result.error;
process.exit(result.status ?? 1);
