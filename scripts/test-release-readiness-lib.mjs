#!/usr/bin/env node

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { auditableSourceFingerprint } from "./release-readiness-lib.mjs";

const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "tine-release-readiness-test-"));

function write(relative, contents) {
  const absolute = path.join(temporary, relative);
  fs.mkdirSync(path.dirname(absolute), { recursive: true });
  fs.writeFileSync(absolute, contents);
}

try {
  const trackedFixtures = new Map([
    ["plugin-sdk/rust/src/lib.rs", "pub const API_VERSION: u32 = 1;\n"],
    ["plugin-sdk/schema/manifest.schema.json", '{"type":"object"}\n'],
    ["community-plugins/example/src/lib.rs", "pub fn activate() {}\n"],
    ["community-plugins/example/manifest.json", '{"id":"example"}\n'],
  ]);
  for (const [relative, contents] of trackedFixtures) write(relative, contents);

  const releaseEvidence = new Map([
    ["docs/releases/v0.6.0-audit-attestation.json", '{"sourceFingerprint":"before"}\n'],
    ["docs/releases/v0.6.0-impact.json", '{"items":[]}\n'],
  ]);
  for (const [relative, contents] of releaseEvidence) write(relative, contents);

  const revocationProofEvidence = new Map([
    ["fixtures/plugin-revocation/README.md", "Proof handoff notes.\n"],
    ["fixtures/plugin-revocation/fixture.json", '{"revokedSignatureSha256":"proof-only"}\n'],
    ["fixtures/plugin-revocation/revoked-index.json.sig", "signed-proof-only\n"],
  ]);
  for (const [relative, contents] of revocationProofEvidence) write(relative, contents);
  write("fixtures/plugin-revocation/revoked-index.json", '{"generation":1}\n');

  const baseline = auditableSourceFingerprint(temporary);
  assert.equal(auditableSourceFingerprint(temporary), baseline, "unchanged inputs must hash deterministically");

  for (const relative of ["plugin-sdk/rust/src/lib.rs", "community-plugins/example/src/lib.rs"]) {
    const original = trackedFixtures.get(relative);
    write(relative, `${original}// changed\n`);
    assert.notEqual(
      auditableSourceFingerprint(temporary),
      baseline,
      `${relative} must invalidate the audit fingerprint`,
    );
    write(relative, original);
    assert.equal(auditableSourceFingerprint(temporary), baseline, `${relative} restoration must restore the fingerprint`);
  }

  write("community-plugins/example/target/release/example.wasm", "ignored build output\n");
  write("fixtures/plugin-revocation/sentinel-src/target/release/sentinel.wasm", "ignored fixture build output\n");
  assert.equal(
    auditableSourceFingerprint(temporary),
    baseline,
    "generated plugin and fixture build output must not invalidate the audit fingerprint",
  );

  for (const [relative, original] of releaseEvidence) {
    write(relative, `${original.trimEnd()} changed\n`);
    assert.equal(
      auditableSourceFingerprint(temporary),
      baseline,
      `${relative} must not invalidate the source audit that it records`,
    );
  }

  for (const [relative, original] of revocationProofEvidence) {
    write(relative, `${original.trimEnd()} changed\n`);
    assert.equal(
      auditableSourceFingerprint(temporary),
      baseline,
      `${relative} must not invalidate the production-source audit it supports`,
    );
  }

  write("fixtures/plugin-revocation/revoked-index.json", '{"generation":2}\n');
  assert.notEqual(
    auditableSourceFingerprint(temporary),
    baseline,
    "the material revocation fixture must continue to invalidate the audit fingerprint",
  );
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}

console.log("Release-readiness fingerprint tests passed (production/plugin sources covered; proof evidence excluded).");
