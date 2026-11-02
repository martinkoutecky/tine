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
 * the same order; a key's values are offered only once this editor has run a
 * property-name session (before, values came from the facets that session
 * fetched, kept for the editor's lifetime), and are read when first needed.
 * Cost per session: one keys request (O(blocks) in the graph, keys only) plus
 * one values request per completed key. */
export function propertyCompletionSession() {
  type Keys = { value: Promise<string[] | null>; owner: () => boolean };
  let keys: Keys | null = null;
  let named = false;
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
            named = true;
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
     *  arrived for the current session; false when nothing was fetched (no
     *  property-name session yet, already loaded, a failed request) or the
     *  session ended meanwhile. */
    load(key: string): Promise<boolean> {
      if (!named) return Promise.resolve(false);
      const held = values.get(key);
      if (held?.owner()) return Promise.resolve(false);
      if (held) values.delete(key);
      const pending = loading.get(key);
      if (pending) return pending.then(() => false);
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
