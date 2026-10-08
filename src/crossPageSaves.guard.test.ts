import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

// Each exemption is one-page by construction; multi-page intents must call
// persistTogether in the same function that marks or flushes their pages.
const ONE_PAGE = new Map([
  ["src/document/history.ts::applyEntry", "raw replay changes one page; snapshot replay delegates all multi-page dirties to undo/redo"],
  ["src/document/edits/blocks.ts::replaceChildOrders", "callers pass orders from one page"],
  ["src/document/edits/blocks.ts::splitBlock", "both branches change the block's one page"],
  ["src/document/edits/capture.ts::captureOutlineInto", "capture writes one named destination"],
  ["src/document/edits/properties.ts::setBlockProperty", "both branches change the block's one page"],
  ["src/document/edits/properties.ts::setPageProperty", "both branches change one named page"],
  ["src/document/edits/identity.ts::ensureBlockId", "stamps one block's page"],
  ["src/document/edits/identity.ts::stampBlockId", "stamps one block's page (ensureStableBlockId body, GH #373)"],
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
    const calls = [...body.matchAll(/\b(?:markDirty|addDirty|flushPage|persistTogether)\s*\(/g)];
    const looping = /\bfor\s*\([^\n]+\)\s*(?:\{\s*)?(?:markDirty|addDirty|flushPage)\s*\(/.test(body)
      || /\bfor\s*\([^\n]+\)\s*\{\s*\n\s*(?:markDirty|addDirty|flushPage)\s*\(/.test(body);
    const cross = calls.length > 1 || looping;
    if (cross && !/\bpersistTogether\s*\(/.test(body) && !ONE_PAGE.has(`${file}::${name}`)) found.push(`${file}::${name}`);
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

  it("flags a second dirty page even after an await", () => {
    expect(crossPageSaveViolations("src/newAction.ts", "async function delayed() { markDirty('A'); await load(); markDirty('B'); }"))
      .toEqual(["src/newAction.ts::delayed"]);
  });
});
