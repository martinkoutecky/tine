#!/usr/bin/env node
// tine#1011: every identity's .deb installs the same paths, so a built .deb must
// name the other identities' packages in Conflicts and Replaces; dpkg then removes
// the other package instead of refusing to install. Checks built packages:
// `node scripts/check-deb-replaces.mjs <dir-or-deb>...`.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { IDENTITIES, linuxPackageName } from "./lib/app-identity.mjs";

const names = Object.values(IDENTITIES).map(linuxPackageName);
const debs = process.argv.slice(2).flatMap((arg) => fs.statSync(arg).isDirectory()
  ? fs.readdirSync(arg).filter((name) => name.endsWith(".deb")).map((name) => path.join(arg, name))
  : [arg]);
if (!debs.length) throw new Error("no .deb to check");
const field = (deb, name) => execFileSync("dpkg-deb", ["-f", deb, name], { encoding: "utf8" }).trim();
const list = (text) => text.split(",").map((entry) => entry.trim()).filter(Boolean).sort();
let failed = false;
for (const deb of debs) {
  const own = field(deb, "Package");
  if (!names.includes(own)) throw new Error(`${deb}: package "${own}" is not one of ${names.join(", ")}`);
  const others = names.filter((name) => name !== own).sort();
  for (const name of ["Conflicts", "Replaces"]) {
    const got = list(field(deb, name));
    if (JSON.stringify(got) === JSON.stringify(others)) continue;
    failed = true;
    console.error(`${deb}: ${name} is "${got.join(", ")}", expected "${others.join(", ")}"`);
  }
  if (!failed) console.log(`${path.basename(deb)}: ${own} replaces ${others.join(", ")}`);
}
if (failed) process.exit(1);
