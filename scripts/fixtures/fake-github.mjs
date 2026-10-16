#!/usr/bin/env node
// A stand-in for `gh` and `curl` that keeps GitHub's release state in a JSON
// file (FAKE_GITHUB_STATE), for scripts/retire-beta-tag.test.mjs. It covers only
// the calls retire-beta-tag.mjs makes. FAKE_GITHUB_FAIL_AT=n makes the n-th
// mutating call fail without applying it (an upload first stores one file, as a
// dropped connection can).
import fs from "node:fs";
import path from "node:path";

const file = process.env.FAKE_GITHUB_STATE;
const state = JSON.parse(fs.readFileSync(file, "utf8"));
const save = () => fs.writeFileSync(file, JSON.stringify(state, null, 2));
const [tool, ...args] = process.argv.slice(2);
const option = (name) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined);
const fail = (message, code = 1) => {
  process.stderr.write(`${message}\n`);
  process.exit(code);
};
const byTag = (tag) => state.releases.find((release) => release.tagName === tag);
const mutate = (apply, partial = () => {}) => {
  state.mutations += 1;
  if (Number(process.env.FAKE_GITHUB_FAIL_AT) === state.mutations) {
    partial();
    save();
    fail(`injected failure at mutation ${state.mutations}`);
  }
  apply();
  save();
};
const download = /^https:\/\/github\.com\/[^/]+\/[^/]+\/releases\/download\/([^/]+)\/([^/?]+)$/;

if (tool === "curl") {
  const url = args.find((arg) => arg.startsWith("https://")).split("?")[0];
  const match = download.exec(url);
  const release = match && byTag(match[1]);
  const asset = release && !release.isDraft ? release.assets[match[2]] : undefined;
  if (args.includes("-w")) {
    process.stdout.write(asset === undefined ? "404" : "200");
    process.exit(asset === undefined ? 22 : 0);
  }
  if (url === "https://tine.page/updates/beta/latest.json") process.stdout.write(JSON.stringify({ version: state.pages }));
  else if (asset !== undefined) process.stdout.write(asset);
  else fail(`curl: (22) 404 ${url}`, 22);
} else if (args[0] === "api") {
  if (args[1] === "-X" && args[2] === "DELETE") {
    const tag = args[3].split("/git/refs/tags/")[1];
    if (!state.tags[tag]) fail("HTTP 422: Reference does not exist");
    mutate(() => delete state.tags[tag]);
  } else {
    const ref = args[1].split("/commits/")[1];
    if (!state.tags[ref]) fail("HTTP 422: No commit found");
    process.stdout.write(`${state.tags[ref]}\n`);
  }
} else if (args[0] === "release") {
  const [, verb, tag] = args;
  const release = byTag(tag);
  if (verb === "view") {
    if (!release) fail("release not found");
    process.stdout.write(JSON.stringify({ ...release, assets: Object.keys(release.assets).map((name) => ({ name })) }));
  } else if (verb === "download") {
    if (!release) fail("release not found");
    const dir = option("--dir");
    fs.mkdirSync(dir, { recursive: true });
    const patterns = args.flatMap((arg, i) => (args[i - 1] === "--pattern" ? [arg] : []));
    for (const name of patterns) {
      if (release.assets[name] === undefined) fail(`no asset ${name}`);
      fs.writeFileSync(path.join(dir, name), release.assets[name], "latin1");
    }
  } else if (verb === "edit") {
    if (!release) fail("release not found");
    mutate(() => {
      if (option("--tag")) release.tagName = option("--tag");
      if (args.includes("--draft=false")) {
        release.isDraft = false;
        state.tags[release.tagName] ??= release.target;
      }
    });
  } else if (verb === "create") {
    if (release) fail("a release with the same tag name already exists");
    mutate(() => state.releases.push({ tagName: tag, name: option("--title"), isDraft: args.includes("--draft"),
      isPrerelease: args.includes("--prerelease"), target: option("--target"), notes: fs.readFileSync(option("--notes-file"), "utf8"), assets: {} }));
  } else if (verb === "upload") {
    if (!release) fail("release not found");
    const files = args.slice(3).filter((arg) => arg.startsWith("/"));
    const store = (name) => { release.assets[path.basename(name)] = fs.readFileSync(name, "latin1"); };
    mutate(() => files.forEach(store), () => store(files[0]));
  } else fail(`fake gh: unsupported release ${verb}`);
} else fail(`fake gh: unsupported ${args.join(" ")}`);
