import { backend } from "./backend";
import { dataRev } from "./ui";
import type { RefGroup } from "./types";

// Batches inline ((uuid)) reference / embed resolutions: every request made in the
// same microtask tick is coalesced into ONE resolve_blocks IPC instead of one per
// ref, and results are cached for the current dataRev. So a page with N block refs
// costs 1 round-trip (not N), duplicate refs share a result, and an edit (dataRev
// bump) refreshes everything on the next resolve.
let cacheRev = -1;
const cache = new Map<string, Promise<RefGroup | null>>();
let pending = new Map<string, (v: RefGroup | null) => void>();
let scheduled = false;

function flush() {
  scheduled = false;
  if (!pending.size) return;
  const batch = [...pending.keys()];
  const resolvers = pending;
  pending = new Map();
  void backend()
    .resolveBlocks(batch)
    .then((results) => batch.forEach((id, i) => resolvers.get(id)?.(results[i] ?? null)))
    .catch(() => batch.forEach((id) => resolvers.get(id)?.(null)));
}

/** Resolve one block reference — coalesced into a shared batch and memoized for
 *  the current dataRev. */
export function resolveBlockBatched(id: string): Promise<RefGroup | null> {
  const rev = dataRev();
  if (rev !== cacheRev) {
    cache.clear(); // stale after any edit — re-resolve at the new generation
    cacheRev = rev;
  }
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
