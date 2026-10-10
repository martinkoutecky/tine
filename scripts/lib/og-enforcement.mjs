import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";

// Ratchet baseline: og after batch 5 and its one-time `cargo fmt --all` reflow
// (the ratchet landed after that batch; see og/batches/05-edit-vocabulary.md).
export const BASE = "2d0349368";
export const SIZE_LIMIT = 1500;
const roots = ["src", "src-tauri/src", "crates"];
const extensions = new Set([".rs", ".ts", ".tsx", ".css"]);

export function isProduction(file) {
  if (!roots.some((root) => file.startsWith(`${root}/`))) return false;
  if (!extensions.has(path.extname(file))) return false;
  if (/(^|\/)(tests|examples|fixtures|vendor|target|gen)\//.test(file)) return false;
  if (/(^|\/)([^/]*\.test\.[^/]+|[^/]*_tests\.rs|test_[^/]*\.rs)$/.test(file)) return false;
  return true;
}

export function filesAtBase(root) {
  return execFileSync("git", ["ls-tree", "-r", "--name-only", BASE], { cwd: root, encoding: "utf8" })
    .trim().split("\n").filter(isProduction);
}

export function currentFiles(root) {
  const found = [];
  function walk(dir) {
    if (!fs.existsSync(path.join(root, dir))) return;
    for (const entry of fs.readdirSync(path.join(root, dir), { withFileTypes: true })) {
      const rel = path.posix.join(dir, entry.name);
      if (entry.isDirectory()) {
        if (!/(^|\/)(tests|examples|fixtures|vendor|target|gen)$/.test(rel)) walk(rel);
      } else if (isProduction(rel)) found.push(rel);
    }
  }
  for (const dir of roots) walk(dir);
  return [...new Set(found)].sort();
}

// Inline Rust test modules can occur near the start of a very large file.
// Remove each complete top-level module, preserving production code after it.
export function productionSource(source) {
  return source.replace(/#\[cfg\(test\)\]\s*mod\s+\w+\s*\{[\s\S]*?^\}/gm, "");
}

/** Production lines: inline `#[cfg(test)] mod` blocks do not count. */
export function lines(source) {
  source = productionSource(source ?? "");
  if (!source) return 0;
  return source.split("\n").length - Number(source.endsWith("\n"));
}

export function checkSizeRatchet(current, baseline) {
  const failures = [];
  for (const [file, count] of Object.entries(current)) {
    const old = baseline[file];
    if (old === undefined && count > SIZE_LIMIT) failures.push(`${file}: new production file has ${count} lines (limit ${SIZE_LIMIT})`);
    if (old > SIZE_LIMIT && count > old) failures.push(`${file}: oversized production file grew ${old} → ${count} lines`);
    if (old !== undefined && old <= SIZE_LIMIT && count > SIZE_LIMIT) failures.push(`${file}: crossed ${SIZE_LIMIT} lines (${old} → ${count})`);
  }
  if (failures.length) throw new Error(`PARITY-CAMPAIGN §3 "Right shape": split along a seam first.\n${failures.join("\n")}`);
}

export function readSizeCounts(root) {
  const baseline = Object.fromEntries(filesAtBase(root).map((file) => [file, lines(execFileSync("git", ["show", `${BASE}:${file}`], { cwd: root, encoding: "utf8", maxBuffer: 32 * 1024 * 1024 }))]));
  const current = Object.fromEntries(currentFiles(root).map((file) => [file, lines(fs.readFileSync(path.join(root, file), "utf8"))]));
  return { current, baseline };
}

// A format is a durable layout, not each filename or each field within JSON.
// The census in docs/og-persisted-formats.md cites the source for every entry.
export const PERSISTED_FORMATS = Object.freeze([
  "page-markdown", "page-org", "graph-config-edn", "graph-custom-css",
  "graph-assets", "asset-sidecar-edn", "asset-trash", "graph-trash",
  "device-settings-json", "graph-session-json", "workspace-registry-json",
  "backup-page-copy", "backup-config-copy", "backup-asset-copy", "backup-snapshot-json",
  "pdf-highlights-edn", "published-site", "restore-recovery",
  "plugin-package", "desktop-launcher", "debug-log",
  "diagnostic-history-jsonl", "diagnostic-session-marker", "diagnostic-report-json",
  "concord-base-ledger",
  "draft-store-json",
  "backup-graph-text-copy",
  "launch-checkpoint",
  "graph-link-identity",
  // Martin 2026-10-10 (Rule 8 new persisted format: share-inbox); docs/adr/0073.
  "share-inbox",
]);
export const PINNED_FORMAT_COUNT = 30;
// Writer sites Martin approved after the base, each with its ADR. Only an
// approved format may add sites here; the count is exact, not a budget.
export const APPROVED_WRITER_SITES = Object.freeze({
  "src-tauri/src/flight_store.rs": { sites: 3, approval: "og QUESTIONS Q5 2026-09-29; docs/adr/0058" },
  "src-tauri/src/concord_ledger.rs": { sites: 1, approval: "og QUESTIONS Q4 2026-09-29; docs/adr/0056" },
  "src-tauri/src/drafts.rs": { sites: 1, approval: "og QUESTIONS Q6 2026-09-29; docs/adr/0061" },
  // Reuses atomic_write_new + no-replace move; replaces the two seed sites with one audited call.
  "src-tauri/src/device_io.rs": { sites: 1, approval: "og QUESTIONS Q7 2026-09-29; OG-R6 replaces two seed sites with one audited atomic_write_new call" },
  // The one checkpoint write (temp + fsync + rename + directory sync).
  "crates/tine-store/src/store/checkpoint.rs": { sites: 1, approval: "Martin 2026-10-02 (SPEC-storage §7.6); docs/adr/0070" },
  // Not a new format: the graph-text rewrite of a moved page, made crash-durable
  // by 3a40f0ca1 through the audited atomic_write_with_check, which this census
  // did not see until it learned that name (2026-10-02).
  "crates/tine-store/src/transaction/move_file.rs": { sites: 1, approval: "graph text via the audited save path; 3a40f0ca1 crash-durable rewritten moves" },
  // Compiled executable resource in a private temporary directory, removed on
  // exit; no durable layout or graph state. Uses the existing audited writer.
  // The one prepared.json write of the share inbox; producers are native
  // code outside this census, and they never write the graph.
  "src-tauri/src/share_inbox.rs": { sites: 2, approval: "Martin 2026-10-10 (Rule 8 new persisted format: share-inbox); docs/adr/0073. Second site: the commit tombstone inside that format (review round 2, manager-specified)" },
  "src-tauri/src/youtube_identity.rs": { sites: 1, approval: "Martin's OG-QBY native YouTube identity task; temporary bundled WebProcess module, no new persisted format" },
});

export function checkFormatCount(formats = PERSISTED_FORMATS) {
  if (formats.length !== PINNED_FORMAT_COUNT || new Set(formats).size !== formats.length) {
    throw new Error(`OG-RULES Rule 8: persisted-format count is pinned at ${PINNED_FORMAT_COUNT}; a new format needs an ADR and Martin's approval.`);
  }
}

// Catch new low-level writer sites, including a new format that has not yet
// been entered in the census. Existing generic writers are audited by callers
// in the census. Counts are per file, so moving a writer needs a census update.
const writePattern = /(?:\b(?:fs|std::fs|tokio::fs)::(?:write|copy|rename)\s*\(|\b(?:File::create|io::copy|std::io::copy|atomic_write|atomic_write_new|atomic_write_with_check|write_payload|write_manifest|write_all)\s*\(|\.create_new\(true\)|\.writeFile\s*\(|\b(?:localStorage|sessionStorage)\.setItem\s*\()/g;
export function writerSiteCounts(source) {
  return [...productionSource(source).matchAll(writePattern)].length;
}

export function checkWriterSites(current, baseline, approved = APPROVED_WRITER_SITES) {
  const failures = [];
  for (const [file, count] of Object.entries(current)) {
    const allowed = (baseline[file] ?? 0) + (approved[file]?.sites ?? 0);
    if (count > allowed) failures.push(`${file}: ${allowed} → ${count} writer sites`);
  }
  if (failures.length) throw new Error(`OG-RULES Rule 8: a new format needs an ADR and Martin's approval. Review new writer sites and the persisted-format census:\n${failures.join("\n")}`);
}

export function readWriterSiteCounts(root) {
  const baseline = Object.fromEntries(filesAtBase(root).map((file) => [file, writerSiteCounts(execFileSync("git", ["show", `${BASE}:${file}`], { cwd: root, encoding: "utf8", maxBuffer: 32 * 1024 * 1024 }))]));
  // QBC moved the two existing device-local width writes out of ui.ts.
  // Transfer their allowance; neither the total nor the persisted formats grow.
  baseline["src/ui.ts"] -= 2;
  baseline["src/sidebarSizing.ts"] = 2;
  // QF3b moved stage-2 verification, with its five test-fault writes of
  // simulated external edits, out of transaction.rs. Transfer their allowance.
  baseline["crates/tine-store/src/transaction.rs"] -= 5;
  baseline["crates/tine-store/src/transaction/read_checks.rs"] = 5;
  const current = Object.fromEntries(currentFiles(root).map((file) => [file, writerSiteCounts(fs.readFileSync(path.join(root, file), "utf8"))]));
  return { current, baseline };
}
