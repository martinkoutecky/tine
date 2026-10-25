#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { deployViewers, unpackViewer, assertRetainedBuilds } from "./quick-share-viewer-lib.mjs";

const root = path.resolve(import.meta.dirname, "..");
const website = path.resolve(process.argv[2] ?? path.join(root, "website"));
const manifest = JSON.parse(fs.readFileSync(path.join(website, "s/generations.json"), "utf8"));
const previous = await fetch("https://tine.page/s/generations.json", { cache: "no-store", redirect: "error", signal: AbortSignal.timeout(15_000) });
if (previous.ok) assertRetainedBuilds(await previous.json(), manifest);
else if (previous.status !== 404) throw new Error("cannot verify previously deployed viewer retention");
const artifacts = path.join(root, "artifacts/quick-share-viewer/downloads");
fs.mkdirSync(artifacts, { recursive: true });
const cached = new Map();
await deployViewers({ manifest, website, download: async (pin, generation) => {
  if (cached.has(pin.sha256)) return cached.get(pin.sha256);
  const scratch = fs.mkdtempSync(path.join(artifacts, "build-"));
  if (pin.tag) execFileSync("gh", ["release", "download", pin.tag, "--pattern", `quick-share-viewer-g${generation}.zip`, "--dir", scratch], { stdio: "inherit" });
  else execFileSync("gh", ["run", "download", String(pin.run_id), "--name", `quick-share-viewer-g${generation}`, "--dir", scratch], { stdio: "inherit" });
  const dir = path.join(scratch, "unpacked");
  unpackViewer(path.join(scratch, `quick-share-viewer-g${generation}.zip`), dir, pin.sha256);
  cached.set(pin.sha256, dir); return dir;
} });
console.log("Quick share viewers verified and staged");
