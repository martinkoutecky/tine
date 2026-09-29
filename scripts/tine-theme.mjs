#!/usr/bin/env node

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

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
const API_VERSIONS = new Set(["0.1", "0.2"]);
const ROOT_FIELDS = new Set([
  "schemaVersion", "id", "name", "version", "apiVersion", "description", "author", "license",
  "source", "modes", "presentation", "screenshots", "portedFrom", "aiDevelopment",
]);
// Registry intake must never certify bytes Tine refuses to install. The runtime
// parser (src/themes/manifest.ts, parseProvenance) is the authority; this block
// mirrors it field for field. GH #410 was the drift: the checker asked only for
// source/revision/authors, so an `ecosystem` the app rejects passed intake.
// src/themes/themeCheckerCli.test.ts holds the one-directional contract —
// anything this checker passes, parseThemeManifest must accept. The checker may
// be STRICTER (the SPDX allow-list and the dotted id are registry policy, not
// runtime rules); it may never be laxer.
const PORTED_FROM_FIELDS = new Set([
  "ecosystem", "name", "source", "revision", "license", "authors", "relationship",
]);
const ECOSYSTEMS = new Set(["logseq", "obsidian", "other"]);
const RELATIONSHIPS = new Set(["behavioral-port", "source-derived"]);
const PRESENTATION = {
  contentTypography: new Set(["default", "editorial-serif"]),
  journalHeader: new Set(["default", "editorial"]),
  todayTaskSummary: new Set(["hidden", "compact"]),
};

function fail(errors, code, message) { errors.push({ code, message }); }
function checkUrl(value) { try { return typeof value === "string" && new URL(value).protocol === "https:"; } catch { return false; } }
/** Mirrors the runtime's `text()`: non-empty, bounded, no control characters. */
function boundedText(value, max) {
  return typeof value === "string" && value.length > 0 && value.length <= max && !/[\u0000-\u001f]/.test(value);
}

export function checkTheme(file) {
  const report = { format: "tine-theme-check/v1", checkedAt: new Date().toISOString(), status: "failed", theme: null, sha256: null, errors: [], warnings: [] };
  let bytes;
  try { bytes = fs.readFileSync(file); } catch { fail(report.errors, "theme.missing", "theme.json is missing"); return report; }
  if (bytes.length > 64 * 1024) { fail(report.errors, "theme.size", "theme.json exceeds 64 KiB"); return report; }
  report.sha256 = crypto.createHash("sha256").update(bytes).digest("hex");
  let value;
  try { value = JSON.parse(bytes.toString("utf8")); } catch { fail(report.errors, "theme.json", "theme.json is invalid JSON"); return report; }
  report.theme = { id: value?.id ?? null, version: value?.version ?? null, name: value?.name ?? null };
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    fail(report.errors, "theme.shape", "theme manifest must be an object");
    return report;
  }
  for (const key of Object.keys(value)) {
    if (!ROOT_FIELDS.has(key)) fail(report.errors, "theme.field", `${key} is not a recognized theme field`);
  }
  if (value.schemaVersion !== 1 || !API_VERSIONS.has(value.apiVersion)) fail(report.errors, "theme.api", "theme schemaVersion/apiVersion must be 1 and one of 0.1, 0.2");
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
  if (value.portedFrom !== undefined) {
    const port = value.portedFrom;
    if (!port || typeof port !== "object" || Array.isArray(port)) {
      fail(report.errors, "theme.provenance", "portedFrom must be an object");
    } else {
      for (const key of Object.keys(port)) {
        if (!PORTED_FROM_FIELDS.has(key)) {
          fail(report.errors, "theme.provenance-field", `portedFrom contains unknown field ${key}`);
        }
      }
      if (!ECOSYSTEMS.has(port.ecosystem)) fail(report.errors, "theme.provenance-ecosystem", "portedFrom.ecosystem is unsupported");
      if (!RELATIONSHIPS.has(port.relationship)) fail(report.errors, "theme.provenance-relationship", "portedFrom.relationship is unsupported");
      if (!Array.isArray(port.authors) || port.authors.length === 0 || port.authors.length > 32) {
        fail(report.errors, "theme.provenance-authors", "portedFrom.authors must contain 1 to 32 entries");
      } else if (port.authors.some((author) => !boundedText(author, 160))) {
        fail(report.errors, "theme.provenance-authors", "each portedFrom author must be bounded plain text");
      }
      if (!boundedText(port.name, 120)) fail(report.errors, "theme.provenance-name", "portedFrom.name must be bounded plain text");
      if (!checkUrl(port.source) || !boundedText(port.source, 500)) fail(report.errors, "theme.provenance-source", "portedFrom.source must be a public https URL");
      if (!boundedText(port.revision, 160)) fail(report.errors, "theme.provenance-revision", "portedFrom.revision must be bounded plain text");
      if (!boundedText(port.license, 80)) fail(report.errors, "theme.provenance-license", "portedFrom.license must be bounded plain text");
    }
  }
  if (value.presentation !== undefined) {
    if (value.apiVersion !== "0.2") {
      fail(report.errors, "theme.presentation-api", "presentation requires theme API 0.2");
    } else if (!value.presentation || typeof value.presentation !== "object" || Array.isArray(value.presentation)) {
      fail(report.errors, "theme.presentation", "presentation must be an object");
    } else {
      for (const [key, setting] of Object.entries(value.presentation)) {
        if (!Object.hasOwn(PRESENTATION, key)) {
          fail(report.errors, "theme.presentation-field", `${key} is not a host-owned presentation setting`);
        } else if (!PRESENTATION[key].has(setting)) {
          fail(report.errors, "theme.presentation-value", `${key} has an unsupported presentation value`);
        }
      }
    }
  }
  report.status = report.errors.length === 0 ? "passed" : "failed";
  return report;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
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
}
