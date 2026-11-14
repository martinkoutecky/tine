import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";

const read = (name) => fs.readFileSync(new URL(`../.github/workflows/${name}.yml`, import.meta.url), "utf8");
const variable = "TINE_QUICK_SHARE_VIEWER_G1_READY";
const check = "node scripts/check-release-readiness.mjs --quick-share-only";

// Workflow contract checks follow test-release-pipeline.mjs's source assertions.
// Flatpak is deliberately excluded pending a decision on offline attestation.
function job(workflow, name) {
  const body = workflow.split(`\n  ${name}:\n`)[1]?.split(/\n  [\w-]+:\n/)[0];
  assert.ok(body, `missing job ${name}`);
  return body;
}

function checkedOutput(body, label) {
  const steps = body.split(/\n      - /).slice(1);
  const checks = steps.filter((step) => step.split("\n").some((line) => line.trim() === `run: ${check}`));
  assert.equal(checks.length, 1, `${label}: run the shared readiness check exactly once`);
  const step = checks[0];
  const id = step.match(/\n        id: ([\w-]+)\n/)?.[1];
  assert.ok(id, `${label}: readiness must expose its output`);
  assert.doesNotMatch(step, /\n        if:/, `${label}: readiness runs on every build`);
  const before = steps.slice(0, steps.indexOf(step)).join("\n");
  assert.match(before, /npm ci/, `${label}: install the check's dependencies`);
  assert.match(before, /npx playwright install(?: --with-deps)? chromium/, `${label}: install the browser before readiness`);
  return { steps, step, expression: "${{ steps." + id + ".outputs.quick_share_viewer_g1_ready }}" };
}

function consumes(body, expression, label) {
  const declarations = body.match(new RegExp(`${variable}: [^\n]+`, "g")) ?? [];
  assert.ok(declarations.length, `${label}: native build needs the viewer attestation`);
  for (const declaration of declarations) {
    assert.equal(declaration, `${variable}: ${expression}`, `${label}: only the readiness output may enable shares`);
  }
}

test("release desktop and Android inherit the checked writer's attestation", () => {
  const release = read("release");
  const preflight = job(release, "preflight");
  const checked = checkedOutput(preflight, "release preflight");
  assert.ok(preflight.includes(`quick_share_viewer_g1_ready: ${checked.expression}`));
  for (const name of ["build", "android"]) {
    const body = job(release, name);
    assert.match(body, /needs: preflight\n/);
    consumes(body, "${{ needs.preflight.outputs.quick_share_viewer_g1_ready }}", `release ${name}`);
  }
});

for (const [workflow, name, command] of [
  ["ios-testflight", "testflight", /npx tauri ios build/],
  ["msix", "package", /npm run tauri build/],
]) {
  test(`${workflow} builds inherit the checked writer's attestation`, () => {
    const source = read(workflow);
    assert.match(source, /\n  workflow_dispatch:/);
    const body = job(source, name);
    const { steps, step, expression } = checkedOutput(body, workflow);
    const builds = steps.filter((candidate) => command.test(candidate));
    assert.ok(builds.length, `${workflow}: missing native build`);
    for (const build of builds) {
      assert.ok(steps.indexOf(step) < steps.indexOf(build), `${workflow}: check before compiling`);
      consumes(build, expression, workflow);
    }
    consumes(body, expression, workflow);
  });
}
