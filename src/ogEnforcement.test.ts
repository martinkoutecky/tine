import { describe, expect, it } from "vitest";
import path from "node:path";
import { readFileSync } from "node:fs";
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
    expect(PINNED_FORMAT_COUNT).toBe(27);
    expect(PERSISTED_FORMATS).toEqual([
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
    expect(() => checkFormatCount(Array.from({ length: PINNED_FORMAT_COUNT + 1 }, (_, i) => `kind-${i}`))).toThrow(/Martin's approval/);
    expect(() => checkWriterSites({ "src-tauri/src/new.rs": 1 }, {})).toThrow(/Martin's approval/);
    expect(() => checkWriterSites({ "src-tauri/src/flight_store.rs": 4 }, {})).toThrow(/3 → 4 writer sites/);
    expect(writerSiteCounts("#[cfg(test)]\nmod tests {\n fs::write(foo, bar);\n}\nfs::write(path, bytes);\n")).toBe(1);
  });
});


it("I-4/I-12: edit only addressed content; imitate queryBuilder.ts edit and removeAt", () => {
  const source = (name: string) => readFileSync(path.join(root, "src", name), "utf8");
  // The tree edits live in queryTree.ts (re-exported by queryBuilder.ts): scan both.
  const query = source("editor/queryBuilder.ts") + source("editor/queryTree.ts");
  expect(query, "Query edits must retain untouched groups; queryBuilder.ts edit is the answerer (I-4/I-12)").not.toContain("normalize(");
  expect(query, "Query edits must not globally prune; imitate queryBuilder.ts removeAt (I-4)").not.toContain("children.map(prune)");
  expect(source("editor/htmlPaste.ts"), "Preserve Turndown code bytes; imitate htmlPaste.ts (I-4)").toContain("service.turndown(doc.body).trim()");
  expect(source("favorites.ts"), "Preserve disk preambles; imitate favorites.ts writeArrangementPage (I-4)").toContain("disk?.pre_block ?? markerFor(format)");
  expect(source("sheet/restructure.ts"), "Keep group content unless represented; imitate restructure.ts flatten (I-4)").toContain("if (!group.retain) deleteBlock(group.id)");
});

it("I-21: workspace resource ownership observes identity, not route replacement", () => {
  const source = readFileSync(path.join(root, "src/components/QueryWorkspace.tsx"), "utf8");
  const rule = "I-21: only workspace identity changes release the search; imitate QueryWorkspace's identity memo";
  expect(source, rule).toContain("const workspaceIdentity = createMemo(() => props.route.id);");
  expect(source, rule).toContain("createEffect(on(workspaceIdentity,");
});


it("I-12: all automatic updates use the preference-aware updater door", () => {
  const app = readFileSync(path.join(root, "src/App.tsx"), "utf8");
  const updater = readFileSync(path.join(root, "src/update.ts"), "utf8");
  const rule = "I-12: automatic scheduling/checks belong to update.ts; imitate scheduleAutomaticUpdateCheck";
  expect(app, rule).toContain("onCleanup(scheduleAutomaticUpdateCheck())");
  expect(app, rule).not.toMatch(/\bcheckForUpdate\(/);
  expect(updater, rule).toContain("await initUpdateSettings()");
  expect(updater, rule).toContain("await offerUpdate(latest, cur, checkForUpdatesAutomatically)");
});
