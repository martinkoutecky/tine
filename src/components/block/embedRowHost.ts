import type { OutlineScope } from "../../document";

/** D-20261030-EMB: where a block sits is per page; what it says is shared. A
 *  whole-block `{{embed ((uuid))}}` shows one bullet: the source root's row, at
 *  the host's position (0.8). That row's position belongs to the host on this
 *  page, so drag, indent/outdent, selection (and through it Delete) and the
 *  bullet's menu act on the host; its text and children stay the source's. */
export interface RowHost { id: string; scope: OutlineScope | null }

// Pointer hit tests (drop target, selection drags) find rows by DOM element.
const rows = new WeakMap<Element, () => RowHost | null>();
export function bindRowHost(row: Element, host: () => RowHost | null): void {
  rows.set(row, host);
}
/** The host a rendered `.ls-block` row stands for, or null for an ordinary row. */
export function rowHostOf(row: Element | null | undefined): RowHost | null {
  return (row && rows.get(row)?.()) ?? null;
}
