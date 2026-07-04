// Faithful TS port of the anonymization core of `lsdoc/tools/graph-check.mjs`
// (Martin's verified reference tool). Behavior MUST match byte-for-byte — this is
// the privacy guarantee for sharing divergence snippets from a private graph.
//
// Ported verbatim (graph-check.mjs lines 881-940): the two scrub tiers, the
// codepoint transformer, the protected-keyword spans, and the UTF-8-length
// replacement table. `Buffer.byteLength(ch,"utf8")` → `enc.encode(ch).length`.
// All indexing stays in JS-string (UTF-16) space exactly as the original does
// (protectedSpans' `m.index` and transformCodepoints' `i` are both UTF-16), so
// the port is index-consistent with the source without a byte conversion here.

export interface Range {
  start: number;
  end: number;
}

const enc = new TextEncoder();

/** Tier 1: collapse each character to its class — A-Z→'A', a-z→'a', 0-9→'9',
 *  non-ASCII → a placeholder of the SAME utf-8 byte length. Preserves length and
 *  every markup/punctuation byte, scrubbing only prose content. */
export function anonymizeTier1(input: string, protectedRanges: Range[] = []): string {
  return transformCodepoints(input, protectedRanges, (ch) => {
    const cp = ch.codePointAt(0)!;
    if (cp >= 0x41 && cp <= 0x5a) return "A";
    if (cp >= 0x61 && cp <= 0x7a) return "a";
    if (cp >= 0x30 && cp <= 0x39) return "9";
    if (cp > 0x7f) return replacementForUtf8Len(enc.encode(ch).length);
    return ch;
  });
}

/** Tier 2: Caesar-shift letters by 1 (A→B, z→a), digits untouched. Retains more
 *  of the original structure than tier 1 — used when tier-1's total collapse
 *  destroyed the divergence (keyword/length-sensitive parses). */
export function anonymizeTier2(input: string, protectedRanges: Range[] = []): string {
  return transformCodepoints(input, protectedRanges, (ch) => {
    const cp = ch.codePointAt(0)!;
    if (cp >= 0x41 && cp <= 0x5a) return String.fromCharCode(((cp - 0x41 + 1) % 26) + 0x41);
    if (cp >= 0x61 && cp <= 0x7a) return String.fromCharCode(((cp - 0x61 + 1) % 26) + 0x61);
    return ch;
  });
}

function transformCodepoints(input: string, protectedRanges: Range[], fn: (ch: string) => string): string {
  let out = "";
  for (let i = 0; i < input.length; ) {
    const cp = input.codePointAt(i)!;
    const ch = String.fromCodePoint(cp);
    const next = i + ch.length;
    out += inProtectedRange(i, protectedRanges) ? ch : fn(ch);
    i = next;
  }
  return out;
}

function inProtectedRange(index: number, ranges: Range[]): boolean {
  return ranges.some((r) => index >= r.start && index < r.end);
}

function replacementForUtf8Len(len: number): string {
  if (len === 2) return "ä";
  if (len === 3) return "中";
  if (len === 4) return "😀";
  return "中";
}

/** Spans that must survive scrubbing because their literal text drives parser
 *  behavior: URLs, org block markers, `#+KEY:`, PROPERTIES drawers, task
 *  markers / SCHEDULED / DEADLINE, weekday + month names. */
export function protectedSpans(input: string): Range[] {
  const ranges: Range[] = [];
  const patterns = [
    /https?:\/\/[^\s<>"'`)\]]+/gi,
    /#\+(?:BEGIN|END)_[A-Z0-9_+-]+/gi,
    /#\+[A-Z0-9_+-]+:/gi,
    /:PROPERTIES:|:END:/gi,
    /\b(?:TODO|DOING|DONE|NOW|LATER|WAITING|WAIT|CANCELED|CANCELLED|SCHEDULED:|DEADLINE:|CLOSED:)\b/gi,
    /\b(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun|Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday)\b/gi,
    /\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec|January|February|March|April|June|July|August|September|October|November|December)\b/gi,
  ];
  for (const re of patterns) {
    for (const m of input.matchAll(re)) ranges.push({ start: m.index!, end: m.index! + m[0].length });
  }
  ranges.sort((a, b) => a.start - b.start || a.end - b.end);
  return ranges;
}

export interface AnonResult<P = unknown> {
  ok: boolean;
  tier?: string;
  input?: string;
  visible?: string;
  lsdocProjection?: P;
  mldocProjection?: P;
}

/** Try each scrub tier in escalating order and ACCEPT the first whose scrubbed
 *  output STILL reproduces the divergence when re-parsed by both parsers. This is
 *  the guarantee: a shared snippet contains no original prose AND provably
 *  triggers the bug. `verify` re-parses a candidate in FRESH parser state.
 *  Ported from graph-check.mjs:857-879. */
export async function anonymizeAndVerify<P>(
  input: string,
  verify: (candidate: string) => Promise<{ ok: boolean; diverges: boolean; lsdocProjection?: P; mldocProjection?: P }>,
): Promise<AnonResult<P>> {
  const attempts: [string, () => string][] = [
    ["tier 1", () => anonymizeTier1(input, [])],
    ["tier 2", () => anonymizeTier2(input, [])],
    ["tier 1 + protected keywords", () => anonymizeTier1(input, protectedSpans(input))],
    ["tier 2 + protected keywords", () => anonymizeTier2(input, protectedSpans(input))],
  ];
  for (const [tier, make] of attempts) {
    const candidate = make();
    const parsed = await verify(candidate);
    if (parsed.ok && parsed.diverges) {
      return {
        ok: true,
        tier,
        input: candidate,
        visible: JSON.stringify(candidate),
        lsdocProjection: parsed.lsdocProjection,
        mldocProjection: parsed.mldocProjection,
      };
    }
  }
  return { ok: false };
}
