import { visibleNeighbor, type OutlineScope } from "./tree";
import { threadRootOf } from "./commentThreads";

/** Body outline order omits whole annotation threads, independent of whether
 * they are drawn inline, in a margin, hidden, or on a secondary surface. */
export function bodyNeighbor(id: string, direction: -1 | 1, scope: OutlineScope | null = null): string | null {
  return visibleNeighbor(id, direction, scope, at => threadRootOf(at) === null);
}
