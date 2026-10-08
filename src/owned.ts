/** Owned asynchronous work. A caller captures an owner before starting work and
 * reads its result through `readOwned`; a retired owner yields `stale`, never a
 * value that can be applied. Owners compose for graph bindings, route or tab
 * intent, and the newest request for one key. Revision owners also cover device
 * preferences. `writeOwned` reports durable failures despite owner retirement;
 * `readOwnedResource` releases a stale resource. `serializeOwned` orders writes
 * for one resource and rejects same-key calls made synchronously inside work.
 * `serializeDurable` orders writes and preserves their failures. An owner check
 * costs O(number of composed predicates); the wrappers add O(1) work beyond
 * that check and the supplied operation. A queued call waits for the cumulative
 * duration of earlier calls with the same object key. Read failures are hidden
 * only when the owner has retired; durable failures always reject. */
import { bindingCurrent, captureBinding, stillBound } from "./binding";

declare const displayBrand: unique symbol;
/** A predicate owner. `[displayBrand]` is present (true) only on an owner whose
 * token includes the display epoch (`graphOwner`, or a composition of one). */
export type Owner = (() => boolean) & { readonly [displayBrand]?: boolean };
/** R4 / I-20: an owner whose token is the display epoch as well as the graph
 * binding. Right for render and read-only results; never for a durable write. */
export type DisplayOwner = (() => boolean) & { readonly [displayBrand]: true };
/** An owner that carries no display epoch: the only kind `writeOwned` and
 * `serializeDurable` accept, so a repaint (typography, journal title format,
 * another page's rename) cannot retire a write's success or failure handling.
 * A plain predicate, `bindingOwner`, `ownedWhen`/`latestOwner`/`revisionOwner`
 * over write owners all qualify. Exemplar: `bindingOwner` in src/owned.ts. */
export type WriteOwner = (() => boolean) & { readonly [displayBrand]?: never };
export type Owned<T> = { kind: "current"; value: T } | { kind: "stale" };
const STALE: Owned<never> = Object.freeze({ kind: "stale" });
const revisions = new WeakMap<object, number>();
const latest = new WeakMap<object, Map<string, number>>();
const queues = new WeakMap<object, Promise<void>>();
const invoking = new WeakSet<object>();

/** Capture the implicit current graph epoch, reset generation, and backend
 * binding generation, then compose optional live predicates. No graph object is
 * captured. Construction is O(1); each check is O(number of predicates).
 * Backend binding or supplied predicates may throw. */
export function graphOwner(...live: Owner[]): DisplayOwner {
  const binding = captureBinding();
  return (() => stillBound(binding) && live.every((predicate) => predicate())) as DisplayOwner;
}

/** Capture the current graph binding (store reset + backend binding
 * generation, NOT the display epoch), then compose optional live predicates.
 * The owner for a durable write and the bookkeeping of its outcome (R4 / I-20).
 * Construction is O(1); each check is O(number of predicates). */
export function bindingOwner(...live: WriteOwner[]): WriteOwner {
  const binding = captureBinding();
  return () => bindingCurrent(binding) && live.every((predicate) => predicate());
}

/** Compose route, tab or surface predicates with another owner. O(number of
 * predicates); a predicate failure is observable to the caller. Composing a
 * display owner yields a display owner. */
export function ownedWhen(...live: WriteOwner[]): WriteOwner;
export function ownedWhen(...live: Owner[]): DisplayOwner;
export function ownedWhen(...live: Owner[]): Owner {
  return () => live.every((predicate) => predicate());
}

/** Capture the newest request for a resource key within a scope. Supersedes
 * older requests for that key only. Construction is O(1); each check is
 * O(number of live predicates), which may throw. */
