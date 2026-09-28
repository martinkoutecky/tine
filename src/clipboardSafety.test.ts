import { readFileSync, readdirSync } from "node:fs";
import { join, relative } from "node:path";
import { expect, it } from "vitest";

function productionFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return productionFiles(path);
    return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name) ? [path] : [];
  });
}

it("all Cut entry points use the guarded copy-before-delete path", () => {
  const sites = productionFiles(join(process.cwd(), "src")).flatMap((path) => {
    const source = readFileSync(path, "utf8");
    return [...source.matchAll(/\b(cutBlocks|cutSheetSelection)\s*\(|copyBlockOutline\s*\(\s*["']cut["']/g)]
      .map((match) => `${relative(process.cwd(), path)}:${match[1] ?? "direct cut"}`);
  }).sort();
  expect(sites, "Every Cut must await clipboard success and recheck the source; ContextMenu.tsx Cut block is the exemplar. Add new paths to this guard.").toEqual([
    "src/components/ContextMenu.tsx:cutBlocks",
    "src/cut.ts:direct cut",
    "src/cut.ts:cutBlocks",
    "src/keybindings.ts:cutBlocks",
    "src/sheet/mutations.ts:cutSheetSelection",
    "src/sheet/selection.ts:cutSheetSelection",
  ].sort());
});

it("all Cut writers keep a token even without a private payload", () => {
  for (const path of ["src/cut.ts", "src/sheet/mutations.ts"]) {
    const source = readFileSync(join(process.cwd(), path), "utf8");
    expect(source, `I-2: ${path} must check clipboardWriteRevision before deletion; src/cut.ts::cutBlocks is the exemplar`).toMatch(/clipboardWriteRevision\(\) === ownership/);
  }
  const direct = productionFiles(join(process.cwd(), "src"))
    .filter((path) => !/\/(?:clipboard|backend|mock)\.ts$/.test(path))
    .flatMap((path) => [...readFileSync(path, "utf8").matchAll(/backend\(\)\.(?:writeText|writeRich|copyImageToClipboard)\s*\(/g)]
      .map(() => relative(process.cwd(), path)));
  expect(direct, "I-2: clipboard replacements must pass through src/clipboard.ts::writeClipboardRich so every Cut token is invalidated").toEqual([]);
});
