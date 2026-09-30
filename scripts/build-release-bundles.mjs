#!/usr/bin/env node
import fs from "node:fs";
import { spawnSync } from "node:child_process";
import { releaseChannel, packagingProblems } from "./release-policy.mjs";

const conf = JSON.parse(fs.readFileSync("src-tauri/tauri.conf.json", "utf8"));
const ship = JSON.parse(fs.readFileSync("src-tauri/app-identity.json", "utf8")).ship;
const channel = releaseChannel(conf);
const problems = packagingProblems(conf, ship);
if (problems.length) throw new Error(problems.join("\n"));
const env = { ...process.env };
for (const name of ["UPDATE_INFORMATION", "LDAI_UPDATE_INFORMATION"]) {
  if (env[name]) env[name] = env[name].replace("|latest|", `|${channel}|`);
}
const args = process.argv.slice(2).filter((arg) => arg !== "--");
const result = spawnSync(process.platform === "win32" ? "npm.cmd" : "npm", ["run", "tauri", "build", "--", ...args], { env, stdio: "inherit", shell: process.platform === "win32" });
if (result.error) throw result.error;
process.exit(result.status ?? 1);
