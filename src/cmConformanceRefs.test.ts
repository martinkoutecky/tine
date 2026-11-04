// Conceptual model conformance, step 2 batch 6 phase A (census Q6 item 2, Q11
// item 4): the frontend's reference reading, run for real, against the Rust
// answers on the same texts.
//
// The Rust harness (`crates/tine-store/src/cm_conformance_extraction_tests.rs`)
// writes `crates/tine-store/tests/fixtures/cm-conformance/frontend-refs/{md,org}.json`
// (and fails when a committed case is stale): per generated text its blocks
// and each block's reference keys from `reference_evidence` (the one answer
// for "what does this block reference").
//
// Checked here: `components/blockSwipeWiring.ts swipeRefNames` (OG's
// `:gestures/disabled-in-block-with-tags` reads the block's refs), per block
// against the evidence keys.
//
// Batch 6 phase B retired the second check: `document/graphRewrite.ts
// unsavedPathsFor` no longer reads references itself; it asks the rename
// (`renameWouldChange`, the `rename_would_change` command), pinned in Rust by
// `crates/tine-graph-features/tests/rename_would_change.rs` and in the window
// by `renameStuckPage.test.ts`.
//
// Differences are compared with the `frontendRefs` part of
// `expected-differences.json`; `CM_REFRESH=1` rewrites that part.
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "./render/parse";
import { swipeRefNames } from "./components/blockSwipeWiring";

interface Entry {
  name: string;
  preBlock: string | null;
  blocks: { raw: string; refs: string[] }[];
}

const FIXTURES = path.resolve(__dirname, "../crates/tine-store/tests/fixtures/cm-conformance");
const EXPECTED = path.join(FIXTURES, "expected-differences.json");

function entries(format: "md" | "org"): [string, Entry][] {
  const file = JSON.parse(readFileSync(path.join(FIXTURES, "frontend-refs", `${format}.json`), "utf8")) as Record<string, Entry>;
  return Object.entries(file).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
}

/** Sorted keys, so the file matches what serde_json (BTreeMap) writes. */
function sorted(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(sorted);
  if (v && typeof v === "object") {
    return Object.fromEntries(Object.keys(v).sort().map((k) => [k, sorted((v as Record<string, unknown>)[k])]));
  }
  return v;
}

const show = (keys: Iterable<string>) => JSON.stringify([...keys].sort());

function swipeDiffs(e: Entry, format: "md" | "org"): string[] {
  const out: string[] = [];
  for (const block of e.blocks) {
    const ts = new Set(swipeRefNames(block.raw, format));
    const rust = new Set(block.refs);
    const extra = [...ts].filter((k) => !rust.has(k));
    const missing = [...rust].filter((k) => !ts.has(k));
    if (extra.length) out.push(`swipeRefNames: frontend-only refs ${show(extra)}`);
    if (missing.length) out.push(`swipeRefNames: evidence-only refs ${show(missing)}`);
  }
  return out;
}

beforeAll(() => initParser());

describe("conceptual model conformance: the frontend's reference walker", () => {
  it("differs from the Rust answers exactly as expected-differences.json records", async () => {
    const now: Record<string, string[]> = {};
    for (const format of ["md", "org"] as const) {
      for (const [id, e] of entries(format)) {
        const diffs = swipeDiffs(e, format).sort();
        if (diffs.length) now[id] = diffs;
      }
    }
    const file = JSON.parse(readFileSync(EXPECTED, "utf8")) as Record<string, unknown>;
    if (process.env.CM_REFRESH) {
      file.frontendRefs = { fixtures: now };
      writeFileSync(EXPECTED, JSON.stringify(sorted(file), null, 2) + "\n");
      return;
    }
    const was = ((file.frontendRefs as { fixtures?: Record<string, string[]> } | undefined)?.fixtures) ?? {};
    expect(
      now,
      "the frontend's reference differences changed; if intended, refresh with CM_REFRESH=1 npx vitest run src/cmConformanceRefs.test.ts",
    ).toEqual(was);
  }, 600_000);
});
