import { reportUiFailure } from "./uiFailure";
import { backend } from "./backend";
import { blockRef, node as docNode, resolveGuideBlockRef } from "./document";
import { graphEpoch } from "./graphSession";
import { changeKeyHasConsumers, onChangeKeyReleased, onChangeKeysFlushed } from "./changeKeys";
import { clearOnBindingInvalidated } from "./binding";
import { createComputed, createRoot } from "solid-js";
import { graphOwner, readOwned } from "./owned";
import type { RefGroup } from "./types";

// Batches inline ((uuid)) reference / embed resolutions: every request made in the
// same microtask tick is coalesced into ONE resolve_blocks IPC instead of one per
// ref. Duplicate refs + re-mounts share the result. Visible IDs re-resolve as
// one batch when their block keys advance after publication (or a full refresh),
// matching OG's reactive entity semantics without rereading unrelated targets.
let cacheRev = -1;
type Answer = RefGroup | null | undefined;
interface Request {
  promise: Promise<Answer>; resolve: (value: Answer) => void;
  completed: boolean; invalidated: boolean; group?: RefGroup | null; bytes: number;
}
const cache = new Map<string, Request>();
let pending = new Map<string, Request>();
let scheduled = false;
const MAX_BYTES = 8 * 1024 * 1024;
const MAX_ENTRY_BYTES = 1024 * 1024;
const MAX_ENTRIES = 512;

export const resolutionCacheStats = () => ({ entries: cache.size, bytes: [...cache.values()].reduce((n, entry) => n + entry.bytes, 0) });

function trim(): void {
  let inactiveBytes = 0, inactiveEntries = 0;
  for (const [id, entry] of cache) {
    if (!entry.completed || changeKeyHasConsumers(`block:${id}`)) continue;
    if (entry.bytes > MAX_ENTRY_BYTES) { cache.delete(id); continue; }
    inactiveBytes += entry.bytes; inactiveEntries++;
  }
  for (const [id, entry] of cache) {
    if (inactiveBytes <= MAX_BYTES && inactiveEntries <= MAX_ENTRIES) break;
    if (!entry.completed || changeKeyHasConsumers(`block:${id}`)) continue;
    cache.delete(id); inactiveBytes -= entry.bytes; inactiveEntries--;
  }
}

function clear(): void {
  for (const request of cache.values()) { request.invalidated = true; request.resolve(null); }
  cache.clear();
  pending.clear();
}
clearOnBindingInvalidated(clear);
onChangeKeyReleased(trim);
onChangeKeysFlushed(affected => {
  for (const [id, entry] of cache) {
    if (affected && !affected.has(`block:${id}`)) continue;
    entry.invalidated = true;
    cache.delete(id);
    if (pending.get(id) === entry) { pending.delete(id); entry.resolve(null); }
  }
  trim();
});
// Eager epoch cleanup also covers a graph/display switch with no subsequent read.
createRoot(() => createComputed(() => { ensureCacheRev(); }));

function ensureCacheRev() {
  const revision = graphEpoch();
  if (revision !== cacheRev) {
    clear();
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
        if (request.invalidated) { request.resolve(null); return; }
        // Backend miss → try the virtual in-app Guide (never on disk). No-op for
        // real graphs, so disk resolutions always win.
        const group = results[i] ?? resolveGuideBlockRef(id);
        request.completed = true;
        request.group = group;
        request.bytes = id.length * 2 + (group ? JSON.stringify(group).length * 8 : 32) + 256;
        request.resolve(group);
      });
      trim();
    })
    .catch((error) => {
      if (owner()) reportUiFailure("block-resolution", error);
      batch.forEach((id) => {
        if (cache.get(id) === resolvers.get(id)) cache.delete(id);
        resolvers.get(id)?.resolve(undefined);
      });
    });
}

/** Resolve one visible block reference — coalesced and memoized for its landed
 *  block revision. `null` means absent; `undefined` means a failed read,
 *  reported through uiFailure and evicted so a later request can retry. */
export function resolveBlockBatched(id: string): Promise<RefGroup | null | undefined> {
  ensureCacheRev();
  const hit = cache.get(id);
  if (hit) { cache.delete(id); cache.set(id, hit); return hit.promise; }
  let resolve!: Request["resolve"];
  const promise = new Promise<Answer>(complete => { resolve = complete; });
  const request: Request = { promise, resolve, completed: false, invalidated: false, bytes: 0 };
  pending.set(id, request);
  cache.set(id, request);
  if (!scheduled) {
    scheduled = true;
    queueMicrotask(flush);
  }
  return promise;
}

/** Best-effort synchronous lookup of a block reference already resolved by the
 *  async batched path in this graph epoch. */
export function resolvedBlockRefSync(id: string): RefGroup | null {
  ensureCacheRev();
  return cache.get(id)?.group ?? null;
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
