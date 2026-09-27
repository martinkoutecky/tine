import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

function documentSources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? documentSources(file) : /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file) ? [file] : [];
  });
}
const SCOPED = ["src/App.tsx", "src/carry.ts", "src/graph.ts", "src/ui.ts", "src/graphSession.ts",
  "src/toasts.ts", "src/components/QuickSwitcher.tsx", "src/components/QueryWorkspace.tsx",
  ...documentSources("src/document")];
const BACKEND_AWAIT = /\bawait\s+(?:backend\(\)|deps)\.(?:getPage|getPageByPath|resolvePage|savePages|deletePage|listJournalConflicts|listSyncConflicts|journalContentDays)\s*\(/;
const LANDING = /\b(?:ensurePageLoaded|reloadPage|markDirty|markConflict|forgetPage|loadSingle|openPage|openPageTarget|pushToast|bumpDataRev|bumpPageInventoryRev|set[A-Z]\w*)\s*\(/;

export function lateLandingViolations(file: string, source: string): string[] {
  const lines = source.split("\n");
  const violations: string[] = [];
  for (let start = 0; start < lines.length; start++) {
    if (!BACKEND_AWAIT.test(lines[start])) continue;
    let guarded = false;
    for (let i = start + 1; i < Math.min(lines.length, start + 19); i++) {
      const line = lines[i].replace(/\/\/.*$/, "");
      if (/\bawait\s+/.test(line)) break;
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
  it("keeps scoped backend completions guarded before editor/state landings", () => {
    for (const file of SCOPED) assertLateLandings(file, readFileSync(file, "utf8"));
  });

  it("fails a planted old-graph completion", () => {
    const planted = "async function stale() {\n const dto = await backend().getPage('P', 'page');\n reloadPage(dto);\n}";
    expect(() => assertLateLandings("src/planted.ts", planted)).toThrow(/I-20.*exemplar src\/document\/edits\/capture\.ts/s);
  });
});
