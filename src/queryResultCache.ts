// Share identical query IPC work across split panes without turning results into
// another graph-lifetime cache. In-flight promises are strongly held; completed
// DTO objects are held weakly, so mounted consumers can share the same object but
// the GC may reclaim it once no view uses it.

const inFlight = new Map<string, Promise<object>>();
const resolved = new Map<string, WeakRef<object>>();
const MAX_RESOLVED_KEYS = 128;
let currentScope = "";
let scopeGeneration = 0;

function enterScope(scope: string): number {
  if (scope !== currentScope) {
    currentScope = scope;
    scopeGeneration++;
    inFlight.clear();
    resolved.clear();
  }
  return scopeGeneration;
}

export function sharedQueryResult<T extends object>(
  scope: string,
  key: string,
  load: () => Promise<T>,
): Promise<T> {
  const generation = enterScope(scope);
  const cacheKey = `${scope}\0${key}`;
  const prior = resolved.get(cacheKey)?.deref() as T | undefined;
  if (prior) {
    const ref = resolved.get(cacheKey)!;
    resolved.delete(cacheKey);
    resolved.set(cacheKey, ref);
    return Promise.resolve(prior);
  }
  // Do not accumulate dead WeakRef keys over a long editing session.
  if (resolved.has(cacheKey)) resolved.delete(cacheKey);
  const running = inFlight.get(cacheKey) as Promise<T> | undefined;
  if (running) return running;

  const promise = load().then((value) => {
    if (generation === scopeGeneration && scope === currentScope) {
      resolved.set(cacheKey, new WeakRef(value));
      while (resolved.size > MAX_RESOLVED_KEYS) {
        const oldest = resolved.keys().next().value as string | undefined;
        if (oldest === undefined) break;
        resolved.delete(oldest);
      }
    }
    return value;
  });
  inFlight.set(cacheKey, promise);
  void promise.finally(() => {
    if (inFlight.get(cacheKey) === promise) inFlight.delete(cacheKey);
  }).catch(() => {});
  return promise;
}

export function resetSharedQueryResultsForTests(): void {
  currentScope = "";
  scopeGeneration++;
  inFlight.clear();
  resolved.clear();
}
