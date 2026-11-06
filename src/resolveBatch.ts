import { reportUiFailure } from "./uiFailure";
import { backend } from "./backend";
import { blockRef, resolveBlockRef, resolveGuideBlockRef } from "./document";
import { graphEpoch } from "./graphSession";
import { changeKeyHasConsumers, onChangeKeyAcquired, onChangeKeyReleased, onChangeKeysFlushed } from "./changeKeys";
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
  id: string; accessed: number; inactiveIndex: number;
  promise: Promise<Answer>; resolve: (value: Answer | PromiseLike<Answer>) => void;
  completed: boolean; invalidated: boolean; group?: RefGroup | null; bytes: number;
  /** The block's key advanced before the answer: the caller is answered by a fresh ask. */
  reask?: boolean;
}
const cache = new Map<string, Request>();
let pending = new Map<string, Request>();
let scheduled = false;
const MAX_BYTES = 8 * 1024 * 1024;
const MAX_ENTRY_BYTES = 1024 * 1024;
const MAX_ENTRIES = 512;
// Indexed min-heap of inactive answers, ordered by last cache access (not by
// completion/release order). Active answers never participate in maintenance.
const inactive: Request[] = [];
let inactiveBytes = 0;
let totalBytes = 0;
let accessSequence = 0;

export const resolutionCacheStats = () => ({ entries: cache.size, bytes: totalBytes });

function swapInactive(a: number, b: number): void {
  [inactive[a], inactive[b]] = [inactive[b], inactive[a]];
  inactive[a].inactiveIndex = a;
  inactive[b].inactiveIndex = b;
}
function positionInactive(index: number): void {
  while (index > 0) {
    const parent = (index - 1) >> 1;
    if (inactive[parent].accessed <= inactive[index].accessed) break;
    swapInactive(index, parent); index = parent;
  }
  while (2 * index + 1 < inactive.length) {
    let child = 2 * index + 1;
    if (child + 1 < inactive.length && inactive[child + 1].accessed < inactive[child].accessed) child++;
    if (inactive[index].accessed <= inactive[child].accessed) break;
    swapInactive(index, child); index = child;
  }
}
function activate(entry: Request): void {
  const index = entry.inactiveIndex;
  if (index < 0) return;
  const last = inactive.pop()!;
  if (index < inactive.length) {
    inactive[index] = last; last.inactiveIndex = index;
    positionInactive(index);
  }
  entry.inactiveIndex = -1;
  inactiveBytes -= entry.bytes;
}
function remove(entry: Request): void {
  activate(entry);
  if (cache.get(entry.id) !== entry) return;
  cache.delete(entry.id);
  totalBytes -= entry.bytes;
}
function deactivate(entry: Request): void {
  if (!entry.completed || entry.inactiveIndex >= 0) return;
  if (entry.bytes > MAX_ENTRY_BYTES) { remove(entry); return; }
  inactiveBytes += entry.bytes;
  entry.inactiveIndex = inactive.length;
  inactive.push(entry);
  positionInactive(entry.inactiveIndex);
}
function trim(): void {
  while (inactiveBytes > MAX_BYTES || inactive.length > MAX_ENTRIES) remove(inactive[0]);
}

function clear(): void {
  for (const request of cache.values()) { request.invalidated = true; request.resolve(null); }
  cache.clear();
  pending.clear();
  inactive.length = 0;
  inactiveBytes = totalBytes = 0;
}
clearOnBindingInvalidated(clear);
onChangeKeyAcquired(key => {
  if (!key.startsWith("block:")) return;
  const entry = cache.get(key.slice(6));
  if (entry) activate(entry);
});
onChangeKeyReleased(keys => {
  for (const key of keys) {
    if (!key.startsWith("block:")) continue;
    const entry = cache.get(key.slice(6));
    if (entry && !changeKeyHasConsumers(key)) deactivate(entry);
  }
  trim();
});
onChangeKeysFlushed(affected => {
  const superseded: [string, Request][] = [];
  const ids = affected ? [...affected].filter(key => key.startsWith("block:")).map(key => key.slice(6)) : cache.keys();
  for (const id of ids) {
    const entry = cache.get(id);
    if (!entry) continue;
    entry.invalidated = true;
    entry.reask = true;
    remove(entry);
    if (pending.get(id) === entry) { pending.delete(id); superseded.push([id, entry]); }
  }
  for (const [id, entry] of superseded) entry.resolve(resolveBlockBatched(id));
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
        if (request.invalidated) { request.resolve(request.reask ? resolveBlockBatched(id) : null); return; }
        // Backend miss → try the virtual in-app Guide (never on disk). No-op for
        // real graphs, so disk resolutions always win.
        const group = results[i] ?? resolveGuideBlockRef(id);
        request.completed = true;
        request.group = group;
        request.bytes = id.length * 2 + (group ? JSON.stringify(group).length * 8 : 32) + 256;
        totalBytes += request.bytes;
        if (!changeKeyHasConsumers(`block:${id}`)) deactivate(request);
        request.resolve(group);
        trim();
      });
      trim();
    })
    .catch((error) => {
      if (owner()) reportUiFailure("block-resolution", error);
      batch.forEach((id) => {
        if (cache.get(id) === resolvers.get(id)) remove(resolvers.get(id)!);
        resolvers.get(id)?.resolve(undefined);
      });
    });
}

/** Resolve one visible block reference — coalesced and memoized for its landed
 *  block revision. `null` means absent, or that the graph was retired meanwhile
 *  (its owner is stale); a block whose key advances during the lookup is asked
 *  again, so a save of the target never reads as absent. `undefined` means a
 *  failed read, reported through uiFailure and evicted so a later request can
 *  retry. */
export function resolveBlockBatched(id: string): Promise<RefGroup | null | undefined> {
  ensureCacheRev();
  const hit = cache.get(id);
  if (hit) {
    hit.accessed = ++accessSequence;
    if (hit.inactiveIndex >= 0) positionInactive(hit.inactiveIndex);
    return hit.promise;
  }
  let resolve!: Request["resolve"];
  const promise = new Promise<Answer>(complete => { resolve = complete; });
  const request: Request = { id, accessed: ++accessSequence, inactiveIndex: -1, promise, resolve, completed: false, invalidated: false, bytes: 0 };
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
  const ref = { uuid, page: g.page, pageKind: g.kind, ...(g.path ? { path: g.path } : {}) };
  const id = resolveBlockRef(ref);
  return id ? blockRef(id) : ref;
}
