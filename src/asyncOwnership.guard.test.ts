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

  it("owns capture delivery, acknowledgement, page targets and media gestures", () => {
    check("src/App.tsx", "export async function installQuickCaptureReceiver(", "export function App(", [
      /bindingGeneration !== backend\(\)\.graphBindingGeneration\(\)/,
      /graphOwner\(owner\)/, /writeOwned\(saveOwner/,
      /readOwnedResource\(owner, listen/, /if \(!owner\(\) \|\| e\.payload\?\.target/,
    ]);
    check("src/capture.tsx", "  const scratchMarkdown =", "  const captureApi:", [
      /const bindingGeneration = backend\(\)\.graphBindingGeneration\(\)/,
      /payload: \{ id, target, bindingGeneration/,
      /scratchRevision !== submittedRevision/, /scratchMarkdown\(\) !== submittedScratch/, /title\(\) !== submittedTitle/,
      /if \(pendingCapture === pending\) scheduleTimeout\(\)/,
    ]);
    check("src/components/ContextMenu.tsx", "function PageMenu(", "export function deletePageMenuLabel(", [
      /const path = props\.path \?\? openedPage\?\.id/,
      /deletePage\(name, kind, captured\.path/,
    ]);
    const menu = section("src/components/ContextMenu.tsx", '<Match when={m().kind === "page"}>', '<Match when={m().kind === "sheet"}>');
    expect(menu, `${RULE}: each newly opened PageMenu owns its own snapshot`).toMatch(/<Show when=\{m\(\)\} keyed>/);
    expect(menu, `${RULE}: PageMenu must retain the opened file path; exemplar PageMenu.remove`).toMatch(/path=\{/);
    const inline = readFileSync("src/render/inline.tsx", "utf8");
    expect([...inline.matchAll(/const onGripDown = mediaResizeGrip\(/g)], `${RULE}: image and video use one owned gesture; exemplar mediaResizeGrip`).toHaveLength(2);
    check("src/render/inline.tsx", "function mediaResizeGrip(", "// Image embed:", [
      /graphOwner\(\(\) => alive && docNode\(id\) === original\)/,
      /onCleanup\(\(\) => \{ alive = false; cancel\(\); \}\)/,
      /next\.pointerId !== event\.pointerId/,
      /removeEventListener\("pointercancel", cancelled\)/,
      /removeEventListener\("lostpointercapture", cancelled\)/,
    ]);
    expect([...inline.matchAll(/window\.addEventListener\("pointer(?:move|up)"/g)], `${RULE}: media listeners belong to mediaResizeGrip, never component copies`).toHaveLength(2);
    const save = readFileSync("src/document/save/engine.ts", "utf8");
    expect([...save.matchAll(/reloadDisposition\((?:target\.)?owner\.name\) !== "reload"/g)],
      `${RULE}: alias owner replacement must ask reloadDisposition before and after single/group saves; exemplar runGroup`).toHaveLength(4);
    expect(save, `${RULE}: no alias-owner busy-check copies; exemplar reloadDisposition`).not.toMatch(/is(?:Dirty|Saving|Conflicted)\((?:target\.)?owner\.name\)/);
  });

});