export function latestOwner(scope: object, key: string, ...live: WriteOwner[]): WriteOwner;
export function latestOwner(scope: object, key: string, ...live: Owner[]): DisplayOwner;
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
export function revisionOwner(key: object, revision: number, ...live: WriteOwner[]): WriteOwner;
export function revisionOwner(key: object, revision: number, ...live: Owner[]): DisplayOwner;
export function revisionOwner(key: object, revision: number, ...live: Owner[]): Owner {
  return () => currentRevision(key) === revision && live.every((predicate) => predicate());
}

/** Read a completion only while its owner remains current. The supplied work
 * has already started. A current work failure rejects unchanged; a stale work
 * failure or success yields `stale`. Owner checks can reject independently.
 * The caller must branch on `kind`. */
export async function readOwned<T>(owner: Owner, work: Promise<T>): Promise<Owned<T>> {
  let value: T;
  try { value = await work; }
  catch (error) {
    if (owner()) throw error;
    return STALE;
  }
  return owner() ? { kind: "current", value } : STALE;
}

/** Observe a caller-supplied durable write regardless of UI ownership. The
 * type cannot prove the work is durable; callers must pass the write promise
 * and report its rejection. Failures reject unchanged, including after owner
 * retirement. Ownership filters only the successful return value. */
export async function writeOwned<T>(owner: WriteOwner, work: Promise<T>): Promise<Owned<T>> {
  const value = await work;
  return owner() ? { kind: "current", value } : STALE;
}

/** Read a resource-bearing completion. Stale success runs cleanup once before
 * yielding `stale`; current work failure rejects and stale work failure yields
 * `stale`. Owner or cleanup failures reject (cleanup can replace an owner
 * failure). Ownership is checked once after success: a later retirement needs
 * the caller's ordinary disposal path. */
export async function readOwnedResource<T>(owner: Owner, work: Promise<T>, cleanup: (value: T) => void | Promise<void>): Promise<Owned<T>> {
  let value: T;
  try { value = await work; }
  catch (error) {
    if (owner()) throw error;
    return STALE;
  }
  let current: boolean;
  try { current = owner(); }
  catch (error) { await cleanup(value); throw error; }
  if (current) return { kind: "current", value };
  await cleanup(value);
  return STALE;
}

/** Run one write after earlier writes for the same object identity. The queue
 * is shared with serializeDurable. A stale owner skips work at dequeue; after
 * start, readOwned determines the outcome. A same-key call made synchronously
 * inside `work` rejects immediately. Calls made after an await cannot be
 * distinguished from outside callers; do not await a nested same-key call.
 * A never-settling operation blocks later calls for that key. An owner error
 * at dequeue rejects this call and lets the queue advance. */
export function serializeOwned<T>(key: object, owner: WriteOwner, work: () => Promise<T>): Promise<Owned<T>> {
  return serialize(key, owner, work, readOwned, "serializeOwned");
}

/** Queue caller-supplied durable work by object identity, sharing the queue
 * with serializeOwned. A stale owner skips work at dequeue; an already-started
 * failure rejects unchanged even after retirement, while a stale success has
 * no delivered value. Same-key synchronous nesting rejects. The same after-
 * await limitation as serializeOwned applies. An owner error at dequeue rejects
 * this call and lets the queue advance. Callers must report failures. */
export function serializeDurable<T>(key: object, owner: WriteOwner, work: () => Promise<T>): Promise<Owned<T>> {
  return serialize(key, owner, work, writeOwned, "serializeDurable");
}

function serialize<T, O extends Owner>(key: object, owner: O, work: () => Promise<T>, read: (owner: O, work: Promise<T>) => Promise<Owned<T>>, name: string): Promise<Owned<T>> {
  if (invoking.has(key)) return Promise.reject(new Error(`${name}: reentrant same key`));
  const before = queues.get(key) ?? Promise.resolve();
  const result = before.then(() => {
    if (!owner()) return STALE as Owned<T>;
    invoking.add(key);
    try { return read(owner, work()); }
    finally { invoking.delete(key); }
  });
  const settled = result.then(() => {}, () => {});
  queues.set(key, settled);
  void settled.then(() => { if (queues.get(key) === settled) queues.delete(key); });
  return result;
}
