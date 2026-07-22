#!/usr/bin/env node

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const TOKENS = new Set([
  "--ls-active-primary-color", "--ls-primary-background-color", "--ls-secondary-background-color",
  "--ls-tertiary-background-color", "--ls-quaternary-background-color", "--ls-primary-text-color",
  "--ls-secondary-text-color", "--ls-title-text-color", "--ls-link-text-color", "--ls-link-text-hover-color",
  "--ls-tag-text-color", "--ls-border-color", "--ls-guideline-color", "--ls-block-highlight-color",
  "--ls-block-bullet-color", "--ls-selection-background-color", "--ls-a-chosen-bg",
  "--ls-page-inline-code-bg-color", "--ls-page-inline-code-color", "--ls-page-mark-bg-color", "--ls-page-mark-color",
]);
const COLOR = /^(?:#[0-9A-Fa-f]{3,8}|transparent|(?:rgb|rgba|hsl|hsla)\([0-9.,%+\- /]+\))$/;
const LICENSES = new Set(["0BSD", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "MIT", "MPL-2.0", "GPL-2.0-only", "GPL-3.0-only", "AGPL-3.0-only", "Unlicense"]);

function fail(errors, code, message) { errors.push({ code, message }); }
function checkUrl(value) { try { return typeof value === "string" && new URL(value).protocol === "https:"; } catch { return false; } }

function checkTheme(file) {
  const report = { format: "tine-theme-check/v1", checkedAt: new Date().toISOString(), status: "failed", theme: null, sha256: null, errors: [], warnings: [] };
  let bytes;
  try { bytes = fs.readFileSync(file); } catch { fail(report.errors, "theme.missing", "theme.json is missing"); return report; }
  if (bytes.length > 64 * 1024) { fail(report.errors, "theme.size", "theme.json exceeds 64 KiB"); return report; }
  report.sha256 = crypto.createHash("sha256").update(bytes).digest("hex");
  let value;
  try { value = JSON.parse(bytes.toString("utf8")); } catch { fail(report.errors, "theme.json", "theme.json is invalid JSON"); return report; }
  report.theme = { id: value?.id ?? null, version: value?.version ?? null, name: value?.name ?? null };
  if (value?.schemaVersion !== 1 || value?.apiVersion !== "0.1") fail(report.errors, "theme.api", "theme schemaVersion/apiVersion must be 1/0.1");
  if (typeof value?.id !== "string" || !/^[a-z0-9](?:[a-z0-9.-]{1,62}[a-z0-9])$/.test(value.id) || !value.id.includes(".")) fail(report.errors, "theme.id", "id must be a lowercase dotted identifier");
  if (!LICENSES.has(value?.license)) fail(report.errors, "theme.license", "license must be a recognized registry SPDX identifier");
  if (!checkUrl(value?.source)) fail(report.errors, "theme.source", "source must be a public https URL");
  if (!Array.isArray(value?.screenshots) || value.screenshots.length > 6 || value.screenshots.some((url) => !checkUrl(url))) fail(report.errors, "theme.screenshots", "screenshots must contain at most 6 public https URLs");
  const modes = value?.modes;
  if (!modes || typeof modes !== "object" || Array.isArray(modes) || (!modes.light && !modes.dark) || Object.keys(modes).some((mode) => !["light", "dark"].includes(mode))) {
    fail(report.errors, "theme.modes", "modes must contain only light and/or dark token objects");
  } else {
    for (const [mode, tokens] of Object.entries(modes)) {
      if (!tokens || typeof tokens !== "object" || Array.isArray(tokens) || Object.keys(tokens).length === 0) {
        fail(report.errors, "theme.tokens", `${mode} tokens must be a non-empty object`);
        continue;
      }
      for (const [token, color] of Object.entries(tokens)) {
        if (!TOKENS.has(token)) fail(report.errors, "theme.token", `${token} is not a host-whitelisted token`);
        if (typeof color !== "string" || !COLOR.test(color)) fail(report.errors, "theme.color", `${mode}.${token} must be a literal color`);
      }
    }
  }
  if (value?.portedFrom && (!checkUrl(value.portedFrom.source) || !value.portedFrom.revision || !Array.isArray(value.portedFrom.authors) || value.portedFrom.authors.length === 0)) {
    fail(report.errors, "theme.provenance", "portedFrom must include source, revision, and original authors");
  }
  report.status = report.errors.length === 0 ? "passed" : "failed";
  return report;
}

const [, , command, target, ...flags] = process.argv;
if (command !== "check" || !target) {
  console.error("Usage: node scripts/tine-theme.mjs check <theme-dir-or-json> [--json]");
  process.exit(2);
}
const input = fs.realpathSync(target);
const file = fs.statSync(input).isDirectory() ? path.join(input, "theme.json") : input;
const report = checkTheme(file);
if (flags.includes("--json")) console.log(JSON.stringify(report, null, 2));
else {
  console.log(`${report.status.toUpperCase()}: ${report.theme?.id ?? "unknown"}@${report.theme?.version ?? "unknown"}`);
  for (const error of report.errors) console.error(`error ${error.code}: ${error.message}`);
  if (report.sha256) console.log(`sha256 ${report.sha256}`);
}
process.exitCode = report.status === "passed" ? 0 : 1;
