/** Is a local font installed? (GH #760, the "not installed; using fallback" hint.)
 *
 * The web platform has no font enumeration, and `document.fonts.check()` answers
 * "true" for a family nothing defines. The portable probe measures one pangram in
 * `"Name", <generic>` against the bare generic: a different width means the name
 * resolved to a real face. Three generics guard against a font that happens to
 * match one of them. Cost: at most three canvas `measureText` calls per family,
 * once per family name for the life of the page (memoized), and only for families
 * of the installed theme rows being shown, never per render of blocks. When the
 * environment cannot measure (no canvas) the answer is "unknown" and no hint shows. */
export type FontPresence = "installed" | "missing" | "unknown";
export type TextMeasurer = (font: string) => number | null;

const SAMPLE = "mmmmmmmmmmlliWQ@0123456789 The quick brown fox";
const GENERICS = ["monospace", "serif", "sans-serif"] as const;

let canvasContext: { font: string; measureText(text: string): { width: number } } | null | undefined;

function canvasMeasurer(font: string): number | null {
  if (canvasContext === undefined) {
    try {
      canvasContext = typeof document === "undefined" ? null : document.createElement("canvas").getContext("2d");
    } catch {
      canvasContext = null;
    }
  }
  if (!canvasContext) return null;
  canvasContext.font = font;
  return canvasContext.measureText(SAMPLE).width;
}

const MEMO_LIMIT = 256;
const memo = new Map<string, FontPresence>();

/** Probe `name` (a family name from a validated stack: letters, digits, `. _ -`
 * and spaces, so it is safe inside a quoted canvas font string). */
export function fontPresence(name: string, measure: TextMeasurer = canvasMeasurer): FontPresence {
  const cached = memo.get(name);
  if (cached && measure === canvasMeasurer) return cached;
  let result: FontPresence = "missing";
  for (const generic of GENERICS) {
    const base = measure(`72px ${generic}`);
    const test = measure(`72px "${name}", ${generic}`);
    if (base === null || test === null || base === 0) { result = "unknown"; break; }
    if (Math.abs(base - test) > 0.01) { result = "installed"; break; }
  }
  if (measure === canvasMeasurer) {
    if (memo.size >= MEMO_LIMIT) memo.clear();
    memo.set(name, result);
  }
  return result;
}

/** Test seam: forget memoized answers and the cached canvas. */
export function resetFontPresenceForTests(): void {
  memo.clear();
  canvasContext = undefined;
}
