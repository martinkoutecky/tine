import { backend } from "./backend";
import { graphEpoch } from "./ui";
import type { RefGroup } from "./types";

// Batches inline ((uuid)) reference / embed resolutions: every request made in the
// same microtask tick is coalesced into ONE resolve_blocks IPC instead of one per
// ref, and each uuid is resolved at most ONCE per open graph (cached). So a page
// with N refs costs 1 round-trip (not N), duplicate refs + re-mounts share the
// result, and — deliberately, to minimize CPU on a throttled machine — refs are
// NOT re-resolved on every edit (they refresh on graph switch / reopen).
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
 *  the lifetime of the open graph. */
export function resolveBlockBatched(id: string): Promise<RefGroup | null> {
  const epoch = graphEpoch();
  if (epoch !== cacheRev) {
    cache.clear(); // new graph — drop the previous graph's resolutions
    cacheRev = epoch;
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
