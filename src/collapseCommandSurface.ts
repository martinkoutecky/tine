import { onCleanup } from "solid-js";
import { clearOnBindingInvalidated } from "./binding";
import { hasSelection, selectionScope } from "./document";

/** Mounted secondary block occurrences expose the same fold contract to global
 * one-level commands as to their bullet arrows. Registrations die with the row. */
export interface CommandCollapseSurface {
  collapsed: (id: string, stored: boolean) => boolean;
  toggle: (id: string, current: boolean) => void;
}
const targets = new Set<{ id: string; active: () => boolean; surface: CommandCollapseSurface }>();
clearOnBindingInvalidated(() => targets.clear());

export function registerCollapseCommandSurface(target: { id: string; active: () => boolean; surface: CommandCollapseSurface }): () => void {
  targets.add(target);
  return () => { targets.delete(target); };
}

export function collapseCommandSurface(id: string): CommandCollapseSurface | undefined {
  return [...targets].find(target => target.id === id && target.active())?.surface;
}

/** A mounted row is the command target while a selection lives in its outline
 * scope, otherwise while it is being edited. */
export function bindCollapseCommandSurface(id: string, surface: CommandCollapseSurface | null | undefined, scope: unknown, editing: () => boolean): void {
  if (surface) onCleanup(registerCollapseCommandSurface({
    id,
    active: () => hasSelection() ? !!scope && selectionScope() === scope : editing(),
    surface,
  }));
}

/** A mounted query-result group, registered under the query block that owns it. A query block's
 * body is its RESULT rows (they are not outline children of the host), so the one-level fold
 * commands read them here and fold them through the group's own local surface (GH #792). */
export interface QueryResultFold {
  hostId: () => string | undefined;
  /** The group's live result roots, in display order. */
  roots: () => string[];
  surface: CommandCollapseSurface;
}
const resultFolds = new Set<QueryResultFold>();
clearOnBindingInvalidated(() => resultFolds.clear());

export function registerQueryResultFold(group: QueryResultFold): () => void {
  resultFolds.add(group);
  return () => { resultFolds.delete(group); };
}

/** The mounted result groups of query block `hostId`. O(registered groups), once per command. */
export function queryResultFolds(hostId: string): QueryResultFold[] {
  return [...resultFolds].filter((group) => group.hostId() === hostId);
}
