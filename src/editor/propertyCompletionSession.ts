import { backend } from "../backend";
import { dbg } from "../debug";
import { graphOwner, readOwned } from "../owned";

/** One editor's property completion (cost-model audit 2026-11-02, row 2b):
 * the graph's property keys are fetched once per trigger session, not on
 * every keystroke, and values only for the key being completed, not every
 * key's values. A session ends when the popup closes (`end`), or when the
 * graph binding changes under it.
 *
 * Same answers as the full-facet fetch it replaces: keys are the same list in
 * the same order; a key's values are read when first needed, including when
 * the key was authored without a property-name session.
 * Cost per session: at most one keys request (O(blocks) in the graph, keys only) plus
 * one values request per completed key. */
export function propertyCompletionSession() {
  type Keys = { value: Promise<string[] | null>; owner: () => boolean };
  let keys: Keys | null = null;
  let generation = 0;
  // Each key's values with the owner of the request that read them: a list
  // read under another graph binding is never offered.
  const values = new Map<string, { list: string[]; owner: () => boolean }>();
  const loading = new Map<string, Promise<boolean>>();

  async function readValues(key: string): Promise<boolean> {
    const at = generation, owner = graphOwner(() => at === generation);
    let result;
    try {
      result = await readOwned(owner, backend().queryFacets(true, { key }));
    } catch (error) {
      // Completion is an optional aid: nothing is cached, so the next
      // keystroke asks again; the list shows no values meanwhile.
      dbg(`property-autocomplete values: ${String(error)}`);
      return false;
    } finally {
      if (at === generation) loading.delete(key);
    }
    if (result.kind === "stale") return false;
    values.set(key, {
      list: result.value.find(([candidate]) => candidate === key)?.[1] ?? [],
      owner,
    });
    return true;
  }

  return {
    /** Every graph property key, once per session; `null` when the graph
     *  changed under the request. Rejects as the request did, and the next
     *  call asks again. */
    keys(): Promise<string[] | null> {
      if (!keys || !keys.owner()) {
        const owner = graphOwner();
        const value = readOwned(owner, backend().queryFacets(true, { keys: true }))
          .then((result) => {
            if (result.kind === "stale") return null;
            return result.value.map(([key]) => key);
          });
        const cached = { value, owner };
        keys = cached;
        value.then(
          (answer) => { if (answer === null && keys === cached) keys = null; },
          () => { if (keys === cached) keys = null; },
        );
      }
      return keys.value;
    },
    /** The key's values already loaded in this session and still owned by
     *  the current graph binding (empty until loaded, or once it changed). */
    values(key: string): string[] {
      const entry = values.get(key);
      return entry?.owner() ? entry.list : [];
    },
    /** Load the key's values once per session. Resolves true when values
     *  arrived for the current session; false when nothing was fetched
     *  (already loaded, a failed request) or the
     *  session ended meanwhile. */
    load(key: string): Promise<boolean> {
      const held = values.get(key);
      if (held?.owner()) return Promise.resolve(false);
      if (held) values.delete(key);
      const pending = loading.get(key);
      // Every query waiting for this answer must refresh on arrival, including
      // a query typed after the request started. The request is still shared.
      if (pending) return pending;
      const request = readValues(key);
      loading.set(key, request);
      return request;
    },
    /** The popup closed: the next trigger starts a fresh session. */
    end() {
      generation += 1;
      keys = null;
      values.clear();
      loading.clear();
    },
  };
}
