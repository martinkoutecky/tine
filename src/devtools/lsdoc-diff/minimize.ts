// Faithful TS port of graph-check.mjs's shrink-to-smallest-divergent-range logic
// (lines 726-855). The original operates on a Node Buffer; here it operates on a
// Uint8Array and ALL offsets stay in UTF-8 BYTES (never JS UTF-16 units) — that
// invariant is load-bearing: line boundaries, chunk cuts, and the reported line
// range are byte-derived, and mixing in UTF-16 indices would corrupt them on any
// non-ASCII content.
//
// The parser boundary (`parseBothFresh`) is injected: it re-parses a text in
// FRESH parser state and reports whether lsdoc and mldoc diverge.

export type Format = "md" | "org";

export interface ParsedPair {
  ok: boolean;
  diverges: boolean;
}
export type ParseBoth = (text: string, format: Format) => Promise<ParsedPair>;

export interface Minimized {
  input: string;
  inputBytes: number;
  lineStart: number;
  lineEnd: number;
  contextDependent: boolean;
}

interface ByteRange {
  start: number;
  end: number;
  contextDependent?: boolean;
}

const enc = new TextEncoder();
const dec = new TextDecoder();

const decode = (bytes: Uint8Array, start: number, end: number): string => dec.decode(bytes.subarray(start, end));

export function toBytes(s: string): Uint8Array {
  return enc.encode(s);
}

export async function minimize(buffer: Uint8Array, format: Format, parseBoth: ParseBoth): Promise<Minimized> {
  const whole = await parseBoth(decode(buffer, 0, buffer.length), format);
  if (!whole.ok || !whole.diverges) {
    return {
      input: decode(buffer, 0, buffer.length),
      inputBytes: buffer.length,
      lineStart: 1,
      lineEnd: lineNumberForOffset(buffer, buffer.length),
      contextDependent: true,
    };
  }

  const ranges = chunkRanges(buffer, format);
  const candidate = await findDivergentRange(buffer, ranges, format, parseBoth);
  const chosen: ByteRange = candidate || { start: 0, end: buffer.length, contextDependent: true };
  const snippet = decode(buffer, chosen.start, chosen.end);
  return {
    input: snippet,
    inputBytes: enc.encode(snippet).length,
    lineStart: lineNumberForOffset(buffer, chosen.start),
    lineEnd: lineNumberForOffset(buffer, Math.max(chosen.start, chosen.end - 1)),
    contextDependent: Boolean(chosen.contextDependent),
  };
}

async function findDivergentRange(
  buffer: Uint8Array,
  ranges: ByteRange[],
  format: Format,
  parseBoth: ParseBoth,
): Promise<ByteRange | null> {
  if (ranges.length <= 1) return null;
  let lo = 0;
  let hi = ranges.length;
  while (hi - lo > 1) {
    const mid = lo + Math.floor((hi - lo) / 2);
    const left = rangeFromChunks(ranges, lo, mid);
    if (await rangeDiverges(buffer, left, format, parseBoth)) {
      hi = mid;
      continue;
    }
    const right = rangeFromChunks(ranges, mid, hi);
    if (await rangeDiverges(buffer, right, format, parseBoth)) {
      lo = mid;
      continue;
    }
    break;
  }
  const scoped = ranges.slice(lo, hi);
  const base = lo;
  const singles = scoped
    .map((r, i) => ({ ...r, i: base + i }))
    .sort((a, b) => a.end - a.start - (b.end - b.start));
  for (const r of singles) {
    if (await rangeDiverges(buffer, r, format, parseBoth)) return r;
  }
  const maxTests = 2_000;
  let tests = 0;
  for (let len = 2; len <= scoped.length; len++) {
    for (let start = 0; start + len <= scoped.length; start++) {
      if (++tests > maxTests) return null;
      const r = rangeFromChunks(ranges, base + start, base + start + len);
      if (await rangeDiverges(buffer, r, format, parseBoth)) return r;
    }
  }
  return null;
}

function rangeFromChunks(ranges: ByteRange[], lo: number, hi: number): ByteRange {
  return { start: ranges[lo].start, end: ranges[hi - 1].end };
}

async function rangeDiverges(buffer: Uint8Array, range: ByteRange, format: Format, parseBoth: ParseBoth): Promise<boolean> {
  if (range.end <= range.start) return false;
  const parsed = await parseBoth(decode(buffer, range.start, range.end), format);
  return parsed.ok && parsed.diverges;
}

export function chunkRanges(buffer: Uint8Array, format: Format): ByteRange[] {
  if (buffer.length === 0) return [{ start: 0, end: 0 }];
  const lines = splitLineRanges(buffer);
  const boundaries = new Set<number>([0, buffer.length]);
  for (let i = 0; i < lines.length; i++) {
    const line = decode(buffer, lines[i].start, lines[i].contentEnd);
    if (i > 0 && isBoundaryLine(line, format)) boundaries.add(lines[i].start);
    if (i > 0 && isBlankLine(buffer.subarray(lines[i - 1].start, lines[i - 1].contentEnd))) boundaries.add(lines[i].start);
  }
  const sorted = [...boundaries].sort((a, b) => a - b);
  const out: ByteRange[] = [];
  for (let i = 0; i + 1 < sorted.length; i++) {
    if (sorted[i] !== sorted[i + 1]) out.push({ start: sorted[i], end: sorted[i + 1] });
  }
  return out.length ? out : [{ start: 0, end: buffer.length }];
}

interface LineRange {
  start: number;
  contentEnd: number;
  end: number;
}

function splitLineRanges(buffer: Uint8Array): LineRange[] {
  const lines: LineRange[] = [];
  let start = 0;
  for (let i = 0; i < buffer.length; i++) {
    if (buffer[i] === 0x0a) {
      const contentEnd = i > start && buffer[i - 1] === 0x0d ? i - 1 : i;
      lines.push({ start, contentEnd, end: i + 1 });
      start = i + 1;
    } else if (buffer[i] === 0x0d) {
      lines.push({ start, contentEnd: i, end: i + 1 });
      start = i + 1;
    }
  }
  if (start < buffer.length) lines.push({ start, contentEnd: buffer.length, end: buffer.length });
  return lines;
}

function isBoundaryLine(line: string, format: Format): boolean {
  if (format === "org" && /^\*+\s/.test(line)) return true;
  return /^([-*+]\s|\d+\.\s)/.test(line);
}

function isBlankLine(buf: Uint8Array): boolean {
  for (const b of buf) {
    if (b !== 0x20 && b !== 0x09 && b !== 0x0c) return false;
  }
  return true;
}

export function lineNumberForOffset(buffer: Uint8Array, offset: number): number {
  let line = 1;
  const end = Math.min(offset, buffer.length);
  for (let i = 0; i < end; i++) {
    if (buffer[i] === 0x0a || buffer[i] === 0x0d) {
      line++;
      if (buffer[i] === 0x0d && buffer[i + 1] === 0x0a && i + 1 < end) i++;
    }
  }
  return line;
}
