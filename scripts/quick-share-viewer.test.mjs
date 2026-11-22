import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { chromium } from "playwright";
import { deployViewers, fileHashes, mergeViewerBuild, unpackViewer, validatePin, validateViewer, sha256, packViewer, assertRetainedBuilds } from "./quick-share-viewer-lib.mjs";
import { servePages } from "./check-quick-share-viewer.mjs";

const artifacts = path.resolve(import.meta.dirname, "../artifacts/quick-share-viewer/tests");
fs.mkdirSync(artifacts, { recursive: true });
function scratch() { return fs.mkdtempSync(path.join(artifacts, "case-")); }
function build(dir, version, chunk = `late-${version}.js`, generation = 1) {
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "index.html"), `<button onclick="import('./${chunk}').then(m=>document.body.dataset.loaded=m.version)">Load later</button>`);
  fs.writeFileSync(path.join(dir, chunk), `export const version = '${version}';`);
  fs.writeFileSync(path.join(dir, "viewer.json"), JSON.stringify({ generation, link: generation, envelope: 1, document: generation, snapshot_schema: generation, built_from: version, files: fileHashes(dir) }));
}
test("old HTML loads a previously unloaded chunk after upgrade and rollback", async () => {
  const dir = scratch(), old = path.join(dir, "old"), next = path.join(dir, "next"), website = path.join(dir, "website");
  const deployed = path.join(website, "s/1");
  build(old, "old"); build(next, "new");
  mergeViewerBuild(old, deployed);
  const server = await servePages(website), browser = await chromium.launch();
  const url = new URL("s/1/", server.url).href;
  try {
    const oldTab = await browser.newPage(); await oldTab.goto(url);
    mergeViewerBuild(next, deployed);
    const newTab = await browser.newPage(); await newTab.goto(url);
    await oldTab.getByRole("button").click();
    await oldTab.waitForFunction(() => document.body.dataset.loaded === "old");
    mergeViewerBuild(old, deployed);
    await newTab.getByRole("button").click();
    await newTab.waitForFunction(() => document.body.dataset.loaded === "new");
    const rolledBack = await browser.newPage(); await rolledBack.goto(url);
    await rolledBack.getByRole("button").click();
    await rolledBack.waitForFunction(() => document.body.dataset.loaded === "old");
  } finally { await browser.close(); await server.close(); }
});
test("a reused asset name with different bytes refuses deployment", () => {
  const dir = scratch(), a = path.join(dir, "a"), b = path.join(dir, "b");
  build(a, "old", "same.js"); build(b, "new", "same.js");
  assert.throws(() => mergeViewerBuild(b, a), /collision/);
});
test("a missing archive, wrong zip digest and wrong file digest all refuse", async () => {
  const dir = scratch(), source = path.join(dir, "source"); build(source, "test");
  const zip = path.join(dir, "viewer.zip"); packViewer(source, zip);
  const hash = sha256(fs.readFileSync(zip));
  assert.throws(() => unpackViewer(path.join(dir, "missing.zip"), path.join(dir, "unpacked"), hash), /ENOENT/);
  assert.throws(() => unpackViewer(zip, path.join(dir, "unpacked"), "0".repeat(64)), /SHA-256/);
  unpackViewer(zip, path.join(dir, "valid"), hash);
  fs.writeFileSync(path.join(dir, "valid", "late-test.js"), "changed");
  assert.throws(() => validateViewer(path.join(dir, "valid")), /hashes mismatch/);
  const pin = { tag: "v0.9.0-beta.1", sha256: hash };
  await assert.rejects(deployViewers({ manifest: { schema: 1, generations: { 1: { current: pin, retained: [pin] } } }, website: dir, download: async () => { throw new Error("missing archive"); } }), /missing archive/);
});
test("current must stay in retained, and pin identities are unambiguous", async () => {
  const pin = { tag: "v0.9.0-beta.1", sha256: "1".repeat(64) };
  await assert.rejects(deployViewers({ manifest: { schema: 1, generations: { 1: { current: pin, retained: [] } } }, website: scratch(), download: async () => assert.fail("should refuse first") }), /retained/);
  assert.throws(() => validatePin({ ...pin, run_id: 123 }), /exactly one/);
  assert.throws(() => validatePin({ tag: "missing-digest" }), /SHA-256/);
});
test("retention cannot shrink, while the same bytes may move from a trial artifact to a release archive", () => {
  const old = { generations: { 1: { retained: [{ run_id: 123, sha256: "a".repeat(64) }] } } };
  assert.throws(() => assertRetainedBuilds(old, { generations: {} }), /cannot remove retained/);
  assert.doesNotThrow(() => assertRetainedBuilds(old, { generations: { 1: { retained: [{ tag: "v0.9.0-beta.1", sha256: "a".repeat(64) }] } } }));
});

test("Pages serves the pinned generation 2 viewer at the locator and keeps generation 1 at its index", async () => {
  const dir = scratch(), website = path.join(dir, "website");
  const one = path.join(dir, "one"), two = path.join(dir, "two");
  build(one, "one");
  build(two, "two", "two.js", 2);
  assert.equal(validateViewer(two, 2).envelope, 1);
  assert.throws(() => validateViewer(two, 1), /generation mismatch/);
  const first = { run_id: 1, sha256: "1".repeat(64) }, second = { run_id: 2, sha256: "2".repeat(64) };
  await deployViewers({ manifest: { schema: 1, generations: {
    1: { current: first, retained: [first] },
    2: { current: second, retained: [second] },
  } }, website, download: async (_, generation) => generation === 1 ? one : two });
  const hosted = await servePages(website);
  try {
    const old = await fetch(`${hosted.url}s/1/#tine-fixture/0123456789abcdef0123/key`);
    assert.equal(old.status, 200);
    assert.equal(await old.text(), fs.readFileSync(path.join(one, "index.html"), "utf8"));
    const response = await fetch(`${hosted.url}s/2/tine-fixture/0123456789abcdef0123#key/page/Project`);
    assert.equal(response.status, 404);
    assert.equal(await response.text(), fs.readFileSync(path.join(two, "index.html"), "utf8"));
  } finally { await hosted.close(); }
});

test("an unpinned generation installs no viewer", async () => {
  const website = scratch();
  const manifest = { schema: 1, generations: { 2: { current: null, retained: [] } } };
  await deployViewers({ manifest, website, download: async () => assert.fail("unpinned viewer downloaded") });
  assert.equal(fs.existsSync(path.join(website, "404.html")), false);
});
