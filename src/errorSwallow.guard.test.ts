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
const FROZEN_SWALLOW_COUNT = 26;
const ORIGINAL_SWALLOW_KEYS = new Set(`
  src/assetCache.ts:130 src/assetCache.ts:212 src/assetCache.ts:264 src/assetCache.ts:69
  src/capture.tsx:259 src/capture.tsx:555 src/components/AboutTab.tsx:17 src/components/AudioOverlay.tsx:112 src/components/AudioOverlay.tsx:149
  src/components/Block.tsx:3156 src/components/HelpShortcuts.tsx:51 src/components/LinkedReferences.tsx:42 src/components/Macro.tsx:313 src/components/PdfViewer.tsx:1032
  src/components/PdfViewer.tsx:1097 src/components/PdfViewer.tsx:535
  src/components/UnlinkedReferences.tsx:38 src/components/WindowChrome.tsx:24
  src/debug.ts:14
  src/pageIconBatch.ts:44 src/plugins/manager.ts:121
  src/plugins/startup.ts:29 src/queryResultCache.ts:55 src/session.ts:293 src/sheet/queryHydration.ts:232
  src/update.ts:61
`.trim().split(/\s+/));
const ALLOWED_SWALLOWS: Record<string, string> = {
  "src/assetCache.ts:69": "best-effort stale blob URL cleanup",
  "src/assetCache.ts:130": "best-effort stale blob URL cleanup",
  "src/assetCache.ts:212": "best-effort stale blob URL cleanup",
  "src/assetCache.ts:264": "best-effort stale blob URL cleanup",
  "src/capture.tsx:259": "legacy best-effort operation needs an error-family audit",
  "src/capture.tsx:555": "legacy error-prose branch; replace with fixed error family",
  "src/components/AboutTab.tsx:17": "legacy best-effort operation needs an error-family audit",
  "src/components/AudioOverlay.tsx:112": "legacy best-effort operation needs an error-family audit",
  "src/components/AudioOverlay.tsx:149": "legacy best-effort operation needs an error-family audit",
  "src/components/Block.tsx:3156": "association failure is intentionally a quiet feature miss (line rebased after asset guards)",
  "src/components/HelpShortcuts.tsx:51": "legacy best-effort operation needs an error-family audit",
  "src/components/LinkedReferences.tsx:42": "legacy error-prose branch; replace with fixed error family",
  "src/components/Macro.tsx:313": "legacy error-prose branch; replace with fixed error family",
  "src/components/PdfViewer.tsx:535": "best-effort viewer resource cleanup",
  "src/components/PdfViewer.tsx:1032": "best-effort viewer resource cleanup",
  "src/components/PdfViewer.tsx:1097": "best-effort viewer resource cleanup",
  "src/components/UnlinkedReferences.tsx:38": "legacy error-prose branch; replace with fixed error family",
  "src/components/WindowChrome.tsx:24": "legacy best-effort operation needs an error-family audit",
  "src/debug.ts:14": "legacy best-effort operation needs an error-family audit",
  "src/pageIconBatch.ts:44": "legacy best-effort operation needs an error-family audit",
  "src/plugins/manager.ts:121": "failed queued write still rejects to caller; catch only keeps the next queued write running",
  "src/plugins/startup.ts:29": "rejection remains on returned pluginInitialization promise; main.tsx reports it",
  "src/queryResultCache.ts:55": "legacy best-effort operation needs an error-family audit",
  "src/session.ts:293": "census #1: session persistence awaits design batch",
  "src/sheet/queryHydration.ts:232": "legacy best-effort operation needs an error-family audit",
  "src/update.ts:61": "legacy best-effort operation needs an error-family audit",
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
