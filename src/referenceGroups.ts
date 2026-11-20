import { pageIdentityKey } from "./pageIdentity";
import type { RefGroup } from "./types";

/** Merge reference groups that answer to one page identity (case, NFC/NFD and
 * boundary-slash spellings fold together, the same key the backend dedups with).
 * The one merger for the Linked and Unlinked References panels. O(blocks). */
export function mergeReferenceGroups(groups: RefGroup[]): RefGroup[] {
  const merged = new Map<string, RefGroup>();
  for (const group of groups) {
    const key = pageIdentityKey(group.page);
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
