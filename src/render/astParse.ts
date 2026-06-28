import { backend } from "../backend";
import { graphEpoch } from "../ui";
import type { Block } from "./ast";

// Batches block-body parses: every parseBlockBatched call made in the same
// microtask tick is coalesced into ONE parse_blocks IPC (per format), and each
// distinct body text is parsed at most once (memoized) until its text changes.
// Mirrors resolveBatch.ts. The body text is the marker/scheduled/property-stripped
// `view.lines` (Block.tsx) — NOT the raw — so block-header chrome stays in
// blockView and the AST renders only the visible body (no double-render).
let cacheRev = -1;
const cache = new Map<string, Promise<Block[]>>();
let pendingMd: { raw: string; resolve: (v: Block[]) => void }[] = [];
let pendingOrg: { raw: string; resolve: (v: Block[]) => void }[] = [];
let scheduled = false;

function flush() {
  scheduled = false;
  const dispatch = (isOrg: boolean, batch: { raw: string; resolve: (v: Block[]) => void }[]) => {
    if (!batch.length) return;
    const raws = batch.map((b) => b.raw);
    void backend()
      .parseBlocks(raws, isOrg)
      .then((results) => batch.forEach((b, i) => b.resolve(results[i] ?? [])))
      .catch(() => batch.forEach((b) => b.resolve([])));
  };
  const md = pendingMd;
  const org = pendingOrg;
  pendingMd = [];
  pendingOrg = [];
  dispatch(false, md);
  dispatch(true, org);
}

/** Parse one block-body text to its lsdoc `Block[]` — coalesced into a shared
 *  batch and memoized until the text (or graph) changes. */
export function parseBlockBatched(text: string, isOrg: boolean): Promise<Block[]> {
  const epoch = graphEpoch();
  if (epoch !== cacheRev) {
    cache.clear();
    cacheRev = epoch;
  }
  const key = (isOrg ? "o\n" : "m\n") + text;
  const hit = cache.get(key);
  if (hit) return hit;
  // Editing churns a fresh key per keystroke-committed body; bound the memo so a
  // long session doesn't grow it without limit (re-parses are cheap + batched).
  if (cache.size > 8000) cache.clear();
  const p = new Promise<Block[]>((resolve) => {
    (isOrg ? pendingOrg : pendingMd).push({ raw: text, resolve });
  });
  cache.set(key, p);
  if (!scheduled) {
    scheduled = true;
    queueMicrotask(flush);
  }
  return p;
}
