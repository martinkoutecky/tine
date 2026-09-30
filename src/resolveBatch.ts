import { reportUiFailure } from "./uiFailure";
import { backend } from "./backend";
import { blockRef, node as docNode, resolveGuideBlockRef } from "./document";
import { dataRev, graphEpoch } from "./graphSession";
import { graphOwner, readOwned } from "./owned";
import type { RefGroup } from "./types";

// Batches inline ((uuid)) reference / embed resolutions: every request made in the
// same microtask tick is coalesced into ONE resolve_blocks IPC instead of one per
// ref. Duplicate refs + re-mounts share the result. Visible UUIDs re-resolve as
// one batch only after a graph transaction lands (`dataRev`), matching OG's
// reactive UUID-entity semantics without doing graph work on every keystroke.
let cacheRev = "";
const cache = new Map<string, Promise<RefGroup | null | undefined>>();
const resolvedCache = new Map<string, RefGroup>();
let pending = new Map<string, (v: RefGroup | null | undefined) => void>();
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
  const owner = graphOwner(() => ensureCacheRev() === batchRev);
  void readOwned(owner, backend().resolveBlocks(batch))
    .then((result) => {
      if (result.kind === "stale") {
        batch.forEach((id) => resolvers.get(id)?.(null));
        return;
      }
      const results = result.value;
      batch.forEach((id, i) => {
        // Backend miss → try the virtual in-app Guide (never on disk). No-op for
        // real graphs, so disk resolutions always win.
        const group = results[i] ?? resolveGuideBlockRef(id);
        if (group && cacheRev === batchRev && ensureCacheRev() === batchRev) resolvedCache.set(id, group);
        resolvers.get(id)?.(group);
      });
    })
    .catch((error) => {
      if (owner()) reportUiFailure("block-resolution", error);
      batch.forEach((id) => {
        if (cacheRev === batchRev) cache.delete(id);
        resolvers.get(id)?.(undefined);
      });
    });
}

/** Resolve one visible block reference — coalesced and memoized for the current
 *  landed graph revision. `null` means absent; `undefined` means a failed read,
 *  reported through uiFailure and evicted so a later request can retry. */
export function resolveBlockBatched(id: string): Promise<RefGroup | null | undefined> {
  ensureCacheRev();
  const hit = cache.get(id);
  if (hit) return hit;
  const p = new Promise<RefGroup | null | undefined>((resolve) => pending.set(id, resolve));
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
