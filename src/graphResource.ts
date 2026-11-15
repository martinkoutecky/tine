import { createMemo, createResource, type Accessor, type ResourceFetcher, type ResourceOptions, type ResourceReturn } from "solid-js";
import { graphEpoch } from "./graphSession";

type Keyed<S> = { readonly value: S; readonly epoch: number };

/** A Solid resource whose answer belongs to the open graph's display epoch:
 * the one way to create a resource whose fetcher reads through `graphOwner`
 * (src/owned.ts). `graphOwner` retires a read when `graphEpoch` moves (graph
 * open or switch, rename, typography, journal-title format); keying the
 * resource on the same epoch makes Solid ask again, against the graph now
 * shown, instead of keeping the retired (empty or older) answer as final, and
 * makes a view that outlives a graph switch re-read the new graph.
 * `src/graphResource.guard.test.ts` holds every `graphOwner` fetcher to it.
 *
 * `source` follows `createResource`: `false`, `null` or `undefined` skips the
 * read; a value is compared with `===` (return a string or a stable value).
 * Cost: one memo; one extra read per mounted resource per epoch move, never
 * per keystroke or save. */
export function createGraphResource<T, S, R = unknown>(
  source: Accessor<S | false | null | undefined>,
  fetcher: ResourceFetcher<S, T, R>,
  options?: ResourceOptions<T, Keyed<S>>,
): ResourceReturn<T, R> {
  const key = createMemo<Keyed<S> | undefined>(() => {
    const value = source();
    return value === false || value === null || value === undefined ? undefined : { value, epoch: graphEpoch() };
  }, undefined, { equals: (a, b) => a === b || (!!a && !!b && a.value === b.value && a.epoch === b.epoch) });
  return createResource(key, (keyed, info) => fetcher(keyed.value, info), options);
}
