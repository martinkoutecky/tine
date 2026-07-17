// Correctness + privacy contract for the in-app anonymizer and chunker. The app
// intentionally fails closed where the developer CLI's reversible fallback
// would expose graph content.
import { describe, expect, it } from "vitest";
import {
  anonymizeTier1,
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

  it("does not retain numeric URL identifiers", () => {
    const original = "https://123.example/account/456";
    const scrubbed = anonymizeTier1(original, protectedSpans(original));
    expect(scrubbed).toBe("https://999.aaaaaaa/aaaaaaa/999");
    expect(scrubbed).not.toContain("123");
    expect(scrubbed).not.toContain("456");
  });

  it("keeps percent escapes syntactically valid without retaining encoded bytes", () => {
    const original = "https://example.test/private%20name%2Fsecret";
    const scrubbed = anonymizeTier1(original, protectedSpans(original));
    expect(scrubbed).toContain("https://");
    expect(scrubbed).toContain("%41");
    expect(scrubbed).not.toContain("%20");
    expect(scrubbed).not.toContain("%2F");
    expect([...scrubbed.matchAll(/%([^\s]{2})/g)].every((m) => /^[0-9A-F]{2}$/.test(m[1]))).toBe(true);
  });

  it("never preserves non-ASCII prose or private custom Org identifiers", () => {
    const original = "#+BEGIN_CLIENT_ACME\n秘密 Проект\n#+PRIVATE_CLIENT: Acme42\n#+END_CLIENT_ACME";
    const scrubbed = anonymizeTier1(original, protectedSpans(original));
    expect(scrubbed).not.toContain("CLIENT");
    expect(scrubbed).not.toContain("ACME");
    expect(scrubbed).not.toContain("PRIVATE");
    expect(scrubbed).not.toContain("秘密");
    expect(scrubbed).not.toContain("Проект");
    expect(scrubbed).toContain("#+BEGIN_");
    expect(scrubbed).toContain("#+END_");
  });

  it("replaces source page names with neutral stable labels", () => {
    expect(anonymizeSourceRel("pages/Client Roadmap.md", 6)).toBe("graph-file-0007.md");
    expect(anonymizeSourceRel("journals/2026_07_11.org", 1)).toBe("graph-file-0002.org");
  });
});

describe("anonymizeAndVerify privacy-safe tier escalation", () => {
  const verify = (accept: (c: string) => boolean) => async (candidate: string) => ({
    ok: true,
    diverges: accept(candidate),
  });

  it("uses only a fixed grammar-token fallback when full collapse loses the mismatch", async () => {
    const r = await anonymizeAndVerify("TODO Secret", verify((c) => c === "TODO Aaaaaa"));
    expect(r.ok).toBe(true);
    expect(r.tier).toBe("tier 1 + protected keywords");
    expect(r.input).toBe("TODO Aaaaaa");
  });

  it("fails closed when only the old reversible Caesar candidate would reproduce", async () => {
    const r = await anonymizeAndVerify("Client秘密42", verify((candidate) => candidate === "Dmjfou秘密53"));
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
      "TODO Secret",
      async (candidate) => {
        const pair = candidate === "AAAA Aaaaaa" ? artifact : actionable;
        return { ok: true, diverges: true, lsdocProjection: pair.lsdoc, mldocProjection: pair.mldoc };
      },
      (parsed) => !isMldocBacktickStateArtifact(parsed.lsdocProjection!, parsed.mldocProjection!),
      { ok: true, diverges: true, lsdocProjection: actionable.lsdoc, mldocProjection: actionable.mldoc },
    );
    expect(r.ok).toBe(true);
    expect(r.tier).toBe("tier 1 + protected keywords");
    expect(r.input).toBe("TODO Aaaaaa");
  });

  it("rejects a scrub that changes which structural parser delta survived", async () => {
    const sameLeft = { blocks: [{ kind: "paragraph", text: "private" }], refs: { page: [], block: [] } };
    const sameRight = { blocks: [{ kind: "heading", text: "other" }], refs: { page: [], block: [] } };
    const differentLeft = { blocks: [{ kind: "paragraph" }], refs: { page: ["x"], block: [] } };
    const differentRight = { blocks: [{ kind: "paragraph" }], refs: { page: [], block: [] } };
    const r = await anonymizeAndVerify(
      "TODO Secret",
      async (candidate) => {
        const pair = candidate === "AAAA Aaaaaa"
          ? { left: differentLeft, right: differentRight }
          : { left: sameLeft, right: sameRight };
        return { ok: true, diverges: true, lsdocProjection: pair.left, mldocProjection: pair.right };
      },
      () => true,
      { ok: true, diverges: true, lsdocProjection: sameLeft, mldocProjection: sameRight },
    );
    expect(r.ok).toBe(true);
    expect(r.tier).toBe("tier 1 + protected keywords");
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
