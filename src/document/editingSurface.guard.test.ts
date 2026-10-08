import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    return /\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) ? [path] : [];
  });
}

describe("document editing operations", () => {
  it("keeps every document-layer startEditing call surface-aware (GH #477)", () => {
    // A document operation that reopens the editor must be told WHICH rendering
    // of the block the caret belongs to. When it is not, `editing()` in Block.tsx
    // falls back to the non-`ref:`/`embed:` copy by design, so Tab inside a block
    // embed silently moved the caret to the source copy of the same block
    // further down the page. `indentBlock` (src/document/edits/blocks.ts) is the
    // exemplar to imitate.
    const calls = sources("src/document").flatMap((file) =>
      readFileSync(file, "utf8").split("\n")
        .map((line, index) => ({ line: line.trim(), where: `${file}:${index + 1}` }))
        .filter((entry) => entry.line.startsWith("startEditing(")),
    );
    // Non-vacuity: this guard is worthless if the scan stops finding the calls.
    expect(calls.length, "I-12: the startEditing scan of src/document found nothing").toBeGreaterThanOrEqual(5);
    for (const call of calls) {
      expect(
        call.line.split(",").length,
        `I-12: ${call.where} reopens the editor without naming a surface, so the caret leaves a block `
          + "embed for the source copy (GH #477). Take an `editingSurface` parameter and forward it as the "
          + "4th argument, like indentBlock does.",
      ).toBeGreaterThanOrEqual(4);
    }
  });
});
