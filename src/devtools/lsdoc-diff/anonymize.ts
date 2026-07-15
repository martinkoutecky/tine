// TS port of the anonymization core of `lsdoc/tools/graph-check.mjs` (Martin's
// verified reference tool), with stricter in-app privacy for source names, URL
// content, numeric identifiers, and percent escapes. The privacy contract is
// enforced by focused tests and verify-after-scrub before anything is shown.
//
// Ported verbatim (graph-check.mjs lines 881-940): the two scrub tiers, the
// codepoint transformer, the protected-keyword spans, and the UTF-8-length
// replacement table. `Buffer.byteLength(ch,"utf8")` → `enc.encode(ch).length`.
// All indexing stays in JS-string (UTF-16) space exactly as the original does
// (protectedSpans' `m.index` and transformCodepoints' `i` are both UTF-16), so
// the port is index-consistent with the source without a byte conversion here.

import { projectionKey } from "./projection";

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

/** Tier 2: Caesar-shift letters and digits by 1 (A→B, z→a, 9→0). Retains more
 *  of the original structure than tier 1 — used when tier-1's total collapse
 *  destroyed the divergence (keyword/length-sensitive parses), without leaking
 *  numeric identifiers in page names or URLs. */
export function anonymizeTier2(input: string, protectedRanges: Range[] = []): string {
  return transformCodepoints(input, protectedRanges, (ch) => {
    const cp = ch.codePointAt(0)!;
    if (cp >= 0x41 && cp <= 0x5a) return String.fromCharCode(((cp - 0x41 + 1) % 26) + 0x41);
    if (cp >= 0x61 && cp <= 0x7a) return String.fromCharCode(((cp - 0x61 + 1) % 26) + 0x61);
    if (cp >= 0x30 && cp <= 0x39) return String.fromCharCode(((cp - 0x30 + 1) % 10) + 0x30);
    return ch;
  });
}

