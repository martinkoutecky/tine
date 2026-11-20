import { pageIdentityKey } from "./pageIdentity";
import type { RefGroup } from "./types";

/** Key a source group by file, falling back to logical identity for pathless rows. */
export function referenceGroupKey(group: RefGroup): string {
  return group.path ? `file\0${group.path}` : pageIdentityKey(group.page);
}

/** Merge repeated groups from the same source. O(blocks). */
export function mergeReferenceGroups(groups: RefGroup[]): RefGroup[] {
  const merged = new Map<string, RefGroup>();
  for (const group of groups) {
    const key = referenceGroupKey(group);
    const existing = merged.get(key);
    if (existing) {
      existing.blocks.push(...group.blocks);
      existing.evidence = [...(existing.evidence ?? []), ...(group.evidence ?? [])];
    } else {
      merged.set(key, { ...group, blocks: [...group.blocks], evidence: [...(group.evidence ?? [])] });
    }
  }
  return [...merged.values()];
}
