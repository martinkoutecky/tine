import { createComputed, createRoot, untrack } from "solid-js";
import type { Format } from "./ast";
import type { Facets } from "./facets";

interface Seed { facets: Facets; owners: number }
const seeded = new Map<string, Seed>();
const owners = new Set<() => void>();
let pending: (() => void)[] = [];
export const facetKey = (raw: string, format: Format) => format + "\0" + raw;
export const seededFacetCount = () => seeded.size;
export const seededFacetsOf = (raw: string, format: Format) => seeded.get(facetKey(raw, format))?.facets;

/** Unowned seeding is retained for standalone projections/tests until reset. */
export function seedFacets(raw: string, format: Format, facets: Facets): void {
  const key = facetKey(raw, format);
  const entry = seeded.get(key);
  if (entry) entry.facets = facets;
  else seeded.set(key, { facets, owners: 1 });
}

/** Acquire during hydration; observe the published node after the store write.
 * Each owner watches just its node slot. Deletion, replacement and edited-away
 * raw release it without scanning the loaded document on a keystroke. */
export function acquireSeededFacets(raw: string, format: Format, facets: Facets, owner: () => { raw: string } | undefined): void {
  const key = facetKey(raw, format);
  const entry = seeded.get(key) ?? { facets, owners: 0 };
  entry.facets = facets;
  entry.owners++;
  seeded.set(key, entry);
  pending.push(() => createRoot(dispose => {
    const node = untrack(owner);
    const release = () => {
      owners.delete(release);
      if (--entry.owners === 0 && seeded.get(key) === entry) seeded.delete(key);
      dispose();
    };
    owners.add(release);
    createComputed(() => {
      const current = owner();
      if (!current || current !== node || current.raw !== raw) release();
    });
  }));
}

export function flushFacetOwners(): void {
  const next = pending;
  pending = [];
  for (const attach of next) attach();
}

export function clearSeededFacets(): void {
  pending = [];
  for (const release of owners) release();
  seeded.clear();
}
