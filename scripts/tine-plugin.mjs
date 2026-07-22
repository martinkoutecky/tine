#!/usr/bin/env node

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const CAPABILITIES = new Set([
  "commands.register",
  "slash-commands.register",
  "block-decorations.register",
  "graph.read.visible",
  "graph.write.block",
  "settings.read",
  "settings.write",
]);
const PLATFORMS = new Set(["desktop", "android", "ios"]);
const LICENSES = new Set([
  "0BSD", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "MIT",
  "MPL-2.0", "LGPL-2.1-only", "LGPL-3.0-only", "GPL-2.0-only",
  "GPL-3.0-only", "AGPL-3.0-only", "Unlicense",
]);
const MAX_MANIFEST = 64 * 1024;
const MAX_WASM = 8 * 1024 * 1024;
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

function rejectUnknownFields(value, allowed, report, code, where) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return;
  const unknown = Object.keys(value).find((key) => !allowed.includes(key));
  if (unknown) add(report, "errors", code, `${where} contains unknown field ${unknown}`);
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

function checkSettings(settings, capabilities, report) {
  if (settings === undefined) return;
  if (!Array.isArray(settings) || settings.length > 64) {
    add(report, "errors", "manifest.settings", "settings must be an array of at most 64 definitions");
    return;
  }
  if (!capabilities.includes("settings.read")) {
    add(report, "errors", "manifest.settings-capability", "settings require settings.read");
  }
  const seen = new Set();
  for (const [index, setting] of settings.entries()) {
    const where = `settings[${index}]`;
    if (!setting || typeof setting !== "object" || Array.isArray(setting)) {
      add(report, "errors", "manifest.settings", `${where} must be an object`);
      continue;
    }
    const common = ["key", "label", "description", "type", "default"];
    const perType = setting.type === "enum" ? ["choices"] : setting.type === "number" ? ["min", "max", "step"] :
      setting.type === "string" ? ["maxLength"] : [];
    rejectUnknownFields(setting, [...common, ...perType], report, "manifest.settings-field", where);
    if (typeof setting.key !== "string" || !/^[a-z][a-z0-9._-]{0,79}$/.test(setting.key) || seen.has(setting.key)) {
      add(report, "errors", "manifest.settings-key", `${where}.key is invalid or duplicated`);
    }
    seen.add(setting.key);
    if (typeof setting.label !== "string" || !setting.label || setting.label.length > 80 ||
        typeof setting.description !== "string" || !setting.description || setting.description.length > 300 || /[\u0000-\u001f]/.test(setting.description)) {
      add(report, "errors", "manifest.settings-text", `${where} needs bounded plain-text label and description`);
    }
    if (setting.type === "boolean") {
      if (typeof setting.default !== "boolean") add(report, "errors", "manifest.settings-default", `${where}.default must be boolean`);
    } else if (setting.type === "enum") {
      const choices = setting.choices;
      if (!Array.isArray(choices) || choices.length < 2 || choices.length > 32 ||
          !choices.some((choice) => choice?.value === setting.default)) {
        add(report, "errors", "manifest.settings-choices", `${where} needs 2 to 32 choices including its default`);
      }
    } else if (setting.type === "number") {
      if (![setting.default, setting.min, setting.max].every(Number.isFinite) || setting.min > setting.max ||
          setting.default < setting.min || setting.default > setting.max ||
          (setting.step !== undefined && (!Number.isFinite(setting.step) || setting.step <= 0))) {
        add(report, "errors", "manifest.settings-bounds", `${where} has invalid numeric bounds`);
      }
    } else if (setting.type === "string") {
      if (!Number.isInteger(setting.maxLength) || setting.maxLength < 1 || setting.maxLength > 4096 ||
          typeof setting.default !== "string" || setting.default.length > setting.maxLength) {
        add(report, "errors", "manifest.settings-length", `${where} has an invalid string bound`);
      }
    } else {
      add(report, "errors", "manifest.settings-type", `${where}.type is unsupported`);
    }
  }
}

function checkPortedFrom(portedFrom, report) {
  if (portedFrom === undefined) return;
  if (!portedFrom || typeof portedFrom !== "object" || Array.isArray(portedFrom) ||
      !["logseq", "obsidian", "other"].includes(portedFrom.ecosystem) ||
      !["behavioral-port", "source-derived"].includes(portedFrom.relationship) ||
      !Array.isArray(portedFrom.authors) || portedFrom.authors.length === 0) {
    add(report, "errors", "manifest.provenance", "portedFrom is incomplete or invalid");
    return;
  }
  rejectUnknownFields(
    portedFrom, ["ecosystem", "name", "source", "revision", "license", "authors", "relationship"],
    report, "manifest.provenance-field", "portedFrom",
  );
  for (const field of ["name", "source", "revision", "license"]) {
    if (typeof portedFrom[field] !== "string" || !portedFrom[field]) {
      add(report, "errors", "manifest.provenance", `portedFrom.${field} is required`);
    }
  }
  try {
    if (new URL(portedFrom.source).protocol !== "https:") throw new Error();
  } catch {
    add(report, "errors", "manifest.provenance", "portedFrom.source must use https");
  }
}

