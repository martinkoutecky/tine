// Port of graph-check.mjs's `selfTest` (lines 1200-1235) — the correctness
// contract for the anonymizer + chunker. If these pass, the in-app scrub behaves
// exactly like Martin's verified CLI tool.
import { describe, expect, it } from "vitest";
import { anonymizeTier1, anonymizeTier2, anonymizeAndVerify } from "./anonymize";
import { chunkRanges, toBytes } from "./minimize";
import { projectionKey } from "./projection";

const enc = new TextEncoder();
const byteLen = (s: string) => enc.encode(s).length;

describe("anonymize tiers", () => {
  it("tier 1 preserves utf-8 byte length and character classes", () => {
    const original = "Ab9é中😀!\n";
    const t1 = anonymizeTier1(original);
    expect(byteLen(t1)).toBe(byteLen(original)); // byte length unchanged
    expect(t1.startsWith("Aa9")).toBe(true); // case/digit classes preserved
    const chars = [...t1];
    expect(byteLen(chars[3])).toBe(2); // é → 2-byte placeholder
    expect(byteLen(chars[4])).toBe(3); // 中 → 3-byte placeholder
    expect(byteLen(chars[5])).toBe(4); // 😀 → 4-byte placeholder
  });

  it("tier 2 Caesar-shifts letters and keeps digits", () => {
    expect(anonymizeTier2("Azaz09")).toBe("Baba09");
  });
});

describe("anonymizeAndVerify tier escalation", () => {
  // Verifier that only accepts tier-2 output of "Ab1" (= "Bc1"), forcing the
  // fallback past tier 1 (= "Aa9").
  const verify = (accept: (c: string) => boolean) => async (candidate: string) => ({
    ok: true,
    diverges: accept(candidate),
  });

  it("selects tier 2 when tier 1 no longer reproduces", async () => {
    const r = await anonymizeAndVerify("Ab1", verify((c) => c === "Bc1"));
    expect(r.ok).toBe(true);
    expect(r.tier).toBe("tier 2");
    expect(r.input).toBe("Bc1");
  });

  it("fails when no tier reproduces the divergence", async () => {
    const r = await anonymizeAndVerify("Ab1", verify(() => false));
    expect(r.ok).toBe(false);
  });
});

describe("projectionKey (vendored compare.mjs resolves + canonicalizes)", () => {
  it("is key-order-insensitive and drops span/aligns/span_map", () => {
    const a = projectionKey({ blocks: [{ kind: "p", inline: [], span: [0, 1] }], refs: { page: ["X"], block: [] } });
    const b = projectionKey({ refs: { block: [], page: ["X"] }, blocks: [{ inline: [], kind: "p", aligns: ["l"] }] });
    expect(a).toBe(b); // ignored keys + key order don't affect equality
  });

  it("distinguishes genuinely different projections", () => {
    const a = projectionKey({ blocks: [{ kind: "p" }], refs: { page: [], block: [] } });
    const b = projectionKey({ blocks: [{ kind: "heading" }], refs: { page: [], block: [] } });
    expect(a).not.toBe(b);
  });
});

describe("chunkRanges byte fidelity", () => {
  it("round-trips CRLF bytes and keeps the CRLF inside the first chunk", () => {
    const crlf = toBytes("- one\r\nbody\r\n\r\n- two\r\n");
    const ranges = chunkRanges(crlf, "md");
    // concatenating every chunk's bytes reproduces the input exactly
    const total = ranges.reduce((n, r) => n + (r.end - r.start), 0);
    const joined = new Uint8Array(total);
    let o = 0;
    for (const r of ranges) {
      joined.set(crlf.subarray(r.start, r.end), o);
      o += r.end - r.start;
    }
    expect(Array.from(joined)).toEqual(Array.from(crlf));
    // first chunk retains its CRLF (0x0d 0x0a) rather than splitting on it
    const first = crlf.subarray(ranges[0].start, ranges[0].end);
    let hasCrlf = false;
    for (let i = 0; i + 1 < first.length; i++) if (first[i] === 0x0d && first[i + 1] === 0x0a) hasCrlf = true;
    expect(hasCrlf).toBe(true);
  });
});
