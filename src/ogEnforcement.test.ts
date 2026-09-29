import { describe, expect, it } from "vitest";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  PINNED_FORMAT_COUNT, PERSISTED_FORMATS,
  checkFormatCount, checkSizeRatchet, checkWriterSites,
  readSizeCounts, readWriterSiteCounts, writerSiteCounts,
} from "../scripts/lib/og-enforcement.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// Whole-repository scans belong to collection, outside Vitest's per-assertion
// timer; the assertions still inspect the exact source snapshot read here.
const sizeCounts = readSizeCounts(root);
const writerCounts = readWriterSiteCounts(root);

describe("og campaign enforcement", () => {
  it("ratchets production file size against the post-batch-5 baseline", () => {
    const { current, baseline } = sizeCounts;
    expect(() => checkSizeRatchet(current, baseline)).not.toThrow();
  }, 30_000);

  it("pins persisted format count and low-level writer sites", () => {
    expect(PINNED_FORMAT_COUNT).toBe(24);
    expect(PERSISTED_FORMATS).toEqual([
      "page-markdown", "page-org", "graph-config-edn", "graph-custom-css",
      "graph-assets", "asset-sidecar-edn", "asset-trash", "graph-trash",
      "device-settings-json", "graph-session-json", "workspace-registry-json",
      "backup-page-copy", "backup-config-copy", "backup-asset-copy", "backup-snapshot-json",
      "pdf-highlights-edn", "published-site", "restore-recovery",
      "plugin-package", "desktop-launcher", "debug-log",
      "diagnostic-history-jsonl", "diagnostic-session-marker", "diagnostic-report-json",
    ]);
    expect(() => checkFormatCount()).not.toThrow();
    const { current, baseline } = writerCounts;
    expect(() => checkWriterSites(current, baseline)).not.toThrow();
  }, 30_000);

  it("fails on planted shape violations", () => {
    expect(() => checkSizeRatchet({ "src/new.ts": 1501 }, {})).toThrow(/split along a seam first/);
    expect(() => checkSizeRatchet({ "src/old.ts": 1502 }, { "src/old.ts": 1501 })).toThrow(/Right shape/);
  });

  it("fails on planted format and writer violations", () => {
    expect(() => checkFormatCount(Array.from({ length: 25 }, (_, i) => `kind-${i}`))).toThrow(/Martin's approval/);
    expect(() => checkWriterSites({ "src-tauri/src/new.rs": 1 }, {})).toThrow(/Martin's approval/);
    expect(() => checkWriterSites({ "src-tauri/src/flight_store.rs": 4 }, {})).toThrow(/3 → 4 writer sites/);
    expect(writerSiteCounts("#[cfg(test)]\nmod tests {\n fs::write(foo, bar);\n}\nfs::write(path, bytes);\n")).toBe(1);
  });
});
