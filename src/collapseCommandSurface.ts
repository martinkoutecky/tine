import { onCleanup } from "solid-js";
import { clearOnBindingInvalidated } from "./binding";
import { hasSelection, selectionScope } from "./document";

/** Mounted secondary block occurrences expose the same fold contract to global
 * one-level commands as to their bullet arrows. Registrations die with the row. */
export interface CommandCollapseSurface {
  collapsed: (id: string, stored: boolean) => boolean;
  toggle: (id: string, current: boolean) => void;
}
interface Target {
  id: string;
  active: () => boolean;
  surface: CommandCollapseSurface;
  /** The host this row stands for (a whole-block embed's root, D-20261030-EMB). */
  host?: () => { id: string; scope: unknown } | null;
}
const targets = new Set<Target>();
clearOnBindingInvalidated(() => targets.clear());

export function registerCollapseCommandSurface(target: Target): () => void {
  targets.add(target);
  return () => { targets.delete(target); };
}

/** The mounted row a fold command on `id` acts through. A selected whole-block
 * embed host folds through its one visible row, the embed root. */
export function collapseCommandRow(id: string): { id: string; surface: CommandCollapseSurface } | undefined {
  for (const target of targets) {
    if (target.id === id && target.active()) return target;
    const host = target.host?.();
    if (host?.id === id && hasSelection() && selectionScope() === host.scope) return target;
  }
  return undefined;
}

/** A mounted row is the command target while a selection lives in its outline
 * scope, otherwise while it is being edited. */
export function bindCollapseCommandSurface(id: string, surface: CommandCollapseSurface | null | undefined, scope: unknown, editing: () => boolean, host?: Target["host"]): void {
  if (surface) onCleanup(registerCollapseCommandSurface({
    id,
    active: () => hasSelection() ? !!scope && selectionScope() === scope : editing(),
    surface,
    host,
  }));
}
