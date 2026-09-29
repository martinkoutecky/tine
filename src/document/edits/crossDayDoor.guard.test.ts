import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// I-12 (one answerer per question): "may these root blocks cross into the
// neighbouring feed day now?" is answered once, in crossDayMove, which re-checks the
// plan after awaiting the feed extender. A second caller of crossMoveBlocks would
// be a second, drifting answerer (master eba7c56b2 H1-H5). Exemplar:
// src/document/edits/moves.ts crossDayMove.
export function crossMoveCallers(source: string): string[] {
  const out: string[] = [];
  const fns = [...source.matchAll(/^(?:export\s+)?(?:async\s+)?function\s+(\w+)\s*\(/gm)];
  fns.forEach((m, i) => {
    const body = source.slice(m.index, fns[i + 1]?.index ?? source.length);
    if (m[1] !== "crossMoveBlocks" && /\bcrossMoveBlocks\s*\(/.test(body)) out.push(m[1]);
  });
  return out;
}

describe("I-12 one door for a cross-day move", () => {
  const source = readFileSync("src/document/edits/moves.ts", "utf8");
  it("reaches crossMoveBlocks only through crossDayMove", () => {
    expect(crossMoveCallers(source)).toEqual(["crossDayMove"]);
    expect(crossMoveCallers(source.replace("export async function moveBlockFeed", "export async function planted() { crossMoveBlocks([], 'a', 'b', 1); }\nexport async function moveBlockFeed")))
      .toContain("planted");
  });
  it("both boundary movers use it", () => {
    for (const name of ["moveBlockFeed", "moveSelectionItems"]) {
      const start = source.indexOf(`function ${name}(`);
      const end = source.indexOf("\nexport ", start + 1);
      expect(source.slice(start, end < 0 ? undefined : end), name).toMatch(/\bcrossDayMove\(/);
    }
  });
});
