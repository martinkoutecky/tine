// M1c: SCHEDULED/DEADLINE "type anywhere, normalize on exit".
//
// While editing you may type a `SCHEDULED:`/`DEADLINE:` line wherever it's
// convenient. When the block is EXITED, a REAL planning line is moved to its
// canonical position — right after the first content line, before any property
// lines (OG's block layout, confirmed by Tine's verified org round-trip fixture
// `crates/tine-core/src/org.rs`: `first line → SCHEDULED → DEADLINE → properties`).
//
// "Real" is decided by lsdoc: only a planning line that lsdoc parses as a
// `Timestamp` is moved, so a `SCHEDULED: <…>` inside inline code or a fenced block
// is content and is left exactly where it is (the same robustness the render path
// gets). Operates on the editor-VISIBLE text; hidden `id::`/`collapsed::` are split
// out before and reattached after (joinProps), so they stay at the very end.
import { parseBody } from "../render/facets";
import type { Format } from "../render/ast";
import { literalBlockOfLine } from "./literalLines";

const PLANNING_LINE = /^\s*(SCHEDULED|DEADLINE):\s*<[^>]+>\s*$/;

export function normalizePlanning(visible: string, format: Format): string {
  // Cheap exits: nothing planning-shaped, or a single line (nowhere to move it).
  if (!/SCHEDULED:|DEADLINE:/.test(visible)) return visible;
  const lines = visible.split("\n");
  if (lines.length < 2) return visible;

  // Gate on lsdoc: only reorder when a real `Scheduled`/`Deadline` Timestamp exists
  // (so an inline-code / fenced `SCHEDULED:` is never moved). One parse, on exit.
  const blocks = parseBody(visible, format);
  const hasTs = blocks.some(
    (b) =>
      "inline" in b &&
      Array.isArray(b.inline) &&
      b.inline.some((i) => i.k === "timestamp" && (i.ts === "Scheduled" || i.ts === "Deadline"))
  );
  if (!hasTs) return visible;

  // Pull out standalone planning lines, keep everything else in order. A line of
  // a code/src/example block is content, never a planning line; and when the
  // first line opens such a block there is no title to anchor after, so nothing
  // moves (inserting after line 0 would put the schedule INTO the code; C3 L14).
  const literal = literalBlockOfLine(visible, format);
  if (literal[0] !== -1) return visible;
  const planning: string[] = [];
  const kept: string[] = [];
  for (const [i, line] of lines.entries()) {
    if (literal[i] === -1 && PLANNING_LINE.test(line)) {
      planning.push(line.trim());
      continue;
    }
    kept.push(line);
  }
  if (planning.length === 0) return visible;
  // A block that is ONLY planning lines has no first content line to anchor after —
  // leave it as-is (else `kept[0]` is undefined and the join prepends a blank line).
  if (kept.length === 0) return visible;

  // Canonical order: SCHEDULED before DEADLINE.
  planning.sort((a, b) => (a.startsWith("SCHEDULED") ? 0 : 1) - (b.startsWith("SCHEDULED") ? 0 : 1));
  // Right after the first content line, before the rest of the body / properties.
  const next = [kept[0], ...planning, ...kept.slice(1)].join("\n");
  return next === visible ? visible : next;
}
