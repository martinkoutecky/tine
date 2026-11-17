import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";

// D-20261117-QSG: every build offers quick share links, so no build input may
// switch them off, and the release refuses to ship while tine.page cannot
// render this writer's links. Source assertions, as in test-release-pipeline.mjs.
const workflows = new URL("../.github/workflows/", import.meta.url);
const read = (name) => fs.readFileSync(new URL(`${name}.yml`, workflows), "utf8");
const check = "node scripts/check-release-readiness.mjs --quick-share-only";

function job(workflow, name) {
  const body = workflow.split(`\n  ${name}:\n`)[1]?.split(/\n  [\w-]+:\n/)[0];
  assert.ok(body, `missing job ${name}`);
  return body;
}

test("the release preflight refuses a release the deployed viewer cannot read", () => {
  const release = read("release");
  const steps = job(release, "preflight").split(/\n      - /).slice(1);
  const checks = steps.filter((step) => step.split("\n").some((line) => line.trim() === `run: ${check}`));
  assert.equal(checks.length, 1, "run the viewer check exactly once, in preflight");
  assert.doesNotMatch(checks[0], /continue-on-error/, "a failed viewer check blocks the release");
  assert.doesNotMatch(checks[0], /\n        if:/, "the viewer check runs on every release");
  const before = steps.slice(0, steps.indexOf(checks[0])).join("\n");
  assert.match(before, /npm ci/, "install the check's dependencies");
  assert.match(before, /npx playwright install(?: --with-deps)? chromium/, "install the browser before the check");
  for (const name of ["build", "android"]) assert.match(job(release, name), /needs: preflight\n/, `${name} waits for preflight`);
});

test("no workflow switches quick share off at build time", () => {
  for (const file of fs.readdirSync(workflows).filter((name) => name.endsWith(".yml"))) {
    assert.doesNotMatch(fs.readFileSync(new URL(file, workflows), "utf8"), /TINE_QUICK_SHARE_VIEWER/, file);
  }
});

test("the native app reads no build-time quick share switch", () => {
  const sources = new URL("../src-tauri/src/", import.meta.url);
  for (const file of fs.readdirSync(sources, { recursive: true }).filter((name) => String(name).endsWith(".rs"))) {
    assert.doesNotMatch(fs.readFileSync(new URL(String(file), sources), "utf8"), /TINE_QUICK_SHARE_VIEWER/, `src-tauri/src/${file}`);
  }
});
