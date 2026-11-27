#!/usr/bin/env node

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// The host's own manifest parser and byte ceilings, bundled from source so the
// checker and the app give one answer to "is this manifest valid".
const host = await (async () => {
  const bundle = await build({
    entryPoints: [path.join(REPO, "src/plugins/manifest.ts")],
    bundle: true, write: false, format: "esm", platform: "neutral", logLevel: "silent",
  });
  return import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString("base64")}`);
})();
const { parsePluginManifest, PLUGIN_MANIFEST_MAX_BYTES: MAX_MANIFEST, PLUGIN_WASM_MAX_BYTES: MAX_WASM } = host;
const LICENSES = new Set([
  "0BSD", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "MIT",
  "MPL-2.0", "LGPL-2.1-only", "LGPL-3.0-only", "GPL-2.0-only",
  "GPL-3.0-only", "AGPL-3.0-only", "Unlicense",
]);
const PORT_GAP_OPERATIONS = /^[a-z][a-z0-9.-]{2,100}$/;
const PORT_GAP_RECOMMENDATIONS = new Set([
  "ship-reduced-port", "request-api-change", "core-feature", "privileged-integration", "do-not-port",
]);

function usage() {
  console.error("Usage: node scripts/tine-plugin.mjs check <plugin-dir> [--json]");
  process.exit(2);
}

function add(report, level, code, message) {
  report[level].push({ code, message });
}

function hasExactKeys(value, required) {
  return value && typeof value === "object" && !Array.isArray(value) &&
    Object.keys(value).length === required.length && required.every((key) => Object.hasOwn(value, key));
}

function boundedText(value, max) {
  return typeof value === "string" && value.length > 0 && value.length <= max && !/[\u0000-\u001f]/.test(value);
}

function isHttpsUrl(value) {
  try {
    return new URL(value).protocol === "https:";
  } catch {
    return false;
  }
}

function checkPortGap(root, report) {
  const gapPath = path.join(root, "port-gap.json");
  if (!fs.existsSync(gapPath)) return;
  report.portGap.status = "failed";
  let gap;
  try {
    const bytes = fs.readFileSync(gapPath, "utf8");
    if (Buffer.byteLength(bytes) > MAX_MANIFEST) throw new Error("report exceeds 64 KiB");
    gap = JSON.parse(bytes);
  } catch (error) {
    add(report, "errors", "port-gap.json", `port-gap.json is invalid: ${error.message}`);
    return;
  }
  const topKeys = ["format", "source", "requestedBehavior", "supportedSubset", "blockers", "recommendation"];
  if (!hasExactKeys(gap, topKeys) || gap.format !== "tine-plugin-port-gap/v1") {
    add(report, "errors", "port-gap.envelope", "port-gap.json has unknown or missing fields, or an incompatible format");
    return;
  }
  const sourceKeys = ["ecosystem", "name", "repository", "revision"];
  if (!hasExactKeys(gap.source, sourceKeys) || !["logseq", "obsidian", "other"].includes(gap.source.ecosystem) ||
      !boundedText(gap.source.name, 120) || !boundedText(gap.source.revision, 160)) {
    add(report, "errors", "port-gap.source", "port-gap source metadata is invalid");
  }
  try {
    if (new URL(gap.source?.repository).protocol !== "https:") throw new Error();
  } catch {
    add(report, "errors", "port-gap.source", "port-gap source repository must use https");
  }
  if (!boundedText(gap.requestedBehavior, 1_000) || !Array.isArray(gap.supportedSubset) || gap.supportedSubset.length > 50 ||
      gap.supportedSubset.some((item) => !boundedText(item, 500))) {
    add(report, "errors", "port-gap.behavior", "requested behavior or supported subset is invalid");
  }
  const blockerKeys = [
    "behavior", "missingHostOperation", "whyExistingApiIsInsufficient", "proposedCapability",
    "securityConsiderations", "reuseCase", "fallback",
  ];
  if (!Array.isArray(gap.blockers) || gap.blockers.length < 1 || gap.blockers.length > 50) {
    add(report, "errors", "port-gap.blockers", "port-gap must contain 1 to 50 blockers");
  } else {
    report.portGap.blockers = gap.blockers.length;
    for (const [index, blocker] of gap.blockers.entries()) {
      if (!hasExactKeys(blocker, blockerKeys) || !boundedText(blocker.behavior, 500) ||
          !PORT_GAP_OPERATIONS.test(blocker.missingHostOperation ?? "") ||
          !PORT_GAP_OPERATIONS.test(blocker.proposedCapability ?? "") ||
          !boundedText(blocker.whyExistingApiIsInsufficient, 1_000) ||
          !boundedText(blocker.securityConsiderations, 1_000) || !boundedText(blocker.reuseCase, 1_000) ||
          !boundedText(blocker.fallback, 1_000)) {
        add(report, "errors", "port-gap.blocker", `port-gap blocker ${index} is invalid`);
      }
    }
  }
  if (!PORT_GAP_RECOMMENDATIONS.has(gap.recommendation)) {
    add(report, "errors", "port-gap.recommendation", "port-gap recommendation is invalid");
  } else {
    report.portGap.recommendation = gap.recommendation;
  }
  report.portGap.status = report.errors.some((item) => item.code.startsWith("port-gap.")) ? "failed" : "passed";
}

// A refusal's report code names the field the host's parser refused (its
// `field`, indices dropped): the per-field codes of tine-plugin-check/v1, and
// `manifest.invalid` for a field that has none. Codes only; the reason is the
// parser's message.
const FIELD_CODES = new Map([
  ["manifest", "manifest.type"], ["schemaVersion", "manifest.schema"], ["apiVersion", "manifest.api"],
  ...["id", "version", "name", "description", "author", "license", "source", "platforms", "capabilities"]
    .map((field) => [field, `manifest.${field}`]),
  ["entry", "manifest.entry-path"], ["aiDevelopment", "manifest.ai"],
  ["contributions.commands[].defaultBinding", "manifest.command-binding"],
  ["capabilities.settings.read", "manifest.settings-capability"],
  ["settings", "manifest.settings"], ["settings[]", "manifest.settings"],
  ["settings[].key", "manifest.settings-key"], ["settings[].label", "manifest.settings-text"],
  ["settings[].description", "manifest.settings-text"], ["settings[].type", "manifest.settings-type"],
  ["settings[].default", "manifest.settings-default"], ["settings[].min", "manifest.settings-bounds"],
  ["settings[].max", "manifest.settings-bounds"], ["settings[].step", "manifest.settings-bounds"],
  ["settings[].maxLength", "manifest.settings-length"],
]);
const FIELD_PATTERNS = [
  [/^contributions\.\w+\[\]\.id$/, "manifest.contribution-id"],
  [/^capabilities\./, "manifest.contribution-capability"],
  [/^settings\[\]\.choices/, "manifest.settings-choices"],
  [/^portedFrom(\.|$)/, "manifest.provenance"],
];
const UNKNOWN_FIELD_CODES = new Map([
  ["manifest", "manifest.field"], ["contributions", "manifest.contributions-field"],
  ["contributions.commands[]", "manifest.command-field"], ["portedFrom", "manifest.provenance-field"],
  ["settings[]", "manifest.settings-field"],
]);

function refusalCode({ field, unknownField }) {
  const path = field.replace(/\[\d+\]/g, "[]");
  if (unknownField) return UNKNOWN_FIELD_CODES.get(path) ?? "manifest.invalid";
  return FIELD_CODES.get(path) ?? FIELD_PATTERNS.find(([pattern]) => pattern.test(path))?.[1] ?? "manifest.invalid";
}

// Validity is the host's answer: the checker runs the parser Tine itself runs
// on install (src/plugins/manifest.ts), so a package the checker passes is one
// the app accepts. The checks below are registry policy on top of it and may
// only be stricter. The parser stops at the first refusal, so one is reported.
function checkManifest(manifest, report) {
  let parsed;
  try {
    parsed = parsePluginManifest(manifest);
  } catch (error) {
    if (typeof error?.field !== "string") throw error;
    add(report, "errors", refusalCode(error), error.message);
    return null;
  }
  if (!LICENSES.has(parsed.license)) {
    add(report, "errors", "manifest.license", "license must be a recognized registry SPDX identifier");
  }
  if (!isHttpsUrl(parsed.source)) add(report, "errors", "manifest.source", "source must be a public https URL");
  if (parsed.portedFrom && !isHttpsUrl(parsed.portedFrom.source)) {
    add(report, "errors", "manifest.provenance", "portedFrom.source must use https");
  }
  for (const [index, command] of (parsed.contributions?.commands ?? []).entries()) {
    if (command.defaultBinding !== undefined && /[\u0000-\u001f]/.test(command.defaultBinding)) {
      add(report, "errors", "manifest.command-binding", `contributions.commands[${index}].defaultBinding is invalid`);
    }
  }
  if (!parsed.aiDevelopment) add(report, "warnings", "manifest.ai", "declare aiDevelopment as none, assisted, or primary");
  return parsed;
}

function checkWasm(bytes, report) {
  if (bytes.length > MAX_WASM) {
    add(report, "errors", "wasm.size", `entry exceeds ${MAX_WASM} bytes`);
    return;
  }
  let module;
  try {
    module = new WebAssembly.Module(bytes);
  } catch {
    add(report, "errors", "wasm.invalid", "entry is not valid WebAssembly");
    return;
  }
  const imports = WebAssembly.Module.imports(module);
  const exports = WebAssembly.Module.exports(module);
  report.wasm.imports = imports;
  report.wasm.exports = exports;
  if (
    imports.length !== 1 || imports[0].module !== "env" ||
    imports[0].name !== "memory" || imports[0].kind !== "memory"
  ) {
    add(report, "errors", "wasm.imports", "entry must import exactly env.memory and no ambient APIs");
  }
  for (const required of ["tine_alloc", "tine_handle", "tine_result_len"]) {
    if (!exports.some((item) => item.name === required && item.kind === "function")) {
      add(report, "errors", "wasm.exports", `entry is missing function export ${required}`);
    }
  }
  report.wasm.bytes = bytes.length;
  report.wasm.sha256 = crypto.createHash("sha256").update(bytes).digest("hex");
}

function checkPlugin(directory) {
  const root = fs.realpathSync(directory);
  const report = {
    format: "tine-plugin-check/v1",
    checkedAt: new Date().toISOString(),
    status: "failed",
    risk: "unknown",
    plugin: null,
    wasm: { bytes: 0, sha256: null, imports: [], exports: [] },
    portGap: { status: "absent", blockers: 0, recommendation: null },
    errors: [],
    warnings: [],
  };
  const manifestPath = path.join(root, "manifest.json");
  let manifestText;
  try {
    manifestText = fs.readFileSync(manifestPath, "utf8");
  } catch {
    add(report, "errors", "manifest.missing", "manifest.json is missing");
    return report;
  }
  if (Buffer.byteLength(manifestText) > MAX_MANIFEST) {
    add(report, "errors", "manifest.size", `manifest exceeds ${MAX_MANIFEST} bytes`);
    return report;
  }
  let manifest;
  try {
    manifest = JSON.parse(manifestText);
  } catch {
    add(report, "errors", "manifest.json", "manifest.json is invalid JSON");
    return report;
  }
  report.plugin = { id: manifest?.id ?? null, version: manifest?.version ?? null, name: manifest?.name ?? null };
  const parsed = checkManifest(manifest, report);
  checkPortGap(root, report);
  if (parsed) {
    const unresolved = path.resolve(root, parsed.entry);
    const relative = path.relative(root, unresolved);
    if (relative.startsWith("..") || path.isAbsolute(relative)) {
      add(report, "errors", "entry.traversal", "entry escapes the plugin directory");
    } else {
      try {
        const entry = fs.realpathSync(unresolved);
        const realRelative = path.relative(root, entry);
        if (realRelative.startsWith("..") || path.isAbsolute(realRelative)) {
          add(report, "errors", "entry.symlink", "entry resolves outside the plugin directory");
        } else {
          checkWasm(fs.readFileSync(entry), report);
        }
      } catch {
        add(report, "errors", "entry.missing", "entry does not exist or cannot be read");
      }
    }
  }
  report.risk = parsed?.capabilities.includes("graph.write.block") ? "review" : parsed ? "low" : "unknown";
  report.status = report.errors.length === 0 ? "passed" : "failed";
  return report;
}

const [, , command, directory, ...flags] = process.argv;
if (command !== "check" || !directory) usage();
const report = checkPlugin(directory);
if (flags.includes("--json")) {
  console.log(JSON.stringify(report, null, 2));
} else {
  const id = report.plugin?.id ?? "unknown";
  const version = report.plugin?.version ?? "unknown";
  console.log(`${report.status.toUpperCase()}: ${id}@${version} (${report.risk} risk)`);
  for (const item of report.errors) console.error(`error ${item.code}: ${item.message}`);
  for (const item of report.warnings) console.warn(`warning ${item.code}: ${item.message}`);
  if (report.wasm.sha256) console.log(`sha256 ${report.wasm.sha256}`);
}
process.exitCode = report.status === "passed" ? 0 : 1;
