#!/usr/bin/env node

import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { validateGuideDemoLinks } from "./guide-demo-validator.mjs";

const fixture = mkdtempSync(path.join(tmpdir(), "tine-guide-demo-validator-"));

try {
  writeFileSync(
    path.join(fixture, "intentional-examples.html"),
    '<a class="ref" href="link.html">link</a><a class="tag" href="demo.html">#demo</a>',
  );
  validateGuideDemoLinks(fixture);

  writeFileSync(
    path.join(fixture, "accidental-targets.html"),
    '<a class="ref" href="accidental-ref.html">unregistered page</a><a class="tag" href="accidental-tag.html">#unregistered-tag</a>',
  );
  assert.throws(
    () => validateGuideDemoLinks(fixture),
    (error) => {
      assert.match(error.message, /accidental-targets\.html: missing local target accidental-ref\.html/);
      assert.match(error.message, /accidental-targets\.html: missing local target accidental-tag\.html/);
      return true;
    },
  );
} finally {
  rmSync(fixture, { recursive: true, force: true });
}

console.log("Guide/demo validator rejects accidental links while accepting deliberate stubs.");
