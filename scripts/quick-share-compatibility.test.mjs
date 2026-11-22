import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { chromium } from "playwright";
import { checkViewer, serveViewer } from "./check-quick-share-viewer.mjs";
import { fixtureFingerprint } from "./quick-share-viewer-lib.mjs";

// Run against a production build, also when packaging the viewer. An archived
// viewer can be supplied to prove compatibility across writer release lines.
const root = path.resolve(import.meta.dirname, "..");
const source = process.env.QUICK_SHARE_VIEWER_DIR || path.join(root, "artifacts/quick-share-viewer/build");
const generation = 2;
const artifacts = path.join(root, "target/artifacts/quick-share-compatibility");
fs.mkdirSync(artifacts, { recursive: true });

test("a cold share requests its stylesheet while the first script is still in flight", async () => {
  const hosted = await serveViewer(source, generation);
  const browser = await chromium.launch();
  const release = Promise.withResolvers();
  try {
    const page = await browser.newPage();
    const fixture = path.join(root, "tests/fixtures/quick-share/g2/pages");
    const link = new URL(fs.readFileSync(path.join(fixture, "link.txt"), "utf8").trim());
    await page.route("https://gist.githubusercontent.com/**", route => route.fulfill({
      body: fs.readFileSync(path.join(fixture, "tine-share.bin"), "utf8"),
      headers: { "access-control-allow-origin": "*" },
    }));
    await page.route("**/assets/*.js", async route => {
      await release.promise;
      await route.continue();
    });
    const firstScript = page.waitForRequest(request => request.resourceType() === "script");
    const stylesheet = page.waitForRequest(request => request.resourceType() === "stylesheet",
      { timeout: 5_000 }).then(() => true, () => false);
    const response = await page.goto(`${hosted.url}${link.pathname.slice("/s/2/".length)}${link.hash}/page/Project`,
      { waitUntil: "commit" });
    assert.equal(response.status(), 404);
    await firstScript;
    await page.locator("#root").waitFor({ state: "attached" });
    const missingHeading = await page.getByRole("heading", { name: "Page not found" }).count();
    assert.deepEqual({ missingHeading, requestedStylesheet: await stylesheet },
      { missingHeading: 0, requestedStylesheet: true });
    release.resolve();
    await page.locator(".page-title").filter({ hasText: "Project" }).first().waitFor();
  } finally {
    release.resolve();
    await browser.close();
    await hosted.close();
  }
});

test("a viewer built for another corpus enables this writer only after rendering its fixtures", async () => {
  const directory = fs.mkdtempSync(path.join(artifacts, "different-corpus-"));
  fs.cpSync(source, directory, { recursive: true });
  const file = path.join(directory, "viewer.json");
  const manifest = JSON.parse(fs.readFileSync(file, "utf8"));
  // Only build provenance changes; the production reader and its hashes stay intact.
  manifest.fixture_sha256 = "0".repeat(64);
  assert.notEqual(manifest.fixture_sha256, fixtureFingerprint(root, generation));
  fs.writeFileSync(file, JSON.stringify(manifest));
  assert.equal(await checkViewer(directory, { generation }), fixtureFingerprint(root, generation));
});

test("matching build provenance cannot enable a viewer that refuses a share", async () => {
  const directory = fs.mkdtempSync(path.join(artifacts, "incompatible-"));
  fs.cpSync(source, directory, { recursive: true });
  const file = path.join(directory, "viewer.json");
  const manifest = JSON.parse(fs.readFileSync(file, "utf8"));
  manifest.fixture_sha256 = fixtureFingerprint(root, generation);
  fs.writeFileSync(file, JSON.stringify(manifest));
  const fixture = path.join(root, "tests/fixtures/quick-share/g2/pages");
  const bytes = Buffer.from(fs.readFileSync(path.join(fixture, "tine-share.bin"), "utf8"), "base64url");
  bytes[4] = 2; // The viewer cannot open a newer envelope version.
  await assert.rejects(checkViewer(directory, {
    generation,
    envelope: bytes.toString("base64url"),
    link: fs.readFileSync(path.join(fixture, "link.txt"), "utf8").trim(),
  }), /Viewer compatibility failed \(native-created\)/);
  const failure = JSON.parse(fs.readFileSync(path.join(root, "artifacts/quick-share-viewer/check-failure.json"), "utf8"));
  assert.match(failure.observed, /viewer refused the share/);
});
