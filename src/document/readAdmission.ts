/** Read and synchronously install a working-set answer only while both its
 * caller and every returned page generation still apply. A superseded answer
 * gets one reread per observed invalidation while wanted; it never installs
 * older text. Owner retirement or a cancelled commit returns stale.
 * Optional scheduling checks at the actual commit, after the queue wait.
 * Cost: at most N+1 sequential reads for N relevant invalidations during the
 * admission; coalesced changes need only one reread. O(returned page names)
 * per comparison, O(1) retained retry state, no polling or graph walk.
 * Owned read/install failures reject. Generations are released on every exit. */
import { readOwned, type Owned, type Owner } from "../owned";
import { pageIdentityKey } from "../pageIdentity";
import { clearOnBindingInvalidated } from "../binding";
import { untrack } from "solid-js";
import { initParser, parserReady } from "../render/parse";

let clock = 0;
let readers = 0;
const generations = new Map<string, number>();
clearOnBindingInvalidated(() => generations.clear());

/** Earlier reads may no longer describe this page. O(1), no I/O or reactive
 * write. The last read completion or a retired binding clears the map. */
export function invalidatePageRead(name: string): void {
  if (readers) invalidatePageReadKey(pageIdentityKey(name));
  else ++clock;
}

/** A publication already carries Rust's key; its boundary normalization is
 * not idempotent. Share the generation update without normalizing it again. */
export function invalidatePageReadKey(key: string): void {
  const generation = ++clock;
  if (readers) generations.set(key, generation);
}

function capturePageRead() {
  const issued = clock;
  readers++;
  return {
    current: (names: readonly string[]) => names.every(name => (generations.get(pageIdentityKey(name)) ?? 0) <= issued),
    release() { if (--readers === 0) generations.clear(); },
  };
}

export async function admitRead<T, R>(owner: Owner, read: () => Promise<T>, names: (value: T) => readonly string[], install: (value: T) => R & (R extends PromiseLike<unknown> ? never : unknown),
  schedule?: (commit: () => Owned<R>) => Promise<Owned<R> | undefined>): Promise<Owned<R>> {
  const started = clock;
  for (let attempt = 0; attempt <= clock - started && owner(); attempt++) {
    if (!untrack(parserReady)) {
      const initialized = await readOwned(owner, initParser());
      if (initialized.kind === "stale" || !owner()) return { kind: "stale" };
    }
    const generation = capturePageRead();
    try {
      const result = await readOwned(owner, read());
      if (result.kind === "stale") return result;
      let superseded = false;
      const commit = (): Owned<R> => {
        if (!owner()) return { kind: "stale" };
        superseded = !generation.current(names(result.value));
        return superseded ? { kind: "stale" } : { kind: "current", value: install(result.value) };
      };
      const admitted = schedule ? await schedule(commit) ?? { kind: "stale" as const } : commit();
      if (!superseded) return admitted;
      // Only a newer relevant generation can spend another read. Successful
      // reads must not exhaust a fixed retry cap and masquerade as retirement.
    } finally { generation.release(); }
  }
  return { kind: "stale" };
}
