import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { test } from "node:test";
import { renderComparison, root, fixturePath, publishedPath } from "./build-compare-page.mjs";

const read = (file) => fs.readFileSync(path.join(root, file), "utf8");
const data = () => JSON.parse(read("website/compare/rows.json"));
test("checked-in generated page matches data, template, evidence and package version", () => {
  assert.equal(read(fixturePath), renderComparison());
});
test("published page matches generator, or the exact pre-stable-cut baseline", () => {
  const published = read(publishedPath);
  // Temporary migration allowance: develop cannot regenerate master's public
  // website. Only this exact old page is exempt; compare:check never exempts it.
  const legacy = "3e165e9f3c6851ae3dea193f086409e973f2c3244903c92796ec1d81b9627c30";
  if (createHash("sha256").update(published).digest("hex") !== legacy) {
    assert.equal(published, renderComparison());
  }
});
test("missing local evidence and changed Guide anchors/excerpts fail generation", () => {
  for (const change of [
    (cell) => { cell.evidence.path = "scripts/e2e-missing-proof.mjs"; },
    (cell) => { cell.evidence.anchor = "missing-anchor"; },
    (cell) => { cell.evidence.contains = "missing evidence excerpt"; },
    (cell) => { cell.evidence.path = "../outside.md"; },
    (cell) => { delete cell.evidence; },
  ]) {
    const input = data();
    change(input.rows[0].tine);
    assert.throws(() => renderComparison({ data: input }));
  }
});
test("claims require reasons and are visibly marked", () => {
  const input = data();
  input.rows[0].tine = { text: "Unverified", claim: true };
  assert.throws(() => renderComparison({ data: input }), /claim reason/);
  input.rows[0].tine.reason = "Needs a runtime benchmark";
  assert.match(renderComparison({ data: input }), /Claim: Needs a runtime benchmark/);
});
test("competitor cells require primary HTTPS sources and actual calendar dates", () => {
  for (const bad of [
    { source: undefined }, { source: "https://example.org/review" },
    { source: "http://help.obsidian.md/syntax" }, { checked: undefined },
    { checked: "2026-02-30" },
  ]) {
    const input = data();
    Object.assign(input.rows[0].obsidian, bad);
    assert.throws(() => renderComparison({ data: input }));
  }
});
test("unknown cells do not imply absence; data text is escaped", () => {
  const input = data();
  input.rows[0].feature = '<script>"&';
  input.rows[0].obsidian = { unknown: true, text: "Unsupported assertion" };
  const html = renderComparison({ data: input });
  assert.match(html, /&lt;script&gt;&quot;&amp;/);
  assert.match(html, />Unknown<\/td>/);
  assert.doesNotMatch(html, /Unsupported assertion/);
});
test("duplicate row IDs cannot silently overwrite a feature", () => {
  const input = data();
  input.rows.push(input.rows[0]);
  assert.throws(() => renderComparison({ data: input }), /Duplicate/);
});
