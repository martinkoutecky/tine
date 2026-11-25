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
}
const targets = new Set<Target>();
clearOnBindingInvalidated(() => targets.clear());

export function registerCollapseCommandSurface(target: Target): () => void {
  targets.add(target);
  return () => { targets.delete(target); };
}

/** The mounted row a fold command on `id` acts through. */
export function commandRowSurface(id: string): CommandCollapseSurface | undefined {
  for (const target of targets) if (target.id === id && target.active()) return target.surface;
  return undefined;
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

/** Rows a mounted group shows for a host block, which the fold commands walk through the group's
 * own surface. A query block's body is its RESULT rows (they are not outline children of the host,
 * GH #792). A whole-block embed host shows one row in its own place, the source root, whose fold is
 * the host's (GH #1032). */
export interface HostedRows {
  hostId: () => string | undefined;
  /** The group's live roots, in display order; for a whole-block embed, its one root row. */
  roots: () => string[];
  surface: CommandCollapseSurface;
  /** The rows stand in the host's place (a whole-block embed) instead of being its body (query results). */
  inPlace: boolean;
}
const hostedRows = new Set<HostedRows>();
clearOnBindingInvalidated(() => hostedRows.clear());

export function registerHostedRows(group: HostedRows): () => void {
  hostedRows.add(group);
  return () => { hostedRows.delete(group); };
}

/** The mounted groups by host block. O(registered groups), once per command. */
export function hostedRowsByHost(): Map<string, HostedRows[]> {
  const byHost = new Map<string, HostedRows[]>();
  for (const group of hostedRows) {
    const host = group.hostId();
    if (host) byHost.set(host, [...byHost.get(host) ?? [], group]);
  }
  return byHost;
}
