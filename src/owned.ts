/** Owned asynchronous work. A caller captures an owner before starting work and
 * reads its result through `readOwned`; a retired owner yields `stale`, never a
 * value that can be applied. Owners compose for graph bindings, route or tab
 * intent, and the newest request for one key. Revision owners also cover device
 * preferences. `serializeOwned` orders writes for one resource. Every operation
 * costs O(1) frontend work plus the caller's asynchronous work; serialization
 * adds the number of earlier writes for the same key to its wait. Current-owner
 * failures reject as supplied by the work; stale failures yield `stale`. Callers
 * handle current failures and need not know the counters or queue state. */
import { captureBinding, stillBound } from "./binding";

export type Owner = () => boolean;
export type Owned<T> = { kind: "current"; value: T } | { kind: "stale" };
const STALE: Owned<never> = Object.freeze({ kind: "stale" });
const revisions = new WeakMap<object, number>();
const latest = new WeakMap<object, Map<string, number>>();
const queues = new WeakMap<object, Promise<void>>();

/** Compose captured graph binding with optional live predicates. O(1); never
 * throws except when a supplied predicate throws. */
export function graphOwner(...live: Owner[]): Owner {
  const binding = captureBinding();
  return () => stillBound(binding) && live.every((predicate) => predicate());
}

/** Compose route, tab or surface predicates with another owner. O(number of
 * predicates); a predicate failure is observable to the caller. */
export function ownedWhen(...live: Owner[]): Owner {
  return () => live.every((predicate) => predicate());
}

/** Capture the newest request for a resource key within a scope. Supersedes
 * older requests for that key only. O(1); no failure. */
export function latestOwner(scope: object, key: string, ...live: Owner[]): Owner {
  let keys = latest.get(scope);
  if (!keys) { keys = new Map(); latest.set(scope, keys); }
  const revision = (keys.get(key) ?? 0) + 1;
  keys.set(key, revision);
  return () => keys!.get(key) === revision && live.every((predicate) => predicate());
}

/** Increment a stable object's revision before a write. O(1); no failure. */
export function advanceRevision(key: object): number {
  const next = (revisions.get(key) ?? 0) + 1;
  revisions.set(key, next);
  return next;
}

/** Read a stable object's revision without changing it. O(1); no failure. */
export function currentRevision(key: object): number {
  return revisions.get(key) ?? 0;
}

/** Own one revision, optionally subject to more live predicates. O(number of
 * predicates); a predicate failure is observable to the caller. */
export function revisionOwner(key: object, revision: number, ...live: Owner[]): Owner {
  return () => currentRevision(key) === revision && live.every((predicate) => predicate());
}

/** Read a completion only while its owner remains current. O(1) beyond the
 * supplied promise; a current failure rejects unchanged, while a stale result
 * or stale failure yields `stale`. The caller must branch on `kind`. */
export async function readOwned<T>(owner: Owner, work: Promise<T>): Promise<Owned<T>> {
  try {
    const value = await work;
    return owner() ? { kind: "current", value } : STALE;
  } catch (error) {
    if (owner()) throw error;
    return STALE;
  }
}

/** Run one write after earlier writes for this resource. O(1) queue work plus
 * the wait for earlier writes; a stale owner skips the write, current failures
 * reject, and a stale completion yields `stale`. Callers handle failures. */
export function serializeOwned<T>(key: object, owner: Owner, work: () => Promise<T>): Promise<Owned<T>> {
  const before = queues.get(key) ?? Promise.resolve();
  const result = before.then(() => owner() ? readOwned(owner, work()) : STALE as Owned<T>);
  const settled = result.then(() => {}, () => {});
  queues.set(key, settled);
  void settled.then(() => { if (queues.get(key) === settled) queues.delete(key); });
  return result;
}