function checkManifest(manifest, report) {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    add(report, "errors", "manifest.type", "manifest.json must contain an object");
    return;
  }
  rejectUnknownFields(manifest, [
    "schemaVersion", "id", "name", "version", "apiVersion", "description", "author", "license",
    "source", "entry", "platforms", "capabilities", "contributions", "settings", "portedFrom", "aiDevelopment",
  ], report, "manifest.field", "manifest");
  rejectUnknownFields(
    manifest.contributions, ["commands", "slashCommands", "blockDecorations"],
    report, "manifest.contributions-field", "contributions",
  );
  for (const [index, command] of (manifest.contributions?.commands ?? []).entries()) {
    rejectUnknownFields(
      command, ["id", "title", "description", "defaultBinding", "platforms"],
      report, "manifest.command-field", `contributions.commands[${index}]`,
    );
    if (command.defaultBinding !== undefined && !boundedText(command.defaultBinding, 80)) {
      add(report, "errors", "manifest.command-binding", `contributions.commands[${index}].defaultBinding is invalid`);
    }
  }
  if (manifest.schemaVersion !== 1) add(report, "errors", "manifest.schema", "schemaVersion must be 1");
  if (manifest.apiVersion !== "0.2") add(report, "errors", "manifest.api", "apiVersion must be 0.2");
  if (typeof manifest.id !== "string" || !/^[a-z0-9](?:[a-z0-9.-]{1,62}[a-z0-9])$/.test(manifest.id) || !manifest.id.includes(".")) {
    add(report, "errors", "manifest.id", "id must be a lowercase dotted identifier");
  }
  if (typeof manifest.version !== "string" || !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?$/.test(manifest.version)) {
    add(report, "errors", "manifest.version", "version must be SemVer");
  }
  for (const field of ["name", "description", "author", "license", "source", "entry"]) {
    if (typeof manifest[field] !== "string" || !manifest[field]) add(report, "errors", `manifest.${field}`, `${field} is required`);
  }
  if (typeof manifest.source === "string") {
    try {
      const source = new URL(manifest.source);
      if (source.protocol !== "https:") throw new Error();
    } catch {
      add(report, "errors", "manifest.source", "source must be a public https URL");
    }
  }
  if (typeof manifest.license === "string" && !LICENSES.has(manifest.license)) {
    add(report, "errors", "manifest.license", "license must be a recognized registry SPDX identifier");
  }
  const platforms = manifest.platforms ?? ["desktop"];
  if (!Array.isArray(platforms) || platforms.length === 0 || platforms.some((item) => !PLATFORMS.has(item))) {
    add(report, "errors", "manifest.platforms", "platforms contains an unsupported target");
  }
  if (!Array.isArray(manifest.capabilities) || manifest.capabilities.some((item) => !CAPABILITIES.has(item))) {
    add(report, "errors", "manifest.capabilities", "capabilities contains an unsupported authority");
  }
  const capabilities = Array.isArray(manifest.capabilities) ? manifest.capabilities : [];
  checkSettings(manifest.settings, capabilities, report);
  checkPortedFrom(manifest.portedFrom, report);
  if (!manifest.aiDevelopment) {
    add(report, "warnings", "manifest.ai", "declare aiDevelopment as none, assisted, or primary");
  } else if (!["none", "assisted", "primary"].includes(manifest.aiDevelopment)) {
    add(report, "errors", "manifest.ai", "aiDevelopment is invalid");
  }
  const required = [
    [manifest.contributions?.commands, "commands.register"],
    [manifest.contributions?.slashCommands, "slash-commands.register"],
    [manifest.contributions?.blockDecorations, "block-decorations.register"],
  ];
  for (const [entries, capability] of required) {
    if (Array.isArray(entries) && entries.length > 0 && !manifest.capabilities?.includes(capability)) {
      add(report, "errors", "manifest.contribution-capability", `contribution requires ${capability}`);
    }
  }
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
  report.plugin = { id: manifest.id ?? null, version: manifest.version ?? null, name: manifest.name ?? null };
  checkManifest(manifest, report);
  checkPortGap(root, report);
  if (typeof manifest.entry === "string") {
    const unresolved = path.resolve(root, manifest.entry);
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
  const capabilities = Array.isArray(manifest.capabilities) ? manifest.capabilities : [];
  report.risk = capabilities.includes("graph.write.block") ? "review" : "low";
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
