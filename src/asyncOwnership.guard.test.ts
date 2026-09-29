import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const RULE = "I-20: an async write or navigation must prove its captured graph and route/tab owner; exemplar src/components/ContextMenu.tsx MakeTemplate";

function section(file: string, start: string, end: string): string {
  const source = readFileSync(file, "utf8");
  const from = source.indexOf(start);
  const to = source.indexOf(end, from + start.length);
  if (from < 0 || to < 0) throw new Error(`${RULE}: missing source boundary ${file} ${start}`);
  return source.slice(from, to);
}

function check(file: string, start: string, end: string, rules: RegExp[]) {
  const body = section(file, start, end);
  for (const rule of rules) expect(body, `${RULE}: ${file} ${start} must match ${rule}`).toMatch(rule);
}

describe("async ownership guard", () => {
  it("pins graph-bound component and module continuations", () => {
    check("src/components/ContextMenu.tsx", "function MakeTemplate(", "function PageMenu(", [/graphOwner\(\)/, /readOwned\(owner, backend\(\)\.listTemplates/, /existing\.kind === "stale"\) return/]);
    check("src/components/blockGestures.ts", "export function beginDrag(", "// --- Click / drag gesture", [/captureBinding\(\)/, /stillBound\(binding\) && dragMoved/]);
    // Every panel write (Field, bool, Remove, AddRow) goes through writeOne: graph session + subject instance.
    check("src/components/PageProps.tsx", "function writeOne(", "function scopeWritable(", [/stillBound\(binding\)/, /subjectOf\(scope\) !== subject/]);
    check("src/components/WorkspaceSwitcher.tsx", "  const remove = async", "  return (", [/graphOwner\(\)/, /confirmed\.kind === "stale"/, /writeOwned\(owner, deleteWorkspace/]);
    check("src/workspaces.ts", "function enqueue<", "function cloneSession(", [/graphOwner\(\)/, /assert\(\)/, /serializeDurable\(operationQueue, owner, run\)/]);
    check("src/guide.ts", "function markGuideAnnounced(", "export function maybeShowGuideAnnouncement", [/if \(!owner\(\)\) return/, /writeOwned\(owner, backend\(\)\.setGuideAnnounced/]);
    check("src/graph.ts", "async function injectCustomCss(", "export async function switchGraph(", [/graphOwner\(\)/, /readOwned\(owner, backend\(\)\.readCustomCss\(\)\)/, /if \(!owner\(\)\) return/]);
    check("src/components/Page.tsx", "function PageSection(", "  return (\n    <div class=\"page-section\">", [/routeIntentRevision\(\)/, /backendGeneration/, /router\.activeId\(\)/]);
    check("src/inpageFind.ts", "export async function revealInPageFindMatch(", "interface TextPart", [/captureBinding\(\)/, /sameRoute\(paneRouter\(paneId\)\.route\(\), route\)/, /if \(!current\(\)\) return false/]);
    check("src/focusFullscreen.ts", "export function setFocusFullscreen(", "  return task;", [/request !== generation/, /ownsFullscreen/, /tail\.then/]);
    check("src/ui.ts", "export async function enterFocusMode(", "export function toggleTheme", [/setFocusFullscreen\(true\)/, /setFocusFullscreen\(false\)/]);
    check("src/mediaEditorSettings.ts", "export async function detectMediaEditorCommand", "export async function initMediaEditorSettings", [/latestOwner\(commandProbes, ed\.settingKey, revisionOwner\(key, currentRevision\(key\)\)\)/, /readOwned\(owner, backend\(\)\.detectMediaEditor/, /result\.kind === "stale"/, /command: mediaEditorCommand\(ed\.settingKey\), applied: false/]);
    const restore = readFileSync("src/backupRestore.ts", "utf8");
    expect(restore, `${RULE}: backup restore must retain its graph owner across confirmation and writes`).toMatch(/graphOwner\(\)[\s\S]*readOwned\(owner, backend\(\)\.confirm[\s\S]*confirmed\.kind === "stale"[\s\S]*writeOwned\(owner, backend\(\)\.restoreBackup/);
    expect(restore, `${RULE}: an old restore must not release a newer graph transition`).toMatch(/if \(transitioning && ownsTransition\(\)\) setGraphTransitioning\(false\)/);
    const session = readFileSync("src/session.ts", "utf8");
    expect(session, `${RULE}: session restore must discard a stale graph read`).toMatch(/export async function restoreSession[\s\S]*graphOwner\(\)[\s\S]*readOwned\(owner, backend\(\)\.loadSession[\s\S]*result\.kind === "stale"/);
  });

});
