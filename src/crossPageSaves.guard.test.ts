import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

// Inherited v0.6.5 choreography plus E-B2's destination-first Undo/Redo fix.
// A new owner must use one below-UI operation, then leave this list unchanged.
const ALLOWED = new Set([
  "src/document/history.ts::applyEntry", "src/document/history.ts::holdHistoryRemovalsUntilAdditionsLand",
  "src/document/edits/blocks.ts::replaceChildOrders", "src/document/edits/selection.ts::cycleSelectionTasks",
  "src/document/edits/selection.ts::deleteSelection", "src/document/edits/moves.ts::moveBlockInternal",
  "src/document/edits/moves.ts::moveBlock", "src/document/edits/moves.ts::crossMoveBlocks",
  "src/document/edits/moves.ts::persistCrossPage", "src/document/edits/moves.ts::moveSelectionItems",
  "src/document/edits/carry.ts::carryUnfinished", "src/carry.ts::persist",
  // These have several branches but each invocation edits one page.
  "src/document/edits/blocks.ts::splitBlock", "src/document/edits/capture.ts::captureOutlineInto",
  "src/document/edits/properties.ts::setBlockProperty", "src/document/edits/properties.ts::setPageProperty",
  "src/document/edits/identity.ts::ensureBlockId", "src/document/edits/identity.ts::ensureStableBlockId",
]);

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(file);
    return /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file) ? [file] : [];
  });
}

export function crossPageSaveViolations(file: string, source: string): string[] {
  const functions = [...source.matchAll(/^(?:export\s+)?(?:async\s+)?function\s+(\w+)\s*\(/gm)];
  const found: string[] = [];
  for (let i = 0; i < functions.length; i++) {
    const name = functions[i][1];
    const body = source.slice(functions[i].index, functions[i + 1]?.index ?? source.length);
    const calls = [...body.matchAll(/\b(?:markDirty|addDirty|flushPage|persistCrossPage)\s*\(/g)];
    const looping = /\bfor\s*\([^\n]+\)\s*(?:\{\s*)?(?:markDirty|addDirty|flushPage)\s*\(/.test(body)
      || /\bfor\s*\([^\n]+\)\s*\{\s*\n\s*(?:markDirty|addDirty|flushPage)\s*\(/.test(body);
    const cross = calls.length > 1 || looping || /\bpersistCrossPage\s*\(/.test(body);
    if (cross && !ALLOWED.has(`${file}::${name}`)) found.push(`${file}::${name}`);
  }
  return found;
}

function assertRatchet(file: string, source: string): void {
  const found = crossPageSaveViolations(file, source);
  if (found.length) throw new Error(`I-3: a new multi-page intent needs one below-UI operation; exemplar crates/tine-graph-features/src/pages.rs rename_page_expected.\n${found.join("\n")}`);
}

describe("I-3 cross-page save ratchet", () => {
  it("does not grow inherited frontend choreography", () => {
    const root = process.cwd();
    for (const absolute of sourceFiles(path.join(root, "src"))) {
      assertRatchet(path.relative(root, absolute), readFileSync(absolute, "utf8"));
    }
  });

  it("fails a planted new multi-page action", () => {
    expect(() => assertRatchet("src/newAction.ts", "function sweep(pages) { for (const page of pages) markDirty(page); }"))
      .toThrow(/I-3:.*exemplar crates\/tine-graph-features\/src\/pages\.rs/s);
  });
});
