import { reportUiFailure } from "./uiFailure";
import { backend } from "./backend";
import { blockRef, node as docNode, resolveGuideBlockRef } from "./document";
import { graphEpoch } from "./graphSession";
import { changed } from "./changeKeys";
import { graphOwner, readOwned } from "./owned";
import type { RefGroup } from "./types";

// Batches inline ((uuid)) reference / embed resolutions: every request made in the
// same microtask tick is coalesced into ONE resolve_blocks IPC instead of one per
// ref. Duplicate refs + re-mounts share the result. Visible IDs re-resolve as
// one batch when their block keys advance after publication (or a full refresh),
// matching OG's reactive entity semantics without rereading unrelated targets.
let cacheRev = -1;
type Answer = RefGroup | null | undefined;
const cache = new Map<string, { revision: string; promise: Promise<Answer> }>();
const resolvedCache = new Map<string, { revision: string; group: RefGroup }>();
let pending = new Map<string, { revision: string; resolve: (v: Answer) => void }>();
let scheduled = false;

function ensureCacheRev() {
  const revision = graphEpoch();
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
  const owner = graphOwner(() => graphEpoch() === batchRev);
  void readOwned(owner, backend().resolveBlocks(batch))
    .then((result) => {
      if (result.kind === "stale") {
        batch.forEach((id) => resolvers.get(id)?.resolve(null));
        return;
      }
      const results = result.value;
      batch.forEach((id, i) => {
        const request = resolvers.get(id)!;
        if (changed(`block:${id}`) !== request.revision) { request.resolve(null); return; }
        // Backend miss → try the virtual in-app Guide (never on disk). No-op for
        // real graphs, so disk resolutions always win.
        const group = results[i] ?? resolveGuideBlockRef(id);
        if (group) resolvedCache.set(id, { revision: request.revision, group });
        request.resolve(group);
      });
    })
    .catch((error) => {
      if (owner()) reportUiFailure("block-resolution", error);
      batch.forEach((id) => {
        if (cache.get(id)?.revision === resolvers.get(id)?.revision) cache.delete(id);
        resolvers.get(id)?.resolve(undefined);
      });
    });
}

/** Resolve one visible block reference — coalesced and memoized for its landed
 *  block revision. `null` means absent; `undefined` means a failed read,
 *  reported through uiFailure and evicted so a later request can retry. */
export function resolveBlockBatched(id: string): Promise<RefGroup | null | undefined> {
  ensureCacheRev();
  const revision = changed(`block:${id}`);
  const hit = cache.get(id);
  if (hit?.revision === revision) return hit.promise;
  pending.get(id)?.resolve(null);
  const p = new Promise<Answer>((resolve) => pending.set(id, { revision, resolve }));
  cache.set(id, { revision, promise: p });
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
  const cached = resolvedCache.get(id);
  return cached?.revision === changed(`block:${id}`) ? cached.group : null;
}

/** Where navigation to block `uuid` should go, given its resolved group `g`.
 *
 *  Contract: a block loaded in the working set answers through the document's
 *  `blockRef` (durable uuid plus its owner's exact path); otherwise the backend
 *  group's page and kind answer. Pure: no navigation, no IPC. The one answerer
 *  for "rendered/raw `((uuid))` → open target", shared by the inline block-ref
 *  click and the Ctrl+O follow-link command (GH #274). */
export function blockRefTarget(
  uuid: string,
  g: RefGroup,
): { uuid: string; page: string; pageKind: RefGroup["kind"]; path?: string } {
  return docNode(uuid) ? blockRef(uuid) : { uuid, page: g.page, pageKind: g.kind };
}
