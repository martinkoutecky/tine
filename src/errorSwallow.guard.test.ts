import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { expect, it } from "vitest";

function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? sources(file) : /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file) ? [file] : [];
  });
}

const SWALLOW = /\bcatch\s*\{\s*\}|\.catch\s*\(\s*\(\s*\)\s*=>\s*(?:\{\s*\}|undefined)\s*\)/;
const PROSE_BRANCH = /(?:\b(?:message|msg|errorText|errText)|\b\w+\.message|String\s*\([^)]*\))\s*(?:\.\s*(?:startsWith|includes)\s*\(|(?:===|==|!==|!=))/;

// Existing I-9 exceptions at a4c46c22c. Every entry is annotated; this
// inventory and its frozen count may only shrink.
const FROZEN_SWALLOW_COUNT = 65;
const ORIGINAL_SWALLOW_KEYS = new Set(`
  src/assetCache.ts:130 src/assetCache.ts:212 src/assetCache.ts:264 src/assetCache.ts:69 src/assetSettings.ts:34
  src/capture.tsx:259 src/capture.tsx:555 src/components/AboutTab.tsx:17 src/components/AudioOverlay.tsx:112 src/components/AudioOverlay.tsx:149
  src/components/Block.tsx:3157 src/components/HelpShortcuts.tsx:51 src/components/LinkedReferences.tsx:42 src/components/Macro.tsx:313 src/components/PdfViewer.tsx:1032
  src/components/PdfViewer.tsx:1097 src/components/PdfViewer.tsx:535 src/components/Settings.tsx:1638 src/components/Settings.tsx:1642 src/components/Settings.tsx:2501
  src/components/Settings.tsx:2505 src/components/Settings.tsx:933 src/components/Settings.tsx:938 src/components/UnlinkedReferences.tsx:38 src/components/WindowChrome.tsx:24
  src/copySettings.ts:36 src/copySettings.ts:41 src/copySettings.ts:45 src/debug.ts:14 src/editor/linkDefault.ts:33
  src/editor/linkDefault.ts:58 src/editor/linkDefault.ts:70 src/guide.ts:93 src/launcherRanking.ts:49 src/localFileSettings.ts:19
  src/mediaEditorSettings.ts:25 src/navSettings.ts:17 src/pageIconBatch.ts:44 src/plugins/manager.ts:121 src/plugins/registry.ts:639
  src/plugins/startup.ts:29 src/queryResultCache.ts:55 src/refCompletionSettings.ts:25 src/session.ts:289 src/sheet/queryHydration.ts:232
  src/smoothScroll.ts:63 src/spellcheckSettings.ts:41 src/spellcheckSettings.ts:46 src/spellcheckSettings.ts:52 src/themeGallery.ts:60
  src/themes/manager.ts:25 src/themes/manager.ts:49 src/ui.ts:207 src/ui.ts:216 src/ui.ts:242
  src/ui.ts:34 src/ui.ts:44 src/ui.ts:526 src/ui.ts:59 src/ui.ts:684
  src/ui.ts:70 src/ui.ts:84 src/ui.ts:98 src/update.ts:61 src/zoom.ts:75
`.trim().split(/\s+/));
const ALLOWED_SWALLOWS: Record<string, string> = {
  "src/assetCache.ts:69": "best-effort stale blob URL cleanup",
  "src/assetCache.ts:130": "best-effort stale blob URL cleanup",
  "src/assetCache.ts:212": "best-effort stale blob URL cleanup",
  "src/assetCache.ts:264": "best-effort stale blob URL cleanup",
  "src/assetSettings.ts:34": "legacy optimistic preference write; error UI needs design",
  "src/capture.tsx:259": "legacy best-effort operation needs an error-family audit",
  "src/capture.tsx:555": "legacy error-prose branch; replace with fixed error family",
  "src/components/AboutTab.tsx:17": "legacy best-effort operation needs an error-family audit",
  "src/components/AudioOverlay.tsx:112": "legacy best-effort operation needs an error-family audit",
  "src/components/AudioOverlay.tsx:149": "legacy best-effort operation needs an error-family audit",
  "src/components/Block.tsx:3157": "association failure is intentionally a quiet feature miss",
  "src/components/HelpShortcuts.tsx:51": "legacy best-effort operation needs an error-family audit",
  "src/components/LinkedReferences.tsx:42": "legacy error-prose branch; replace with fixed error family",
  "src/components/Macro.tsx:313": "legacy error-prose branch; replace with fixed error family",
  "src/components/PdfViewer.tsx:535": "best-effort viewer resource cleanup",
  "src/components/PdfViewer.tsx:1032": "best-effort viewer resource cleanup",
  "src/components/PdfViewer.tsx:1097": "best-effort viewer resource cleanup",
  "src/components/Settings.tsx:933": "legacy optimistic preference write; error UI needs design",
  "src/components/Settings.tsx:938": "legacy optimistic preference write; error UI needs design",
  "src/components/Settings.tsx:1638": "legacy optimistic preference write; error UI needs design",
  "src/components/Settings.tsx:1642": "legacy optimistic preference write; error UI needs design",
  "src/components/Settings.tsx:2501": "legacy optimistic preference write; error UI needs design",
  "src/components/Settings.tsx:2505": "legacy optimistic preference write; error UI needs design",
  "src/components/UnlinkedReferences.tsx:38": "legacy error-prose branch; replace with fixed error family",
  "src/components/WindowChrome.tsx:24": "legacy best-effort operation needs an error-family audit",
  "src/copySettings.ts:36": "legacy optimistic preference write; error UI needs design",
  "src/copySettings.ts:41": "legacy optimistic preference write; error UI needs design",
  "src/copySettings.ts:45": "legacy optimistic preference write; error UI needs design",
  "src/debug.ts:14": "legacy best-effort operation needs an error-family audit",
  "src/editor/linkDefault.ts:33": "legacy optimistic preference write; error UI needs design",
  "src/editor/linkDefault.ts:58": "legacy optimistic preference write; error UI needs design",
  "src/editor/linkDefault.ts:70": "legacy optimistic preference write; error UI needs design",
  "src/guide.ts:93": "census #2: config write intent awaits design batch",
  "src/launcherRanking.ts:49": "legacy optimistic preference write; error UI needs design",
  "src/localFileSettings.ts:19": "legacy optimistic preference write; error UI needs design",
  "src/mediaEditorSettings.ts:25": "legacy optimistic preference write; error UI needs design",
  "src/navSettings.ts:17": "legacy optimistic preference write; error UI needs design",
  "src/pageIconBatch.ts:44": "legacy best-effort operation needs an error-family audit",
  "src/plugins/manager.ts:121": "legacy plugin queue or audit write; error UI needs design",
  "src/plugins/registry.ts:639": "legacy plugin queue or audit write; error UI needs design",
  "src/plugins/startup.ts:29": "legacy plugin queue or audit write; error UI needs design",
  "src/queryResultCache.ts:55": "legacy best-effort operation needs an error-family audit",
  "src/refCompletionSettings.ts:25": "legacy optimistic preference write; error UI needs design",
  "src/session.ts:289": "census #1: session persistence awaits design batch",
  "src/sheet/queryHydration.ts:232": "legacy best-effort operation needs an error-family audit",
  "src/smoothScroll.ts:63": "legacy optimistic preference write; error UI needs design",
  "src/spellcheckSettings.ts:41": "legacy optimistic preference write; error UI needs design",
  "src/spellcheckSettings.ts:46": "legacy optimistic preference write; error UI needs design",
  "src/spellcheckSettings.ts:52": "legacy optimistic preference write; error UI needs design",
  "src/themeGallery.ts:60": "legacy optimistic preference write; error UI needs design",
  "src/themes/manager.ts:25": "legacy best-effort operation needs an error-family audit",
  "src/themes/manager.ts:49": "legacy best-effort operation needs an error-family audit",
  "src/ui.ts:34": "legacy optimistic preference write; error UI needs design",
  "src/ui.ts:44": "legacy optimistic preference write; error UI needs design",
  "src/ui.ts:59": "legacy optimistic preference write; error UI needs design",
  "src/ui.ts:70": "legacy optimistic preference write; error UI needs design",
  "src/ui.ts:84": "legacy optimistic preference write; error UI needs design",
  "src/ui.ts:98": "legacy optimistic preference write; error UI needs design",
  "src/ui.ts:207": "legacy optimistic preference write; error UI needs design",
  "src/ui.ts:216": "legacy optimistic preference write; error UI needs design",
  "src/ui.ts:242": "legacy optimistic preference write; error UI needs design",
  "src/ui.ts:526": "legacy optimistic preference write; error UI needs design",
  "src/ui.ts:684": "legacy optimistic preference write; error UI needs design",
  "src/update.ts:61": "legacy best-effort operation needs an error-family audit",
  "src/zoom.ts:75": "legacy best-effort operation needs an error-family audit",
};

export function swallowViolations(file: string, source: string): string[] {
  return source.split("\n").flatMap((line, index) =>
    SWALLOW.test(line) || PROSE_BRANCH.test(line) ? [`${file}:${index + 1}`] : []);
}

it("I-9 ratchets swallowed errors and prose branches; exemplar src/document/save/engine.ts doSave", () => {
  const found = sources("src").flatMap((file) => swallowViolations(file, readFileSync(file, "utf8")));
  expect(Object.keys(ALLOWED_SWALLOWS).length).toBeLessThanOrEqual(FROZEN_SWALLOW_COUNT);
  expect(Object.keys(ALLOWED_SWALLOWS).filter((key) => !ORIGINAL_SWALLOW_KEYS.has(key)),
    "I-9: allow-list may only shrink; exemplar src/document/save/engine.ts doSave").toEqual([]);
  expect(found.filter((key) => !ALLOWED_SWALLOWS[key]),
    "I-9: new swallowed error or prose branch; exemplar src/document/save/engine.ts doSave").toEqual([]);
  expect(Object.keys(ALLOWED_SWALLOWS).filter((key) => !found.includes(key)),
    "I-9: remove stale allow-list entries").toEqual([]);
});

it("detects planted empty catches and error-prose branches", () => {
  expect(swallowViolations("src/planted.ts", "try {} catch {}\np.catch(() => undefined)\nif (message.startsWith('bad')) fail();\nif (e.message === 'bad') fail();"))
    .toHaveLength(4);
});
