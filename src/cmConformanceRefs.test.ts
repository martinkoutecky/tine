// Conceptual model conformance, step 2 batch 6 phase A (census Q6 item 2, Q11
// item 4): the frontend's own reference walker (`render/pageRefs.ts
// pageRefsInText`), run for real through its two callers, against the Rust
// answers on the same texts.
//
// The Rust harness (`crates/tine-store/src/cm_conformance_extraction_tests.rs`)
// writes `crates/tine-store/tests/fixtures/cm-conformance/frontend-refs/{md,org}.json`
// (and fails when a committed case is stale): per generated text its blocks,
// each block's reference keys from `reference_evidence` (the one answer for
// "what does this block reference"), and per candidate page key whether the
// backend rename rewrites the text (`refs::rename_rewrite` changes it, the
// check the planner's `include_inputs` applies to an unsaved buffer) and
// whether the page is one of that name's `explicit_referrers`.
//
// Checked here:
// - `components/blockSwipeWiring.ts swipeRefNames` (OG's
//   `:gestures/disabled-in-block-with-tags` reads the block's refs), per block
//   against the evidence keys;
// - `document/graphRewrite.ts unsavedPathsFor`, through `renameOrMergePage`
//   exactly as `renameStuckPage.test.ts` drives it: the text is the unsaved,
//   unsavable edit of a stuck page, and the rename of each candidate key either
//   refuses (`{ unsaved, mentions: true }`) or proceeds; that must match whether
//   the backend's rename would rewrite the text.
//
// Differences are compared with the `frontendRefs` part of
// `expected-differences.json`; `CM_REFRESH=1` rewrites that part.
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { beforeAll, describe, expect, it, vi } from "vitest";
import { initParser } from "./render/parse";
import { backend } from "./backend";
import { renameOrMergePage } from "./graph";
import { setDoc } from "./document/model";
import { resetStore } from "./document";
import { activatePageInstance } from "./document/instance";
import { bindTestHost } from "./document/host/wiring.test.support";
import { installDiskHost } from "./diskHost.test.support";
import { setRaw } from "./document/edits/blocks";
import { swipeRefNames } from "./components/blockSwipeWiring";
import type { PageDto } from "./types";

interface Entry {
  name: string;
  preBlock: string | null;
  blocks: { raw: string; refs: string[] }[];
  renames: Record<string, { rewrites: boolean; referrer: boolean }>;
}

const FIXTURES = path.resolve(__dirname, "../crates/tine-store/tests/fixtures/cm-conformance");
const EXPECTED = path.join(FIXTURES, "expected-differences.json");
/** The stuck page: a name no corpus text references. */
const STUCK = "Zz stuck page";

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

/** A graph whose one stuck page holds the entry's text as an edit no save
 *  attempt can write (an I/O refusal), as `renameStuckPage.test.ts` does. */
async function stuckGraph(e: Entry, format: "md" | "org"): Promise<void> {
  resetStore();
  vi.restoreAllMocks();
  const ids = e.blocks.map((_, i) => `b${i}`);
  setDoc({
    byId: Object.fromEntries(ids.map((id) => [id, { id, raw: "placeholder", collapsed: false, parent: null, page: STUCK, children: [] as string[] }])),
    pages: [{ name: STUCK, id: `pages/${STUCK}.md`, kind: "page", title: STUCK, preBlock: e.preBlock, roots: ids, format, readOnly: false, guide: false }],
    feed: [STUCK],
    loaded: true,
  });
  activatePageInstance(STUCK);
  installDiskHost(await bindTestHost(), {
    read: (key, name) => ({ id: key, name, kind: "page", title: name, pre_block: null, blocks: [], rev: "r1" }) as PageDto,
    write: () => { throw new Error("io:PermissionDenied"); },
  });
  vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: "pages/New.md" });
  vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "renamed", touched: [] });
  e.blocks.forEach((b, i) => setRaw(ids[i], b.raw, { timetracking: false }));
}

async function renameDiffs(e: Entry, format: "md" | "org"): Promise<string[]> {
  const out: string[] = [];
  // The page's format is the entry's: the stuck page's texts are parsed as it.
  await stuckGraph(e, format);
  for (const [key, rust] of Object.entries(e.renames).sort(([a], [b]) => (a < b ? -1 : 1))) {
    const outcome = await renameOrMergePage(key, "New");
    const blocked = typeof outcome === "object" && outcome.unsaved === STUCK && outcome.mentions;
    if (blocked !== rust.rewrites) {
      const why = rust.rewrites ? "the rename rewrites this text" : "the rename leaves this text alone";
      // A namespace parent: `unsavedPathsFor` refuses on a descendant mention
      // (its prefix test); the backend renames, and so rewrites, a descendant
      // only when the descendant page exists (divergence R8), which this text
      // alone cannot say.
      const parent = e.blocks.some((b) => b.refs.some((r) => r.startsWith(`${key}/`)));
      const ns = !rust.rewrites && parent ? " (namespace parent: rewritten only if the child page exists, R8)" : "";
      out.push(`unsavedPathsFor: ${blocked ? "refuses" : "allows"} renaming [${key}], ${why}${rust.referrer ? "" : " (no evidence reference)"}${ns}`);
    }
  }
  return out;
}

beforeAll(() => initParser());

describe("conceptual model conformance: the frontend's reference walker", () => {
  it("differs from the Rust answers exactly as expected-differences.json records", async () => {
    const now: Record<string, string[]> = {};
    for (const format of ["md", "org"] as const) {
      for (const [id, e] of entries(format)) {
        const diffs = [...swipeDiffs(e, format), ...(await renameDiffs(e, format))].sort();
        if (diffs.length) now[id] = diffs;
      }
    }
    resetStore();
    vi.restoreAllMocks();
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