function transformCodepoints(input: string, protectedRanges: Range[], fn: (ch: string) => string): string {
  let out = "";
  for (let i = 0; i < input.length; ) {
    // Preserve percent-encoding syntax without retaining the encoded byte. A
    // character-by-character Caesar shift can turn `%2F` into invalid `%3G`,
    // which makes URL parsing disappear during re-verification.
    if (
      input[i] === "%" &&
      /^[0-9A-Fa-f]{2}$/.test(input.slice(i + 1, i + 3)) &&
      !inProtectedRange(i, protectedRanges)
    ) {
      out += "%41";
      i += 3;
      continue;
    }
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
 *  behavior: URL schemes, org block markers, `#+KEY:`, PROPERTIES drawers,
 *  task markers / SCHEDULED / DEADLINE, weekday + month names.
 *
 *  Only the `http://` / `https://` prefix is protected. Protecting the complete
 *  URL used to leak private hosts and paths; scrubbing the prefix used to stop
 *  both parsers from recognizing a URL and could erase a URL-sensitive
 *  divergence. The remaining URL characters keep their punctuation and byte
 *  shape through the normal anonymizer. */
export function protectedSpans(input: string): Range[] {
  const ranges: Range[] = [];
  const patterns = [
    /https?:\/\//gi,
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

/** A finding's source path is itself private graph content (usually the page
 *  name). Replace it with a stable per-run label while retaining only the file
 *  format needed to interpret the snippet. */
export function anonymizeSourceRel(rel: string, index: number): string {
  const ext = rel.toLowerCase().endsWith(".org") ? ".org" : ".md";
  return `graph-file-${String(index + 1).padStart(4, "0")}${ext}`;
}

export interface AnonResult<P = unknown> {
  ok: boolean;
  tier?: string;
  input?: string;
  visible?: string;
  lsdocProjection?: P;
  mldocProjection?: P;
}

interface VerifiedCandidate<P> {
  ok: boolean;
  diverges: boolean;
  lsdocProjection?: P;
  mldocProjection?: P;
}

/** Structural identity of the delta between the two canonical parser
 * projections. Scalar payloads are intentionally ignored (the anonymizer must
 * change them), while side, path, type, missing keys and array shape are kept.
 * Thus "a mismatch still exists" is insufficient: it must be the same class of
 * mismatch at the same structural location. */
export function divergenceSignature(left: unknown, right: unknown): string {
  const a = JSON.parse(projectionKey(left as never));
  const b = JSON.parse(projectionKey(right as never));
  const out: string[] = [];
  const typeOf = (value: unknown) => value === null ? "null" : Array.isArray(value) ? "array" : typeof value;
  // Syntax discriminators must retain their actual values; paragraph→heading is
  // not the same defect as paragraph→list. Prose/URL/ref payloads may be scrubbed,
  // so other scalar mismatches retain path and type but not private values.
  const structuralScalarKeys = new Set([
    "kind", "k", "type", "format", "level", "marker", "checkbox",
    "ordered", "start", "indent", "style", "delimiter", "language",
  ]);
  const pathPart = (key: string) => key.replaceAll("~", "~0").replaceAll("/", "~1");
  const walk = (path: string, av: unknown, bv: unknown, aPresent = true, bPresent = true) => {
    if (!aPresent || !bPresent) {
      out.push(`${path}|missing-${aPresent ? "right" : "left"}|${typeOf(aPresent ? av : bv)}`);
      return;
    }
    const at = typeOf(av);
    const bt = typeOf(bv);
    if (at !== bt) {
      out.push(`${path}|type|${at}->${bt}`);
      return;
    }
    if (Array.isArray(av) && Array.isArray(bv)) {
      if (av.length !== bv.length) out.push(`${path}|array-length|${av.length}->${bv.length}`);
      const length = Math.max(av.length, bv.length);
      for (let index = 0; index < length; index += 1) {
        walk(`${path}/${index}`, av[index], bv[index], index < av.length, index < bv.length);
      }
      return;
    }
    if (av && bv && at === "object") {
      const ao = av as Record<string, unknown>;
      const bo = bv as Record<string, unknown>;
      const keys = [...new Set([...Object.keys(ao), ...Object.keys(bo)])].sort();
      for (const key of keys) {
        walk(`${path}/${pathPart(key)}`, ao[key], bo[key], Object.hasOwn(ao, key), Object.hasOwn(bo, key));
      }
      return;
    }
    if (!Object.is(av, bv)) {
      const key = path.slice(path.lastIndexOf("/") + 1).replaceAll("~1", "/").replaceAll("~0", "~");
      const detail = structuralScalarKeys.has(key)
        ? `${JSON.stringify(av)}->${JSON.stringify(bv)}`
        : `${at}->${bt}`;
      out.push(`${path}|scalar|${detail}`);
    }
  };
  walk("", a, b);
  return out.sort().join("\n");
}

/** Try each scrub tier in escalating order and ACCEPT the first whose scrubbed
 *  output STILL reproduces the divergence when re-parsed by both parsers. This is
 *  the guarantee: a shared snippet contains no original prose AND provably
 *  triggers the bug. `verify` re-parses a candidate in FRESH parser state.
 *  Ported from graph-check.mjs:857-879. */
export async function anonymizeAndVerify<P>(
  input: string,
  verify: (candidate: string) => Promise<VerifiedCandidate<P>>,
  accept: (parsed: VerifiedCandidate<P>) => boolean = () => true,
  expected?: VerifiedCandidate<P>,
): Promise<AnonResult<P>> {
  const expectedSignature = expected?.ok
    && expected.diverges
    && expected.lsdocProjection !== undefined
    && expected.mldocProjection !== undefined
    ? divergenceSignature(expected.lsdocProjection, expected.mldocProjection)
    : null;
  const attempts: [string, () => string][] = [
    ["tier 1", () => anonymizeTier1(input, [])],
    ["tier 2", () => anonymizeTier2(input, [])],
    ["tier 1 + protected keywords", () => anonymizeTier1(input, protectedSpans(input))],
    ["tier 2 + protected keywords", () => anonymizeTier2(input, protectedSpans(input))],
  ];
  for (const [tier, make] of attempts) {
    const candidate = make();
    const parsed = await verify(candidate);
    // Reproducing any parser difference is not enough: a scrub can erase the
    // original actionable delta while leaving only a known oracle artifact.
    // Callers may reject that candidate and let the remaining tiers try to
    // preserve a faithful, shareable reproduction.
    const signature = parsed.lsdocProjection !== undefined && parsed.mldocProjection !== undefined
      ? divergenceSignature(parsed.lsdocProjection, parsed.mldocProjection)
      : null;
    if (parsed.ok && parsed.diverges && accept(parsed)
      && (expected === undefined || (expectedSignature !== null && signature === expectedSignature))) {
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
