import type { Cardinality, ObservedType, RegistryRow } from "./queryIr";

/** Match a key to the registry's normalized name. The exact form wins; a
 * relaxed spelling is for display only and never becomes a write key. O(keys). */
export function registryRowFor(rows: RegistryRow[] | undefined, key: string): RegistryRow | undefined {
  if (!rows?.length) return undefined;
  const relaxed = key.trim().toLowerCase().replace(/[ _]/g, "-");
  return rows.find((row) => row.normalized_name === key) ?? rows.find((row) => row.normalized_name === relaxed);
}

/** The declaration overrides the observed majority for comparisons. O(1). */
export function effectiveTypeOf(row: RegistryRow): { type: ObservedType; cardinality: Cardinality } {
  return row.declared
    ? { type: row.declared[0], cardinality: row.declared[1] }
    : { type: row.observed_type, cardinality: row.cardinality };
}
