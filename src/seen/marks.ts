// Mapping a backend `SeenPageChanges` reply onto the rendered page (ADR 0079).
// Pure: the backend addresses blocks by zero-based structural paths into the
// snapshot the page sent; the snapshot carries the runtime ids.
import type { SeenBlockPath, SeenPageChanges } from "../backendTypes";

export interface SnapshotNode {
  id: string;
  raw: string;
  children: SnapshotNode[];
}

/** The page-roots list has no block id; its removed markers hang off this key. */
export const SEEN_PAGE_ROOT = "";

export interface RemovedMark {
  text: string;
}

export interface SeenMarks {
  /** Changed block id -> its raw text when the snapshot was taken. A block whose
   *  raw differs now was edited locally since and loses its mark. */
  blocks: Record<string, string>;
  /** Removed bodies drawn after this block id (the previous sibling). */
  after: Record<string, RemovedMark[]>;
  /** Removed bodies drawn first inside this parent id (SEEN_PAGE_ROOT = page). */
  first: Record<string, RemovedMark[]>;
  removedCount: number;
  identityKnown: boolean;
  unattributed: boolean;
}

export const emptyMarks = (): SeenMarks => ({
  blocks: {}, after: {}, first: {}, removedCount: 0, identityKnown: true, unattributed: false,
});

/** The node at `path` ([rootIndex, childIndex, ...]), or null when it does not exist. */
export function nodeAtPath(roots: readonly SnapshotNode[], path: SeenBlockPath): SnapshotNode | null {
  let level: readonly SnapshotNode[] = roots;
  let found: SnapshotNode | null = null;
  for (const index of path) {
    found = level[index] ?? null;
    if (!found) return null;
    level = found.children;
  }
  return found;
}

export function marksFromChanges(reply: SeenPageChanges, roots: readonly SnapshotNode[]): SeenMarks {
  if (!reply.available) return emptyMarks();
  const marks: SeenMarks = {
    blocks: {}, after: {}, first: {}, removedCount: 0,
    identityKnown: reply.identity_known, unattributed: reply.unattributed,
  };
  for (const block of reply.blocks) {
    const node = nodeAtPath(roots, block.block_path);
    if (node) marks.blocks[node.id] = node.raw;
  }
  for (const removed of reply.removed) {
    // A removal is never dropped for a path that no longer resolves: it falls
    // back to the start of its parent's list, then of the page.
    const previous = removed.previous_sibling_path ? nodeAtPath(roots, removed.previous_sibling_path) : null;
    const parent = removed.parent_path ? nodeAtPath(roots, removed.parent_path) : null;
    const [bucket, key] = previous ? [marks.after, previous.id] : [marks.first, parent?.id ?? SEEN_PAGE_ROOT];
    (bucket[key] ??= []).push({ text: removed.text });
    marks.removedCount++;
  }
  return marks;
}

/** How many changes the header counts: still-unedited marked blocks plus removals. */
export function changeCount(marks: SeenMarks, rawOf: (id: string) => string | undefined): number {
  let count = marks.removedCount;
  for (const [id, raw] of Object.entries(marks.blocks)) if (rawOf(id) === raw) count++;
  return count;
}
