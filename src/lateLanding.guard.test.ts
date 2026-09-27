import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

function productionSources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? productionSources(file) : /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file) ? [file] : [];
  });
}
const SCOPED = productionSources("src");
const BACKEND_AWAIT = /\bawait\s+(?:backend\(\)|api|deps)\.[A-Za-z]\w*\s*\(/;
const BACKEND_THEN = /(?:backend\(\)|api|deps)\s*\.\s*[A-Za-z]\w*\s*\([\s\S]*?\)\s*\.then\s*\(/;
const LANDING = /\b(?:ensurePageLoaded|reloadPage|markDirty|markConflict|forgetPage|loadSingle|openPage|openPageTarget|pushToast|bumpDataRev|bumpPageInventoryRev|set[A-Z]\w*)\s*\(/;

// I-20 existing landings at a4c46c22c. Each entry needs its reason and
// the list may only shrink as ownership is moved behind the session door.
const FROZEN_LATE_LANDING_COUNT = 62;
const ORIGINAL_LATE_KEYS = new Set(`
  src/assetSettings.ts:41 src/capture.tsx:330 src/components/AudioOverlay.tsx:86 src/components/Block.tsx:1321 src/components/Block.tsx:1669
  src/components/Block.tsx:1695 src/components/Block.tsx:1715 src/components/ContextMenu.tsx:660 src/components/ContextMenu.tsx:762 src/components/ContextMenu.tsx:781
  src/components/LinkedReferences.tsx:114 src/components/LiveRefGroup.tsx:72 src/components/PdfViewer.tsx:1003 src/components/Settings.tsx:1044 src/components/Settings.tsx:1641
  src/components/Settings.tsx:1867 src/components/Settings.tsx:1884 src/components/Settings.tsx:192 src/components/Settings.tsx:2188 src/components/Settings.tsx:2384
  src/components/Settings.tsx:2500 src/components/Settings.tsx:2605 src/components/Settings.tsx:2697 src/components/Settings.tsx:2706 src/components/Settings.tsx:2719
  src/components/Settings.tsx:2728 src/components/Settings.tsx:2748 src/components/Settings.tsx:559 src/components/Sidebar.tsx:344 src/components/Sidebar.tsx:413
  src/components/UnlinkedReferences.tsx:53 src/debug.ts:38 src/editor/linkDefault.ts:58 src/filedrop.ts:84 src/graph.ts:269
  src/graph.ts:313 src/graph.ts:53 src/graph.ts:71 src/guide.ts:75 src/launcherRanking.ts:43
  src/mediaEditorSettings.ts:42 src/mediaEditorSettings.ts:57 src/nativeChrome.ts:68 src/nativeChrome.ts:81 src/pageIconBatch.ts:42
  src/pageIndex.ts:71 src/plugins/manager.ts:267 src/plugins/manager.ts:273 src/plugins/manager.ts:784 src/plugins/registry.ts:455
  src/plugins/registry.ts:513 src/print.ts:111 src/render/inline.tsx:1241 src/render/inline.tsx:794 src/router.ts:733
  src/session.ts:286 src/session.ts:302 src/sheet/queryHydration.ts:325 src/spellcheckSettings.ts:88 src/themes/manager.ts:50
  src/ui.ts:239 src/workspaces.ts:96
`.trim().split(/\s+/));
const ALLOWED_LATE_LANDINGS: Record<string, string> = {
  "src/assetSettings.ts:41": "legacy settings UI result needs a binding audit",
  "src/capture.tsx:330": "legacy UI continuation needs a binding audit",
  "src/components/AudioOverlay.tsx:86": "legacy UI continuation needs a binding audit",
  "src/components/Block.tsx:1321": "legacy block UI result needs a binding audit (line rebased after asset guards)",
  "src/components/Block.tsx:1669": "census #2: photo capture failure toast (line rebased after asset guards)",
  "src/components/Block.tsx:1695": "census #2: recording stop failure toast (line rebased after asset guards)",
  "src/components/Block.tsx:1715": "census #2: recording start failure toast (line rebased after asset guards)",
  "src/components/ContextMenu.tsx:660": "legacy context-menu continuation needs a binding audit",
  "src/components/ContextMenu.tsx:762": "legacy context-menu continuation needs a binding audit",
  "src/components/ContextMenu.tsx:781": "legacy context-menu continuation needs a binding audit",
  "src/components/LinkedReferences.tsx:114": "legacy UI continuation needs a binding audit",
  "src/components/LiveRefGroup.tsx:72": "legacy UI continuation needs a binding audit",
  "src/components/PdfViewer.tsx:1003": "census #2: PDF write intent awaits its design batch",
  "src/components/Settings.tsx:192": "legacy settings UI result needs a binding audit",
  "src/components/Settings.tsx:559": "legacy settings UI result needs a binding audit",
  "src/components/Settings.tsx:1044": "legacy settings UI result needs a binding audit",
  "src/components/Settings.tsx:1641": "legacy settings preference result needs a binding audit",
  "src/components/Settings.tsx:1867": "census #2: graph maintenance intent awaits its design batch",
  "src/components/Settings.tsx:1884": "census #2: graph maintenance intent awaits its design batch",
  "src/components/Settings.tsx:2188": "census #2: graph maintenance intent awaits its design batch",
  "src/components/Settings.tsx:2384": "census #2: graph maintenance intent awaits its design batch",
  "src/components/Settings.tsx:2500": "legacy watch-mode preference result needs a binding audit",
  "src/components/Settings.tsx:2605": "legacy settings UI result needs a binding audit",
  "src/components/Settings.tsx:2697": "census #2: graph maintenance intent awaits its design batch",
  "src/components/Settings.tsx:2706": "census #2: graph maintenance intent awaits its design batch",
  "src/components/Settings.tsx:2719": "census #2: graph maintenance intent awaits its design batch",
  "src/components/Settings.tsx:2728": "census #2: graph maintenance intent awaits its design batch",
  "src/components/Settings.tsx:2748": "census #2: graph maintenance intent awaits its design batch",
  "src/components/Sidebar.tsx:344": "legacy UI continuation needs a binding audit",
  "src/components/Sidebar.tsx:413": "legacy sidebar navigation needs a binding audit",
  "src/components/UnlinkedReferences.tsx:53": "legacy UI continuation needs a binding audit",
  "src/debug.ts:38": "debug reporting is best effort",
  "src/editor/linkDefault.ts:58": "legacy UI continuation needs a binding audit",
  "src/filedrop.ts:84": "census #2: CSV/TSV size refusal toast (line rebased after drop guard)",
  "src/graph.ts:53": "census #1: graph-session continuation awaits its design batch",
  "src/graph.ts:71": "census #1: graph-session continuation awaits its design batch",
  "src/graph.ts:269": "census #1: graph-session continuation awaits its design batch",
  "src/graph.ts:313": "census #1: graph-session continuation awaits its design batch",
  "src/guide.ts:75": "census #2: guide write intent awaits its design batch",
  "src/launcherRanking.ts:43": "legacy UI continuation needs a binding audit",
  "src/mediaEditorSettings.ts:42": "legacy settings UI result needs a binding audit",
  "src/mediaEditorSettings.ts:57": "legacy settings UI result needs a binding audit",
  "src/nativeChrome.ts:68": "legacy UI continuation needs a binding audit",
  "src/nativeChrome.ts:81": "legacy UI continuation needs a binding audit",
  "src/pageIndex.ts:71": "legacy UI continuation needs a binding audit",
  "src/pageIconBatch.ts:42": "legacy page-icon read result needs a binding audit",
  "src/plugins/manager.ts:267": "legacy plugin completion needs an ownership audit",
  "src/plugins/manager.ts:273": "legacy plugin completion needs an ownership audit",
  "src/plugins/manager.ts:784": "legacy plugin completion needs an ownership audit",
  "src/plugins/registry.ts:455": "legacy plugin completion needs an ownership audit",
  "src/plugins/registry.ts:513": "legacy plugin completion needs an ownership audit",
  "src/print.ts:111": "legacy UI continuation needs a binding audit",
  "src/render/inline.tsx:794": "census #2: asset write intent awaits its design batch",
  "src/render/inline.tsx:1241": "legacy PDF navigation needs a binding audit",
  "src/router.ts:733": "legacy UI continuation needs a binding audit",
  "src/session.ts:286": "census #1: graph-session continuation awaits its design batch",
  "src/session.ts:302": "census #1: graph-session continuation awaits its design batch",
  "src/sheet/queryHydration.ts:325": "legacy UI continuation needs a binding audit",
  "src/spellcheckSettings.ts:88": "legacy settings UI result needs a binding audit",
  "src/themes/manager.ts:50": "legacy UI continuation needs a binding audit",
  "src/ui.ts:239": "census #2: config write intent awaits its design batch",
  "src/workspaces.ts:96": "legacy UI continuation needs a binding audit",
};

export function lateLandingViolations(file: string, source: string): string[] {
  const lines = source.split("\n");
  const violations: string[] = [];
  for (let start = 0; start < lines.length; start++) {
    const window = lines.slice(start, start + 6).join("\n");
    const chained = /(?:backend\(\)|\bapi\b|\bdeps\b)/.test(lines[start]) && BACKEND_THEN.test(window);
    if (!BACKEND_AWAIT.test(lines[start]) && !chained) continue;
    const landingStart = chained ? start + window.slice(0, window.search(/\.then\s*\(/)).split("\n").length - 1 : start + 1;
    let guarded = false;
    for (let i = landingStart; i < Math.min(lines.length, landingStart + 18); i++) {
      const line = lines[i].replace(/\/\/.*$/, "");
      if (/\bawait\s+/.test(line) && !chained) break;
      if (/\bstillBound\s*\(/.test(line)) guarded = true;
      if (LANDING.test(line)) {
        if (!guarded) violations.push(`${file}:${i + 1}: unguarded landing after backend await at line ${start + 1}`);
        break;
      }
    }
  }
  return violations;
}

function assertLateLandings(file: string, source: string): void {
  const violations = lateLandingViolations(file, source);
  if (violations.length) throw new Error(
    `I-20: a late backend result must prove its graph binding before a state/write landing; exemplar src/document/edits/capture.ts captureOutlineInto.\n${violations.join("\n")}`
  );
}

describe("I-20 late landing scan", () => {
  it("keeps all production backend completions guarded before editor/state landings", () => {
    const violations = SCOPED.flatMap((file) => lateLandingViolations(file, readFileSync(file, "utf8")));
    expect(Object.keys(ALLOWED_LATE_LANDINGS).length).toBeLessThanOrEqual(FROZEN_LATE_LANDING_COUNT);
    expect(Object.keys(ALLOWED_LATE_LANDINGS).filter((key) => !ORIGINAL_LATE_KEYS.has(key)),
      "I-20: allow-list may only shrink; exemplar src/document/edits/capture.ts captureOutlineInto").toEqual([]);
    expect(violations.filter((violation) => !ALLOWED_LATE_LANDINGS[violation.split(": unguarded")[0]]),
      "I-20: late backend results must prove binding; exemplar src/document/edits/capture.ts captureOutlineInto").toEqual([]);
    expect(Object.keys(ALLOWED_LATE_LANDINGS).filter((key) => !violations.some((violation) => violation.startsWith(key + ":"))),
      "I-20: remove stale allow-list entries").toEqual([]);
  });

  it("fails a planted old-graph completion", () => {
    const planted = "async function stale() {\n const dto = await backend().getPage('P', 'page');\n reloadPage(dto);\n}";
    expect(() => assertLateLandings("src/planted.ts", planted)).toThrow(/I-20.*exemplar src\/document\/edits\/capture\.ts/s);
  });
  it("detects any api method and then chain", () => {
    const planted = "api.someNewMethod().then((dto) => {\n setPage(dto);\n});";
    expect(lateLandingViolations("src/planted.ts", planted)).toHaveLength(1);
    const multiline = "backend().someNewMethod()\n  .then((dto) => {\n    setPage(dto);\n  });";
    expect(lateLandingViolations("src/planted.ts", multiline)).toHaveLength(1);
  });
});
