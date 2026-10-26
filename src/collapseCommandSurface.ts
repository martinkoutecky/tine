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
