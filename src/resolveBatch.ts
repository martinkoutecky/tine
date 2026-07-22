import { backend } from "./backend";
import { resolveGuideBlockRef } from "./store";
import { dataRev, graphEpoch } from "./ui";
import type { RefGroup } from "./types";

// Batches inline ((uuid)) reference / embed resolutions: every request made in the
// same microtask tick is coalesced into ONE resolve_blocks IPC instead of one per
// ref. Duplicate refs + re-mounts share the result. Visible UUIDs re-resolve as
// one batch only after a graph transaction lands (`dataRev`), matching OG's
// reactive UUID-entity semantics without doing graph work on every keystroke.
let cacheRev = "";
const cache = new Map<string, Promise<RefGroup | null>>();
const resolvedCache = new Map<string, RefGroup>();
let pending = new Map<string, (v: RefGroup | null) => void>();
let scheduled = false;

function ensureCacheRev() {
  const revision = `${graphEpoch()}\0${dataRev()}`;
  if (revision !== cacheRev) {
    cache.clear();
    resolvedCache.clear();
    cacheRev = revision;
  }
  return revision;
}

function flush() {
  scheduled = false;
  if (!pending.size) return;
  const batch = [...pending.keys()];
  const resolvers = pending;
  const batchRev = cacheRev;
  pending = new Map();
  void backend()
    .resolveBlocks(batch)
    .then((results) =>
      batch.forEach((id, i) => {
        // Backend miss → try the virtual in-app Guide (never on disk). No-op for
        // real graphs, so disk resolutions always win.
        const group = results[i] ?? resolveGuideBlockRef(id);
        if (group && cacheRev === batchRev && ensureCacheRev() === batchRev) resolvedCache.set(id, group);
        resolvers.get(id)?.(group);
      })
    )
    .catch(() =>
      batch.forEach((id) => resolvers.get(id)?.(resolveGuideBlockRef(id)))
    );
}

/** Resolve one visible block reference — coalesced and memoized for the current
 *  landed graph revision. */
export function resolveBlockBatched(id: string): Promise<RefGroup | null> {
  ensureCacheRev();
  const hit = cache.get(id);
  if (hit) return hit;
  const p = new Promise<RefGroup | null>((resolve) => pending.set(id, resolve));
  cache.set(id, p);
  if (!scheduled) {
    scheduled = true;
    queueMicrotask(flush);
  }
  return p;
}

/** Best-effort synchronous lookup of a block reference already resolved by the
 *  async batched path in this graph epoch. */
export function resolvedBlockRefSync(id: string): RefGroup | null {
  ensureCacheRev();
  return resolvedCache.get(id) ?? null;
}
