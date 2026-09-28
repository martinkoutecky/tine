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
    check("src/components/ContextMenu.tsx", "function MakeTemplate(", "function PageMenu(", [/captureBinding\(\)/, /await backend\(\)\.listTemplates/, /if \(!stillBound\(binding\)\) return/]);
    check("src/components/Block.tsx", "function beginDrag(", "export interface CaptureApi", [/captureBinding\(\)/, /stillBound\(binding\) && dragMoved/]);
    check("src/components/PageProps.tsx", "function Field(", "  return (\n    <div class=\"pp-field\">", [/stillBound\(props\.binding\)/]);
    check("src/components/WorkspaceSwitcher.tsx", "  const remove = async", "  return (", [/captureBinding\(\)/, /if \(!stillBound\(binding\)\) return/]);
    check("src/workspaces.ts", "function enqueue<", "function cloneSession(", [/captureBinding\(\)/, /assert\(\)/, /await promise/]);
    check("src/guide.ts", "function markGuideAnnounced(", "export function maybeShowGuideAnnouncement", [/if \(!stillBound\(binding\)\) return/]);
    check("src/graph.ts", "async function injectCustomCss(", "/** Pick a folder", [/captureBinding\(\)/, /if \(!stillBound\(binding\)\) return/]);
    check("src/components/Page.tsx", "function PageSection(", "  return (\n    <div class=\"page-section\">", [/routeIntentRevision\(\)/, /backendGeneration/, /router\.activeId\(\)/]);
    check("src/inpageFind.ts", "export async function revealInPageFindMatch(", "interface TextPart", [/captureBinding\(\)/, /sameRoute\(paneRouter\(paneId\)\.route\(\), route\)/, /if \(!current\(\)\) return false/]);
    check("src/focusFullscreen.ts", "export function setFocusFullscreen(", "  return task;", [/request !== generation/, /ownsFullscreen/, /tail\.then/]);
    check("src/ui.ts", "export async function enterFocusMode(", "export function toggleTheme", [/setFocusFullscreen\(true\)/, /setFocusFullscreen\(false\)/]);
    check("src/mediaEditorSettings.ts", "export async function detectMediaEditorCommand", "export async function initMediaEditorSettings", [/latestOwner\(commandProbes, ed\.settingKey, revisionOwner\(key, currentRevision\(key\)\)\)/, /readOwned\(owner, backend\(\)\.detectMediaEditor/, /result\.kind === "stale"/, /command: mediaEditorCommand\(ed\.settingKey\), applied: false/]);
    const restore = readFileSync("src/backupRestore.ts", "utf8");
    expect(restore, `${RULE}: backup restore must retain its graph owner across confirmation and writes`).toMatch(/captureBinding\(\)[\s\S]*backend\(\)\.confirm[\s\S]*if \(!stillBound\(binding\)\) return;[\s\S]*restoreBackup/);
    expect(restore, `${RULE}: an old restore must not release a newer graph transition`).toMatch(/if \(ownsTransition\(\)\) setGraphTransitioning\(false\)/);
    const session = readFileSync("src/session.ts", "utf8");
    expect(session, `${RULE}: session restore must discard a stale graph read`).toMatch(/export async function restoreSession[\s\S]*captureBinding\(\)[\s\S]*loadSession[\s\S]*stillBound\(binding\)/);
  });

});
