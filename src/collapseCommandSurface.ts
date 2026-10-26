import { clearOnBindingInvalidated } from "./binding";

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
