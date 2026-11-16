import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { checkViewer } from "./check-quick-share-viewer.mjs";
import { fixtureFingerprint } from "./quick-share-viewer-lib.mjs";

// Run against a production build, also when packaging the viewer. An archived
// viewer can be supplied to prove compatibility across writer release lines.
const root = path.resolve(import.meta.dirname, "..");
const source = process.env.QUICK_SHARE_VIEWER_DIR || path.join(root, "artifacts/quick-share-viewer/build");
const artifacts = path.join(root, "target/artifacts/quick-share-compatibility");
fs.mkdirSync(artifacts, { recursive: true });

test("a viewer built for another corpus enables this writer only after rendering its fixtures", async () => {
  const directory = fs.mkdtempSync(path.join(artifacts, "different-corpus-"));
  fs.cpSync(source, directory, { recursive: true });
  const file = path.join(directory, "viewer.json");
  const manifest = JSON.parse(fs.readFileSync(file, "utf8"));
  // Only build provenance changes; the production reader and its hashes stay intact.
  manifest.fixture_sha256 = "0".repeat(64);
  assert.notEqual(manifest.fixture_sha256, fixtureFingerprint(root));
  fs.writeFileSync(file, JSON.stringify(manifest));
  assert.equal(await checkViewer(directory), fixtureFingerprint(root));
});

test("matching build provenance cannot enable a viewer that refuses a share", async () => {
  const directory = fs.mkdtempSync(path.join(artifacts, "incompatible-"));
  fs.cpSync(source, directory, { recursive: true });
  const file = path.join(directory, "viewer.json");
  const manifest = JSON.parse(fs.readFileSync(file, "utf8"));
  manifest.fixture_sha256 = fixtureFingerprint(root);
  fs.writeFileSync(file, JSON.stringify(manifest));
  const fixture = path.join(root, "tests/fixtures/quick-share/g1/pages");
  const bytes = Buffer.from(fs.readFileSync(path.join(fixture, "tine-share.bin"), "utf8"), "base64url");
  bytes[4] = 2; // The generation-1 reader cannot open this envelope version.
  await assert.rejects(checkViewer(directory, {
    envelope: bytes.toString("base64url"),
    link: fs.readFileSync(path.join(fixture, "link.txt"), "utf8").trim(),
  }), /Viewer compatibility failed \(native-created\)/);
  const failure = JSON.parse(fs.readFileSync(path.join(root, "artifacts/quick-share-viewer/check-failure.json"), "utf8"));
  assert.match(failure.observed, /viewer refused the share/);
});
