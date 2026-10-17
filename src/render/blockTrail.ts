import { node as docNode } from "../document";

/** One owner for breadcrumb ancestry (OG `breadcrumb`, level-limit 3): the
 * nearest `limit` ancestors of a live block, outermost first, and whether more
 * exist above them. Walks at most limit + 1 parents, so it is O(1) per call even
 * on deep or malformed outlines. Null on a missing ancestor or a cycle: a trail
 * is never invented. Used by Linked References, block embeds and the right
 * sidebar's block cards. */
export function nearestAncestors(id: string, limit = 3): { ids: string[]; more: boolean } | null {
  const start = docNode(id);
  if (!start) return null;
  const nearest: string[] = [];
  const seen = new Set([id]);
  let parent = start.parent;
  while (parent !== null && nearest.length <= limit) {
    if (seen.has(parent)) return null;
    const ancestor = docNode(parent);
    if (!ancestor) return null;
    seen.add(parent);
    nearest.push(parent);
    parent = ancestor.parent;
  }
  return { ids: nearest.slice(0, limit).reverse(), more: nearest.length > limit };
}
