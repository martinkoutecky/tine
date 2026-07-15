// Port of graph-check.mjs's `selfTest` (lines 1200-1235) — the correctness
// contract for the anonymizer + chunker. If these pass, the in-app scrub behaves
// exactly like Martin's verified CLI tool.
import { describe, expect, it } from "vitest";
import {
  anonymizeTier1,
  anonymizeTier2,
  anonymizeAndVerify,
  divergenceSignature,
  anonymizeSourceRel,
  protectedSpans,
} from "./anonymize";
import { chunkRanges, toBytes } from "./minimize";
import { isMldocBacktickStateArtifact } from "./oracle-artifacts";
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

  it("tier 2 Caesar-shifts letters and digits", () => {
    expect(anonymizeTier2("Azaz09")).toBe("Baba10");
  });

  it("keeps URL schemes parseable while scrubbing the host, path, query, and fragment", () => {
    const original = "See https://private.example/Client/Plan?q=Secret#Board";
    const scrubbed = anonymizeTier1(original, protectedSpans(original));
    expect(scrubbed).toContain("https://");
    expect(scrubbed).not.toContain("private");
    expect(scrubbed).not.toContain("example");
    expect(scrubbed).not.toContain("Client");
    expect(scrubbed).not.toContain("Secret");
    expect(scrubbed).toMatch(/https:\/\/a+\.a+\/A[a]+\/A[a]+\?a=A[a]+#A[a]+/);
  });

  it("does not retain numeric URL identifiers in the structure-preserving tier", () => {
    const original = "https://123.example/account/456";
    const scrubbed = anonymizeTier2(original, protectedSpans(original));
    expect(scrubbed).toBe("https://234.fybnqmf/bddpvou/567");
    expect(scrubbed).not.toContain("123");
    expect(scrubbed).not.toContain("456");
  });

  it("keeps percent escapes syntactically valid without retaining encoded bytes", () => {
    const original = "https://example.test/private%20name%2Fsecret";
    const scrubbed = anonymizeTier2(original, protectedSpans(original));
    expect(scrubbed).toContain("https://");
    expect(scrubbed).toContain("%41");
    expect(scrubbed).not.toContain("%20");
    expect(scrubbed).not.toContain("%2F");
    expect([...scrubbed.matchAll(/%([^\s]{2})/g)].every((m) => /^[0-9A-F]{2}$/.test(m[1]))).toBe(true);
  });

  it("replaces source page names with neutral stable labels", () => {
    expect(anonymizeSourceRel("pages/Client Roadmap.md", 6)).toBe("graph-file-0007.md");
    expect(anonymizeSourceRel("journals/2026_07_11.org", 1)).toBe("graph-file-0002.org");
  });
});

describe("anonymizeAndVerify tier escalation", () => {
  // Verifier that only accepts tier-2 output of "Ab1" (= "Bc2"), forcing the
  // fallback past tier 1 (= "Aa9").
  const verify = (accept: (c: string) => boolean) => async (candidate: string) => ({
    ok: true,
    diverges: accept(candidate),
  });

  it("selects tier 2 when tier 1 no longer reproduces", async () => {
    const r = await anonymizeAndVerify("Ab1", verify((c) => c === "Bc2"));
    expect(r.ok).toBe(true);
    expect(r.tier).toBe("tier 2");
    expect(r.input).toBe("Bc2");
  });

  it("fails when no tier reproduces the divergence", async () => {
    const r = await anonymizeAndVerify("Ab1", verify(() => false));
    expect(r.ok).toBe(false);
  });

  it("keeps trying when a scrubbed candidate reproduces only a rejected oracle artifact", async () => {
    const refs = { page: [] as string[], block: [] as string[] };
    const artifact = {
      lsdoc: { blocks: [{ kind: "paragraph", inline: [{ k: "plain", text: "ä " }, { k: "code", text: "`aaaa" }] }], refs },
      mldoc: { blocks: [{ kind: "paragraph", inline: [{ k: "plain", text: "ä `" }, { k: "code", text: "aaaa" }] }], refs },
    };
    const actionable = {
      lsdoc: { blocks: [{ kind: "paragraph", inline: [{ k: "plain", text: "lsdoc" }] }], refs },
      mldoc: { blocks: [{ kind: "paragraph", inline: [{ k: "plain", text: "mldoc" }] }], refs },
    };
    const r = await anonymizeAndVerify(
      "Ab1",
      async (candidate) => {
        const pair = candidate === "Aa9" ? artifact : actionable;
        return { ok: true, diverges: true, lsdocProjection: pair.lsdoc, mldocProjection: pair.mldoc };
      },
      (parsed) => !isMldocBacktickStateArtifact(parsed.lsdocProjection!, parsed.mldocProjection!),
      { ok: true, diverges: true, lsdocProjection: actionable.lsdoc, mldocProjection: actionable.mldoc },
    );
    expect(r.ok).toBe(true);
    expect(r.tier).toBe("tier 2");
    expect(r.input).toBe("Bc2");
  });

  it("rejects a scrub that changes which structural parser delta survived", async () => {
    const sameLeft = { blocks: [{ kind: "paragraph", text: "private" }], refs: { page: [], block: [] } };
    const sameRight = { blocks: [{ kind: "heading", text: "other" }], refs: { page: [], block: [] } };
    const differentLeft = { blocks: [{ kind: "paragraph" }], refs: { page: ["x"], block: [] } };
    const differentRight = { blocks: [{ kind: "paragraph" }], refs: { page: [], block: [] } };
    const r = await anonymizeAndVerify(
      "Ab1",
      async (candidate) => {
        const pair = candidate === "Aa9"
          ? { left: differentLeft, right: differentRight }
          : { left: sameLeft, right: sameRight };
        return { ok: true, diverges: true, lsdocProjection: pair.left, mldocProjection: pair.right };
      },
      () => true,
      { ok: true, diverges: true, lsdocProjection: sameLeft, mldocProjection: sameRight },
    );
    expect(r.ok).toBe(true);
    expect(r.tier).toBe("tier 2");
  });

  it("treats scrubbed scalar payloads as the same divergence identity", () => {
    const a1 = { blocks: [{ kind: "plain", text: "secret" }], refs: { page: [], block: [] } };
    const b1 = { blocks: [{ kind: "code", text: "private" }], refs: { page: [], block: [] } };
    const a2 = { blocks: [{ kind: "plain", text: "aaaaaa" }], refs: { page: [], block: [] } };
    const b2 = { blocks: [{ kind: "code", text: "bbbbbbb" }], refs: { page: [], block: [] } };
    expect(divergenceSignature(a1, b1)).toBe(divergenceSignature(a2, b2));

    const structurallyDifferent = { blocks: [{ kind: "heading", text: "bbbbbbb" }], refs: { page: [], block: [] } };
    expect(divergenceSignature(a2, structurallyDifferent)).not.toBe(divergenceSignature(a1, b1));
  });

  it("can retain a URL-sensitive divergence without retaining the private URL", async () => {
    const original = "- https://private.example/Client/Plan?q=Secret#Board";
    const r = await anonymizeAndVerify(original, verify((candidate) =>
      candidate.includes("https://") && !candidate.includes("private.example"),
    ));
    expect(r.ok).toBe(true);
    expect(r.input).toContain("https://");
    expect(r.input).not.toContain("private.example");
    expect(r.input).not.toContain("Client");
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
