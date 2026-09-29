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
const PROSE_BRANCH = /(?:\b(?:message|msg|errorText|errText)|\b\w+\.message|(?<![\w$])String\s*\([^)]*\))\s*(?:\.\s*(?:startsWith|includes)\s*\(|(?:===|==|!==|!=))/;

// Historical I-9 keys from a4c46c22c. The allow-list is empty; these keys
// preserve the monotonic ratchet if an exception is ever proposed again.
const FROZEN_SWALLOW_COUNT = 0;
const ORIGINAL_SWALLOW_KEYS = new Set(`
  src/assetCache.ts:130 src/assetCache.ts:212 src/assetCache.ts:264 src/assetCache.ts:69
  src/components/AboutTab.tsx:17 src/components/AudioOverlay.tsx:117 src/components/AudioOverlay.tsx:154
  src/components/Block.tsx:3151 src/components/HelpShortcuts.tsx:51 src/components/LinkedReferences.tsx:44
  src/components/PdfViewer.tsx:492 src/components/PdfViewer.tsx:993 src/components/PdfViewer.tsx:1058
  src/capture.tsx:264 src/capture.tsx:572
  
  src/components/UnlinkedReferences.tsx:39 src/components/WindowChrome.tsx:24
  src/debug.ts:14
  src/pageIconBatch.ts:44 src/plugins/manager.ts:122
  src/plugins/startup.ts:29 src/queryResultCache.ts:55 src/session.ts:293 src/sheet/queryHydration.ts:233
  src/update.ts:61
`.trim().split(/\s+/));
const ALLOWED_SWALLOWS: Record<string, string> = {};

const NAMED_BEST_EFFORT_HELPERS: Record<string, string[]> = {
  "src/assetCache.ts": ["ignoreEvictedAssetCleanupFailure", "revokeEvictedUrl"],
  "src/components/PdfViewer.tsx": ["ignorePdfDestroyFailure", "discardPdfDocument"],
  "src/plugins/startup.ts": ["observePluginInitializationFailure"],
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
  for (const [file, helpers] of Object.entries(NAMED_BEST_EFFORT_HELPERS)) {
    const source = readFileSync(file, "utf8");
    for (const helper of helpers) {
      expect(source).toMatch(new RegExp(`function ${helper}\\(`));
      expect(source).toMatch(new RegExp(`(?:then\\([^)]*,\\s*|\\b)${helper}\\b`));
    }
  }
});

it("detects planted empty catches and error-prose branches", () => {
  expect(swallowViolations("src/planted.ts", "try {} catch {}\np.catch(() => undefined)\nif (message.startsWith('bad')) fail();\nif (e.message === 'bad') fail();"))
    .toHaveLength(4);
  expect(swallowViolations("src/capture.tsx", "if (eventToBindingString(e) === want) submit();")).toEqual([]);
});
