import { visibleNeighbor, type OutlineScope } from "./tree";
import { doc } from "./model";
import { bodyScopeOf } from "./commentThreads";

/** Main-body order omits whole threads on every surface. A message body's
 * merge/navigation order stays inside that message's visible outline. */
export function bodyNeighbor(id: string, direction: -1 | 1, scope: OutlineScope | null = null): string | null {
  const owner = bodyScopeOf(id);
  if (owner) scope = { ...scope, roots: doc.byId[owner].children };
  return owner === undefined ? null : visibleNeighbor(id, direction, scope, at => bodyScopeOf(at) === owner);
}
