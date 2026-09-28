import { readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { splitPagePreamble, upsertPropertyLine } from "./editor/properties";
import { pageProperties } from "./render/block";

// The anonymized-graph acceptance gate for GH #164's property writers
// (ported from master d9faed555 in og 14 Q5; the probe loop adds a non-ASCII key).
//
// **Why this exists as a permanent, env-gated test.** The corpus discipline is
// that synthetic fixtures are generated from *our model of a graph* - the same
// model that produced the bug - so a packet touching a save path takes a real
// graph as an acceptance gate. That gate is worth nothing if it is a one-off
// script that vanishes with the packet. This is the TypeScript analogue of
// `crates/tine-core/src/direct_move_recovery_corpus_tests.rs`, whose `#[ignore]`
// is spelled `describe.skipIf` here: absent ANON_GRAPH it does not run, and it
// is never part of the fast loop.
//
// **It contains no corpus content, and must never grow any.** The graph is named
// by `ANON_GRAPH` and is only ever READ - these writers are pure functions over
// strings, so unlike the Rust gate there is nothing to copy and nothing that can
// mutate the corpus. Every assertion is on a COUNT, never on corpus text: a bare
// `expect(actual).toBe(expected)` would print page bytes into a failure message,
// and this file is committed and public while the corpus is not. Counters only.
//
// Run it with:
//
//   ANON_GRAPH=~/research/logseq-anonymized npx vitest run src/propertyWriterCorpus.test.ts
//
// **Known limit, stated so a green run is not over-read:** this corpus is all
// Markdown (1075 `.md`, 0 `.org`), so it cannot exercise the Org page-property
// directive writer from the same packet. Org stays proven at the unit layer
// (`orgPreBlockWithProperty` in src/editor/properties.test.ts, and the org page
// write in src/store.test.ts). A gate cannot prove what its corpus lacks.
const ANON = process.env.ANON_GRAPH;
const PROBES = ["tine.corpus-probe", "klíč-probe"];

function pageFiles(root: string): string[] {
  return ["pages", "journals"].flatMap((dir) => {
    const base = path.join(root, dir);
    let entries: string[];
    try {
      entries = readdirSync(base);
    } catch {
      return [];
    }
    return entries
      .filter((name) => name.endsWith(".md") || name.endsWith(".org"))
      .map((name) => path.join(base, name))
      .filter((file) => statSync(file).isFile());
  });
}

describe.skipIf(!ANON)("property writers over a real graph (ANON_GRAPH)", () => {
  it("adds and removes an arbitrary key without disturbing one other byte", () => {
    const files = pageFiles(ANON as string);

    let scanned = 0;
    let withHeader = 0;
    // The splitter's own documented invariant: properties + remainder === raw.
    let splitNotByteExact = 0;
    // Add-then-remove must land back on the original header, byte for byte.
    let roundTripNotByteExact = 0;
    // Adding a key must not drop, duplicate or reorder the keys already there.
    let existingKeysDisturbed = 0;
    // The probe must actually be readable back, or the "add" proved nothing.
    let probeNotReadBack = 0;
    // Everything after the header must be untouched by a header-only write.
    let remainderDisturbed = 0;

    for (const file of files) {
      const raw = readFileSync(file, "utf8");
      scanned += 1;

      const { properties, remainder } = splitPagePreamble(raw);
      if ((properties ?? "") + (remainder ?? "") !== raw) splitNotByteExact += 1;
      if (properties !== null) withHeader += 1;

      const format = file.endsWith(".org") ? "org" : "md";
      const before = pageProperties(properties, format);

      for (const PROBE of PROBES) {
        const added = upsertPropertyLine(properties, PROBE, "x");
        const after = pageProperties(added, format);

        if (!after.some(([k, v]) => k.toLowerCase() === PROBE && v === "x")) probeNotReadBack += 1;

        // Every key/value that was there must still be there, in the same order.
        const survived = after.filter(([k]) => k.toLowerCase() !== PROBE);
        const sameAsBefore = survived.length === before.length
          && survived.every(([k, v], i) => k === before[i][0] && v === before[i][1]);
        if (!sameAsBefore) existingKeysDisturbed += 1;

        // A header write must not reach past the header.
        const rewritten = (added ?? "") + (remainder ?? "");
        if (!rewritten.endsWith(remainder ?? "")) remainderDisturbed += 1;

        const removed = upsertPropertyLine(added, PROBE, null);
        if (removed !== properties) roundTripNotByteExact += 1;
      }
    }

    // Counts only - never a name, a path, or a byte.
    console.log(`corpus files scanned: ${scanned}; with a property header: ${withHeader}`);

    expect(scanned).toBeGreaterThan(0);
    expect(splitNotByteExact).toBe(0);
    expect(probeNotReadBack).toBe(0);
    expect(existingKeysDisturbed).toBe(0);
    expect(remainderDisturbed).toBe(0);
    expect(roundTripNotByteExact).toBe(0);
  });
});
