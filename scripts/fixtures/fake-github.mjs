#!/usr/bin/env node
// A stand-in for `gh` and `curl` that keeps GitHub's release state in a JSON
// file (FAKE_GITHUB_STATE), for scripts/retire-beta-tag.test.mjs. It covers only
// the calls retire-beta-tag.mjs makes, with GitHub's documented behaviour where
// it matters: a release created on an existing tag keeps that tag's commit, a
// draft's tag is created when it is published, and a failed asset upload can
// leave an empty asset in state "starter".
//
// FAKE_GITHUB_FAIL_AT=n fails the n-th mutating call. A failed upload leaves
// its files as FAKE_GITHUB_PARTIAL says, one outcome per file in order
// (complete | starter | absent; default "complete,starter"); other failed
// mutations change nothing. FAKE_GITHUB_FAIL_READ=<text>:<n> fails the n-th
// read whose arguments contain <text> with an HTTP 503 (which `gh release
// view` reports as "release not found", as the real CLI can).
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
const read = () => {
  const [text, nth] = (process.env.FAKE_GITHUB_FAIL_READ ?? "").split(":");
  if (!text || !args.join(" ").includes(text)) return;
  state.reads = { ...state.reads, [text]: (state.reads?.[text] ?? 0) + 1 };
  save();
  if (state.reads[text] !== Number(nth)) return;
  // gh 2.96 `release view` turns a REST outage into "release not found".
  fail(args[0] === "release" ? "release not found" : "HTTP 503: Service Unavailable");
};
const store = (release, name, outcome) => {
  if (outcome === "absent") return;
  const data = outcome === "starter" ? "" : fs.readFileSync(name, "latin1");
  state.nextAsset = (state.nextAsset ?? 0) + 1;
  release.assets[path.basename(name)] = { id: state.nextAsset, state: outcome === "starter" ? "starter" : "uploaded", data };
};
const complete = (asset) => asset?.state === "uploaded";
// A release as GitHub's REST API returns it.
const rest = (release, owner, repo) => ({
  tag_name: release.tagName, name: release.name, draft: release.isDraft, prerelease: release.isPrerelease,
  target_commitish: release.target,
  assets: Object.entries(release.assets).map(([name, asset]) => ({
    name, state: asset.state, size: Buffer.byteLength(asset.data, "latin1"),
    url: `https://api.github.com/repos/${owner}/${repo}/releases/assets/${asset.id}`,
  })),
});
const download = /^https:\/\/github\.com\/[^/]+\/[^/]+\/releases\/download\/([^/]+)\/([^/?]+)$/;

if (tool === "curl") {
  const url = args.find((arg) => arg.startsWith("https://")).split("?")[0];
  const match = download.exec(url);
  const release = match && byTag(match[1]);
  const asset = release && !release.isDraft ? release.assets[match[2]] : undefined;
  if (args.includes("-w")) {
    process.stdout.write(complete(asset) ? "200" : "404");
    process.exit(complete(asset) ? 0 : 22);
  }
  if (url === "https://tine.page/updates/beta/latest.json") process.stdout.write(JSON.stringify({ version: state.pages }));
  else if (complete(asset)) process.stdout.write(asset.data);
  else fail(`curl: (22) 404 ${url}`, 22);
} else if (args[0] === "api") {
  read();
  if (args[1] === "-X" && args[2] === "DELETE") {
    const endpoint = args[3];
    const assetId = Number(/\/releases\/assets\/(\d+)$/.exec(endpoint)?.[1]);
    if (assetId) {
      const owner = state.releases.find((release) => Object.values(release.assets).some((asset) => asset.id === assetId));
      if (!owner) fail("HTTP 404: Not Found");
      mutate(() => { for (const [name, asset] of Object.entries(owner.assets)) if (asset.id === assetId) delete owner.assets[name]; });
    } else {
      const tag = endpoint.split("/git/refs/tags/")[1];
      if (!state.tags[tag]) fail("HTTP 422: Reference does not exist");
      mutate(() => delete state.tags[tag]);
    }
  } else if (args.includes("--paginate")) {
    const [owner, repo] = args.find((arg) => arg.startsWith("repos/")).split("/").slice(1, 3);
    process.stdout.write(JSON.stringify([state.releases.map((release) => rest(release, owner, repo))]));
  } else if (args[1].includes("/releases/tags/")) {
    const [, owner, repo] = args[1].split("/");
    const release = byTag(args[1].split("/releases/tags/")[1]);
    if (!release || release.isDraft) fail("gh: Not Found (HTTP 404)");
    process.stdout.write(JSON.stringify(rest(release, owner, repo)));
  } else {
    const ref = args[1].split("/commits/")[1];
    if (!state.tags[ref]) fail(`HTTP 422: No commit found for SHA: ${ref}`);
    process.stdout.write(`${state.tags[ref]}\n`);
  }
} else if (args[0] === "release") {
  const [, verb, tag] = args;
  const release = byTag(tag);
  if (verb === "view") {
    read();
    // Like gh 2.96, any failed lookup reads as "release not found".
    if (!release) fail("release not found");
    process.stdout.write(JSON.stringify(release));
  } else if (verb === "download") {
    if (!release) fail("release not found");
    const dir = option("--dir");
    fs.mkdirSync(dir, { recursive: true });
    const patterns = args.flatMap((arg, i) => (args[i - 1] === "--pattern" ? [arg] : []));
    for (const name of patterns) {
      if (!complete(release.assets[name])) fail(`no asset ${name}`);
      if (fs.existsSync(path.join(dir, name)) && !args.includes("--clobber")) fail(`${name} already exists`);
      fs.writeFileSync(path.join(dir, name), release.assets[name].data, "latin1");
    }
  } else if (verb === "edit") {
    if (!release) fail("release not found");
    mutate(() => {
      if (option("--tag")) release.tagName = option("--tag");
      if (option("--target")) release.target = option("--target");
      if (args.includes("--draft=false")) {
        release.isDraft = false;
        // Publishing uses an existing tag as it is; only a missing one is created.
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
    const clobber = args.includes("--clobber");
    if (!clobber && files.some((name) => release.assets[path.basename(name)])) fail("asset under the same name already exists");
    const outcomes = (process.env.FAKE_GITHUB_PARTIAL ?? "complete,starter").split(",");
    mutate(
      () => files.forEach((name) => store(release, name, "complete")),
      () => files.forEach((name, i) => {
        if (clobber) delete release.assets[path.basename(name)];
        store(release, name, outcomes[i] ?? "absent");
      }),
    );
  } else fail(`fake gh: unsupported release ${verb}`);
} else fail(`fake gh: unsupported ${args.join(" ")}`);
