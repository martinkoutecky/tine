#!/usr/bin/env node

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const fixtureRoot = path.join(root, "fixtures/plugin-revocation");
const allowUnsigned = process.argv.includes("--allow-unsigned");
const paths = {
  manifest: path.join(fixtureRoot, "manifest.json"),
  wasm: path.join(fixtureRoot, "plugin.wasm"),
  publicKey: path.join(fixtureRoot, "registry-ed25519.pub.pem"),
  controlIndex: path.join(fixtureRoot, "control-index.json"),
  controlSignature: path.join(fixtureRoot, "control-index.json.sig"),
  revokedIndex: path.join(fixtureRoot, "revoked-index.json"),
  revokedSignature: path.join(fixtureRoot, "revoked-index.json.sig"),
  metadata: path.join(fixtureRoot, "fixture.json"),
};

for (const [name, value] of Object.entries(paths)) {
  if (name === "revokedSignature") continue;
  if (!fs.existsSync(value) || !fs.statSync(value).isFile()) {
    throw new Error(`plugin revocation fixture ${name} is missing: ${path.relative(root, value)}`);
  }
}

const readJson = (file) => JSON.parse(fs.readFileSync(file, "utf8"));
const sha256 = (bytes) => crypto.createHash("sha256").update(bytes).digest("hex");
const verify = (indexPath, signaturePath) => crypto.verify(
  null,
  fs.readFileSync(indexPath),
  fs.readFileSync(paths.publicKey),
  Buffer.from(fs.readFileSync(signaturePath, "utf8").trim(), "base64"),
);
const metadata = readJson(paths.metadata);
const manifest = readJson(paths.manifest);
const control = readJson(paths.controlIndex);
const revoked = readJson(paths.revokedIndex);

if (metadata.identity !== `${manifest.id}@${manifest.version}`) throw new Error("fixture identity metadata does not match manifest");
if (metadata.identity !== "page.tine.e2e.revocation-sentinel@0.0.1") throw new Error("reserved fixture identity changed unexpectedly");
if (manifest.capabilities?.join(",") !== "block-decorations.register"
    || manifest.contributions?.blockDecorations?.length !== 1
    || manifest.contributions.blockDecorations[0].kind !== "thread-lines") {
  throw new Error("sentinel must expose only the harmless host-owned thread-lines contribution");
}
if (control.schemaVersion !== 1 || control.revocations?.length !== 0) throw new Error("positive-control index is not empty");
const matching = revoked.revocations?.filter((item) => `${item.id}@${item.version}` === metadata.identity) ?? [];
if (revoked.schemaVersion !== 1 || matching.length !== 1) throw new Error("revoked index does not contain exactly the reserved identity");
for (const [name, file] of [["manifestSha256", paths.manifest], ["wasmSha256", paths.wasm], ["controlIndexSha256", paths.controlIndex], ["revokedIndexSha256", paths.revokedIndex], ["publicKeySha256", paths.publicKey]]) {
  const actual = sha256(fs.readFileSync(file));
  if (metadata[name] !== actual) throw new Error(`${name} mismatch: expected ${metadata[name]}, got ${actual}`);
}
if (!verify(paths.controlIndex, paths.controlSignature)) throw new Error("production-signed empty positive-control index did not verify");

const runner = fs.readFileSync(path.join(root, "scripts/run-e2e.mjs"), "utf8");
const linuxRelease = runner.slice(runner.indexOf('"linux-release": ['), runner.indexOf('"windows-smoke": ['));
const registeredForRelease = linuxRelease.includes('["plugin-revocation"');

if (!fs.existsSync(paths.revokedSignature)) {
  if (registeredForRelease) throw new Error("plugin-revocation must not be registered in linux-release before its production signature exists");
  const message = `PENDING: ${path.relative(root, paths.revokedSignature)} needs one offline production signature`;
  if (!allowUnsigned) throw new Error(`${message}; see fixtures/plugin-revocation/README.md`);
  console.log(message);
  console.log("PASS (pre-signature preparation only): control signature, identity, schema, and digests verified");
  process.exit(0);
}
if (!verify(paths.revokedIndex, paths.revokedSignature)) throw new Error("revoked-index production signature did not verify");
metadata.revokedSignatureSha256 && (() => {
  const actual = sha256(fs.readFileSync(paths.revokedSignature));
  if (metadata.revokedSignatureSha256 !== actual) throw new Error(`revokedSignatureSha256 mismatch: expected ${metadata.revokedSignatureSha256}, got ${actual}`);
})();
if (!registeredForRelease) throw new Error("production signature verifies; now register plugin-revocation in linux-release and run its burn-in");
console.log(`PASS: production-signed control and revoked fixtures verified for ${metadata.identity}`);
